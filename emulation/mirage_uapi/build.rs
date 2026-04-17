//! Build script — parse the AMD KFD and DRM-AMDGPU UAPI headers with
//! bindgen and emit Rust bindings. Running bindgen at build time keeps
//! the `#[repr(C)]` structs authoritative: they stay in sync with
//! whatever kernel UAPI ships on the build host.
//!
//! The two resulting files (`kfd.rs`, `drm.rs`) are included verbatim
//! from [`crate::kfd`] and [`crate::drm`] in `src/lib.rs`.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=MIRAGE_UAPI_KFD_HEADER");
    println!("cargo:rerun-if-env-changed=MIRAGE_UAPI_DRM_HEADER");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let kfd_header = env::var("MIRAGE_UAPI_KFD_HEADER")
        .unwrap_or_else(|_| "/usr/include/linux/kfd_ioctl.h".to_string());
    let drm_header = env::var("MIRAGE_UAPI_DRM_HEADER")
        .unwrap_or_else(|_| "/usr/include/drm/amdgpu_drm.h".to_string());

    generate(
        &kfd_header,
        &out_dir.join("kfd.rs"),
        "kfd_ioctl_.*|AMDKFD_.*",
    );
    generate(
        &drm_header,
        &out_dir.join("drm.rs"),
        "drm_amdgpu_.*|DRM_AMDGPU_.*|DRM_IOCTL_AMDGPU_.*|DRM_COMMAND_BASE",
    );
}

fn generate(header: &str, out: &Path, allowlist: &str) {
    if !Path::new(header).exists() {
        // Gracefully emit an empty bindings module so downstream crates
        // still compile on systems without the UAPI headers.
        std::fs::write(out, "// UAPI header not found at build time.\n").unwrap();
        return;
    }
    let bindings = bindgen::Builder::default()
        .header(header)
        .allowlist_type(allowlist)
        .allowlist_var(allowlist)
        .derive_default(true)
        .derive_debug(true)
        .derive_copy(true)
        .layout_tests(false)
        .generate_comments(false)
        .generate()
        .unwrap_or_else(|e| panic!("bindgen {header}: {e}"));
    bindings
        .write_to_file(out)
        .unwrap_or_else(|e| panic!("write {}: {}", out.display(), e));
}
