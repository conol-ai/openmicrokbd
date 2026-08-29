use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(openmicro_sparkle)");
    println!("cargo:rerun-if-changed=macos/sparkle_bridge.m");
    println!("cargo:rerun-if-env-changed=OPENMICRO_SPARKLE_FRAMEWORK_DIR");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=windows/OpenMicro.rc");
        println!("cargo:rerun-if-changed=windows/OpenMicro.exe.manifest");
        println!("cargo:rerun-if-changed=../site/public/favicon.ico");
        embed_resource::compile("windows/OpenMicro.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to compile Windows application resources");
        return;
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let Some(framework_dir) = env::var_os("OPENMICRO_SPARKLE_FRAMEWORK_DIR").map(PathBuf::from)
    else {
        // Normal source builds deliberately use the no-op Rust implementation.
        // Release packaging supplies the pinned framework directory.
        return;
    };
    validate_framework(&framework_dir);

    let framework_flag = format!("-F{}", framework_dir.display());
    cc::Build::new()
        .file("macos/sparkle_bridge.m")
        .flag("-fobjc-arc")
        .flag("-mmacosx-version-min=11.0")
        .flag(&framework_flag)
        .compile("openmicro_sparkle_bridge");

    println!("cargo:rustc-cfg=openmicro_sparkle");
    println!(
        "cargo:rustc-link-search=framework={}",
        framework_dir.display()
    );
    println!("cargo:rustc-link-lib=framework=Sparkle");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
}

fn validate_framework(parent: &Path) {
    let framework = parent.join("Sparkle.framework");
    let header = framework.join("Headers/Sparkle.h");
    let binary = framework.join("Versions/B/Sparkle");
    if !header.is_file() || !binary.is_file() {
        panic!(
            "OPENMICRO_SPARKLE_FRAMEWORK_DIR must contain a complete Sparkle.framework: {}",
            parent.display()
        );
    }
}
