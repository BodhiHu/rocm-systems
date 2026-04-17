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
    println!("cargo:rerun-if-changed=include/amdgpu_drm_compat.h");
    println!(
        "cargo:rerun-if-changed=../../experimental/rocjitsu/lib/rocjitsu/external_headers/hsa_headers/linux/uapi/kfd_ioctl.h"
    );
    println!("cargo:rerun-if-changed=../../projects/amdsmi/include/libdrm/amdgpu_drm.h");
    println!(
        "cargo:rerun-if-changed=../../projects/rocr-runtime/libhsakmt/include/hsakmt/drm/amdgpu_drm.h"
    );
    println!("cargo:rerun-if-env-changed=MIRAGE_UAPI_KFD_HEADER");
    println!("cargo:rerun-if-env-changed=MIRAGE_UAPI_DRM_HEADER");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("mirage_uapi lives under emulation/");

    let default_kfd_header = repo_root
        .join("experimental/rocjitsu/lib/rocjitsu/external_headers/hsa_headers/linux/uapi/kfd_ioctl.h");
    let default_drm_header = manifest_dir.join("include/amdgpu_drm_compat.h");

    let kfd_header = env::var("MIRAGE_UAPI_KFD_HEADER")
        .map(PathBuf::from)
        .unwrap_or(default_kfd_header);
    let drm_header = env::var("MIRAGE_UAPI_DRM_HEADER")
        .map(PathBuf::from)
        .unwrap_or(default_drm_header);

    generate(
        &kfd_header,
        &out_dir.join("kfd.rs"),
        "kfd_ioctl_.*|AMDKFD_.*",
        &[format!("-I{}", repo_root.display())],
    );
    generate(
        &drm_header,
        &out_dir.join("drm.rs"),
        "drm_amdgpu_.*|DRM_AMDGPU_.*|DRM_IOCTL_AMDGPU_.*|DRM_COMMAND_BASE",
        &[
            format!("-I{}", repo_root.join("projects/amdsmi/include").display()),
            format!(
                "-I{}",
                repo_root
                    .join("projects/rocr-runtime/libhsakmt/include/hsakmt")
                    .display()
            ),
        ],
    );
}

fn generate(header: &Path, out: &Path, allowlist: &str, clang_args: &[String]) {
    if !header.exists() {
        // Gracefully emit an empty bindings module so downstream crates
        // still compile on systems without the UAPI headers.
        std::fs::write(out, "// UAPI header not found at build time.\n").unwrap();
        return;
    }
    let mut bindings = bindgen::Builder::default().header(header.display().to_string());
    for arg in clang_args {
        bindings = bindings.clang_arg(arg);
    }
    let bindings = bindings
        .allowlist_type(allowlist)
        .allowlist_var(allowlist)
        .derive_default(true)
        .derive_debug(true)
        .derive_copy(true)
        .layout_tests(false)
        .generate_comments(false)
        .generate()
        .unwrap_or_else(|e| panic!("bindgen {}: {e}", header.display()));
    bindings
        .write_to_file(out)
        .unwrap_or_else(|e| panic!("write {}: {}", out.display(), e));
}
