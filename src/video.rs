//! Offscreen OpenGL target that mpv renders into, shown by egui as a texture.

use eframe::glow::{self, HasContext};

use crate::mpv::RenderContext;

pub struct VideoTarget {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    pub texture_id: eframe::egui::TextureId,
    size: [i32; 2],
}

impl VideoTarget {
    pub fn new(gl: &glow::Context, frame: &mut eframe::Frame) -> Result<Self, String> {
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
            let texture_id = frame.register_native_glow_texture(texture);
            Ok(Self { fbo, texture, texture_id, size: [0, 0] })
        }
    }

    /// Renders a new frame if mpv has one, or if the target was resized.
    /// `size` is in physical pixels.
    pub fn update(&mut self, gl: &glow::Context, render: &RenderContext, size: [i32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        let resized = size != self.size;
        if resized {
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
        }

        if render.frame_ready() || resized {
            let id = self.fbo.0.get() as i32;
            // No flip: mpv's FBO output already matches egui's top-down UVs.
            render.render(id, size[0], size[1], false);
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe { gl.delete_framebuffer(self.fbo) };
    }
}
