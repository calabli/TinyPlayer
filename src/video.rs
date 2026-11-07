//! Offscreen OpenGL targets that mpv renders into, shown by egui as textures.
//! Panoramic video takes one more pass, from the whole frame to the view.

use eframe::egui::TextureId;
use eframe::glow::{self, HasContext};

use crate::mpv::RenderContext;
use crate::vr::Projection;

const VERTEX: &str = r"#version 150
out vec2 ndc;
void main() {
    // One triangle that covers the whole target.
    ndc = vec2(float((gl_VertexID & 1) << 2) - 1.0, float((gl_VertexID & 2) << 1) - 1.0);
    gl_Position = vec4(ndc, 0.0, 1.0);
}
";

// Longitude 0 is the middle of the picture. `eye` is [left, top, width,
// height]; texture rows run downward, as in `VideoTarget::update`.
const FRAGMENT: &str = r"#version 150
const float PI = 3.14159265358979;
uniform sampler2D frame;
uniform mat3 rotation;
uniform vec2 tan_half;
uniform vec4 eye;
uniform float span;
in vec2 ndc;
out vec4 color;
void main() {
    vec3 ray = normalize(rotation * vec3(ndc.x * tan_half.x, -ndc.y * tan_half.y, 1.0));
    float u = atan(ray.x, ray.z) / (2.0 * PI * span) + 0.5;
    float v = 0.5 - asin(clamp(ray.y, -1.0, 1.0)) / PI;
    if (u < 0.0 || u > 1.0) {
        color = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }
    // Stays half a texel inside, so the other eye can't bleed in at the seam.
    vec2 inset = 0.5 / vec2(textureSize(frame, 0));
    vec2 uv = clamp(eye.xy + vec2(u, v) * eye.zw, eye.xy + inset, eye.xy + eye.zw - inset);
    color = vec4(texture(frame, uv).rgb, 1.0);
}
";

struct Layer {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    id: TextureId,
    size: [i32; 2],
}

impl Layer {
    fn new(gl: &glow::Context, frame: &mut eframe::Frame) -> Result<Self, String> {
        unsafe {
            let texture = gl.create_texture()?;
            let fbo = gl.create_framebuffer()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (p, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            // egui takes ownership of the texture and frees it on exit.
            let id = frame.register_native_glow_texture(texture);
            Ok(Self { fbo, texture, id, size: [0, 0] })
        }
    }

    /// True if the size changed.
    fn resize(&mut self, gl: &glow::Context, size: [i32; 2]) -> bool {
        if size == self.size {
            return false;
        }
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(self.texture),
                0,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        self.size = size;
        true
    }

    fn destroy(&self, gl: &glow::Context) {
        unsafe { gl.delete_framebuffer(self.fbo) };
    }
}

struct Projector {
    program: glow::Program,
    // Core OpenGL can't draw without one, even with no vertex data.
    vertices: glow::VertexArray,
}

impl Projector {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, source) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)] {
                let shader = gl.create_shader(kind)?;
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    let log = gl.get_shader_info_log(shader);
                    gl.delete_shader(shader);
                    return Err(log);
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            gl.link_program(program);
            let linked = gl.get_program_link_status(program);
            let log = gl.get_program_info_log(program);
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }
            if !linked {
                gl.delete_program(program);
                return Err(log);
            }
            let vertices = gl.create_vertex_array()?;
            Ok(Self { program, vertices })
        }
    }

    /// egui sets up its own OpenGL state before it paints, so this leaves
    /// only the bindings clean.
    fn draw(&self, gl: &glow::Context, source: glow::Texture, target: &Layer, view: &Projection) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.fbo));
            gl.viewport(0, 0, target.size[0], target.size[1]);
            for cap in [glow::SCISSOR_TEST, glow::BLEND, glow::DEPTH_TEST, glow::STENCIL_TEST, glow::CULL_FACE] {
                gl.disable(cap);
            }
            gl.color_mask(true, true, true, true);
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vertices));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(source));

            let at = |name: &str| gl.get_uniform_location(self.program, name);
            gl.uniform_1_i32(at("frame").as_ref(), 0);
            gl.uniform_matrix_3_f32_slice(at("rotation").as_ref(), false, &view.rotation);
            gl.uniform_2_f32_slice(at("tan_half").as_ref(), &view.tan_half);
            gl.uniform_4_f32_slice(at("eye").as_ref(), &view.eye);
            gl.uniform_1_f32(at("span").as_ref(), view.span);

            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }

    fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vertices);
        }
    }
}

/// A panoramic picture to show: the view, and the size to render the whole
/// frame at.
pub struct Panorama {
    pub view: Projection,
    pub size: [i32; 2],
}

pub struct VideoTarget {
    frame: Layer,
    view: Layer,
    /// Built when first needed, so a driver that can't compile it only loses
    /// panoramic video.
    projector: Option<Projector>,
    drawn: Option<Projection>,
}

impl VideoTarget {
    pub fn new(gl: &glow::Context, frame: &mut eframe::Frame) -> Result<Self, String> {
        let (video, view) = (Layer::new(gl, frame)?, Layer::new(gl, frame)?);
        Ok(Self { frame: video, view, projector: None, drawn: None })
    }

    /// Renders a new frame if mpv has one, or if the target was resized, and
    /// returns the texture to show. `window` is in physical pixels.
    ///
    /// mpv normally renders at window size. A panorama renders larger, as a
    /// view only shows a small part of it.
    pub fn update(
        &mut self,
        gl: &glow::Context,
        render: &RenderContext,
        window: [i32; 2],
        panorama: Option<Panorama>,
    ) -> Result<TextureId, String> {
        let window = window.map(|v| v.max(1));
        let size = match &panorama {
            Some(p) => {
                let limit = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) }.max(1);
                p.size.map(|v| v.clamp(1, limit))
            }
            None => window,
        };
        let resized = self.frame.resize(gl, size);
        let fresh = render.frame_ready() || resized;
        if fresh {
            let id = self.frame.fbo.0.get() as i32;
            // No flip: mpv's FBO output already matches egui's top-down UVs.
            render.render(id, size[0], size[1], false);
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
        }

        let Some(panorama) = panorama else {
            self.drawn = None;
            return Ok(self.frame.id);
        };
        let view_resized = self.view.resize(gl, window);
        if fresh || view_resized || self.drawn != Some(panorama.view) {
            if self.projector.is_none() {
                self.projector = Some(Projector::new(gl)?);
            }
            if let Some(projector) = &self.projector {
                projector.draw(gl, self.frame.texture, &self.view, &panorama.view);
            }
            self.drawn = Some(panorama.view);
        }
        Ok(self.view.id)
    }

    pub fn destroy(&self, gl: &glow::Context) {
        self.frame.destroy(gl);
        self.view.destroy(gl);
        if let Some(projector) = &self.projector {
            projector.destroy(gl);
        }
    }
}
