//! Build steps:
//! - embed the exe icon on Windows (assets/app.rc → assets/icon.ico)
//! - copy the bundled libmpv next to the built executable, so `cargo run`
//!   works without any manual setup

use std::path::PathBuf;

fn main() {
    embed_icon();
    copy_libmpv();
}

fn embed_icon() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if let Err(e) = embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional() {
            println!("cargo:warning=couldn't embed the exe icon: {e}");
        }
    }
}

fn copy_libmpv() {
    let (dir, file) = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => ("windows", "libmpv-2.dll"),
        Ok("macos") => ("macos", "libmpv.2.dylib"),
        _ => return,
    };

    let src = PathBuf::from("third_party/mpv").join(dir).join(file);
    println!("cargo:rerun-if-changed={}", src.display());
    if !src.exists() {
        println!(
            "cargo:warning=libmpv not found at {} (the player will look for it on the system path)",
            src.display()
        );
        return;
    }

    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out → walk up to target/<profile>
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let Some(profile_dir) = out.ancestors().nth(3) else {
        return;
    };
    let dst = profile_dir.join(file);

    let needs_copy = match (std::fs::metadata(&src), std::fs::metadata(&dst)) {
        (Ok(s), Ok(d)) => s.len() != d.len(),
        _ => true,
    };
    if needs_copy {
        if let Err(e) = std::fs::copy(&src, &dst) {
            println!("cargo:warning=failed to copy libmpv: {e}");
        }
    }
}
