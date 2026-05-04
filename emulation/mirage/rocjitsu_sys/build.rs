//! Build script for `rocjitsu_sys`.
//!
//! # What this does
//!
//! 1. Locates the rocjitsu C headers and generates Rust FFI bindings
//!    for every `rj_*` declaration using [`bindgen`].
//! 2. Links against a real `librocjitsu_kmd.so` — either a prebuilt
//!    one pointed at by `ROCJITSU_LIB_DIR`, or a freshly built tree
//!    produced by driving rocjitsu's own CMake build from
//!    `experimental/rocjitsu` in the monorepo. All `rj_*` public
//!    symbols are exported from `librocjitsu_kmd.so` via
//!    `__attribute__((visibility("default")))`, so linking against
//!    that shared library is sufficient.
//! 3. If neither a prebuilt library nor source checkout is available,
//!    falls back to a C stub so downstream crates at least *link* on
//!    hosts without rocjitsu. Calls into stubbed symbols return
//!    `ROCJITSU_STATUS_ERROR` at runtime rather than corrupting
//!    memory.
//!
//! # Environment overrides
//!
//! * `ROCJITSU_INCLUDE_DIR` — directory containing `rocjitsu/rocjitsu.h`.
//!   Auto-detected from the in-tree checkout when unset.
//! * `ROCJITSU_LIB_DIR` — directory containing a prebuilt
//!   `librocjitsu_kmd.so`. When unset, the script attempts to build
//!   rocjitsu from source.
//! * `ROCJITSU_SOURCE_DIR` — path to the rocjitsu source tree.
//!   Auto-detected as `<monorepo>/experimental/rocjitsu` when unset.
//! * `ROCJITSU_NO_BUILD=1` — skip the source build and fall back to
//!   the stub, even if sources are available. Useful for fast
//!   `cargo check` iterations.

use std::env;
use std::path::{Path, PathBuf};

const ROCJITSU_LIB_NAME: &str = "rocjitsu_kmd";
const STUB_LIB_NAME: &str = "rocjitsu_stub";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ROCJITSU_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=ROCJITSU_LIB_DIR");
    println!("cargo:rerun-if-env-changed=ROCJITSU_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=ROCJITSU_NO_BUILD");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let bindings_out = out_dir.join("bindings.rs");

    // --- headers + bindings -----------------------------------------------
    let include_dir = resolve_include_dir();
    let Some(include_dir) = include_dir else {
        std::fs::write(
            &bindings_out,
            "// rocjitsu headers not found at build time.\n",
        )
        .unwrap();
        build_stub();
        return;
    };

    let header = include_dir.join("rocjitsu/rocjitsu.h");
    println!("cargo:rerun-if-changed={}", header.display());

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.display()))
        .allowlist_type("rj_.*")
        .allowlist_function("rj_.*")
        .allowlist_var("RJ_.*|ROCJITSU_.*")
        .derive_default(true)
        .derive_debug(true)
        .derive_copy(true)
        .layout_tests(false)
        .generate_comments(false)
        .generate()
        .unwrap_or_else(|e| panic!("bindgen {}: {}", header.display(), e));
    bindings
        .write_to_file(&bindings_out)
        .unwrap_or_else(|e| panic!("write {}: {}", bindings_out.display(), e));

    // --- linking ----------------------------------------------------------
    if let Ok(lib_dir) = env::var("ROCJITSU_LIB_DIR") {
        link_prebuilt(Path::new(&lib_dir));
        return;
    }

    if env::var("ROCJITSU_NO_BUILD").as_deref() == Ok("1") {
        eprintln!("cargo:warning=ROCJITSU_NO_BUILD=1 — linking stub implementation");
        build_stub();
        return;
    }

    if let Some(source_dir) = resolve_source_dir() {
        let lib_dir = build_rocjitsu(&source_dir);
        link_prebuilt(&lib_dir);
    } else {
        eprintln!("cargo:warning=rocjitsu source not found — linking stub implementation");
        build_stub();
    }
}

fn resolve_include_dir() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("ROCJITSU_INCLUDE_DIR") {
        let p = PathBuf::from(explicit);
        if p.join("rocjitsu/rocjitsu.h").exists() {
            return Some(p);
        }
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    for ancestor in manifest.ancestors() {
        let candidate = ancestor.join("experimental/rocjitsu/lib/rocjitsu/include");
        if candidate.join("rocjitsu/rocjitsu.h").exists() {
            return Some(candidate);
        }
    }
    for sys in ["/usr/include", "/usr/local/include", "/opt/rocm/include"] {
        let p = Path::new(sys);
        if p.join("rocjitsu/rocjitsu.h").exists() {
            return Some(p.to_path_buf());
        }
    }
    None
}

fn resolve_source_dir() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("ROCJITSU_SOURCE_DIR") {
        let p = PathBuf::from(explicit);
        if p.join("CMakeLists.txt").exists() {
            return Some(p);
        }
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    for ancestor in manifest.ancestors() {
        let candidate = ancestor.join("experimental/rocjitsu");
        if candidate.join("CMakeLists.txt").exists() {
            return Some(candidate);
        }
    }
    None
}

/// Drive the upstream rocjitsu CMake build.
///
/// Produces a `librocjitsu_kmd.so` whose exported `rj_*` symbols (via
/// `RJ_API_EXPORT` default visibility) cover the public C API. Returns
/// the directory containing the shared library.
fn build_rocjitsu(source_dir: &Path) -> PathBuf {
    println!(
        "cargo:rerun-if-changed={}",
        source_dir.join("CMakeLists.txt").display()
    );

    let fetch_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("rocjitsu_fetch");

    let mut config = cmake::Config::new(source_dir);
    // rocjitsu requires C++20 including `<format>`, which is only
    // available in libstdc++ from GCC 13+ and libc++ from Clang 16+.
    // Prefer an explicit toolchain if the caller set one; otherwise
    // pick a known-good in-tree compiler when the default (e.g. gcc-11
    // on Ubuntu 22.04) is too old.
    if env::var_os("CXX").is_none()
        && let Some(cxx) = pick_cxx_compiler()
    {
        config.define("CMAKE_CXX_COMPILER", &cxx);
    }

    let dst = config
        .define("RJ_BUILD_GUI", "OFF")
        .define("BUILD_TESTING", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define(
            "FETCHCONTENT_BASE_DIR",
            fetch_dir.to_string_lossy().as_ref(),
        )
        .profile("Release")
        .build_target("rocjitsu_kmd_shim")
        .very_verbose(false)
        .build();

    // `cmake::Config::build` returns the install prefix; rocjitsu has no
    // install rules so the artefacts only exist under `build/`.
    let build_dir = dst.join("build");
    let lib_dir = build_dir.join("lib/rocjitsu/src/rocjitsu/kmd");
    let so = lib_dir.join(format!("lib{ROCJITSU_LIB_NAME}.so"));
    assert!(
        so.exists(),
        "expected {} after building rocjitsu; got missing artefact",
        so.display()
    );
    lib_dir
}

fn link_prebuilt(lib_dir: &Path) {
    let lib_dir = lib_dir
        .canonicalize()
        .unwrap_or_else(|_| lib_dir.to_path_buf());
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib={ROCJITSU_LIB_NAME}");
    // Export the library directory via `links = "rocjitsu_kmd"`
    // metadata. Direct dependents receive this as
    // `DEP_ROCJITSU_KMD_LIB_DIR` and are expected to emit an rpath for
    // their own binaries / tests (see `mirage_rocjitsu/build.rs`).
    println!("cargo:lib_dir={}", lib_dir.display());
}

fn build_stub() {
    let stub = Path::new("stub/rocjitsu_stub.c");
    println!("cargo:rerun-if-changed={}", stub.display());
    cc::Build::new()
        .file(stub)
        .warnings(false)
        .compile(STUB_LIB_NAME);
}

/// Pick a C++ compiler new enough to provide `<format>` (C++20).
///
/// Checks a short list of common binaries and returns the first one
/// found. Returns `None` if nothing suitable is on `PATH`, in which
/// case CMake's default will be used and the build will fail with a
/// clear error for the user to fix.
fn pick_cxx_compiler() -> Option<PathBuf> {
    for candidate in ["g++-14", "g++-13", "clang++-18", "clang++-17", "clang++-16"] {
        if let Ok(path) = which(candidate) {
            return Some(path);
        }
    }
    None
}

fn which(name: &str) -> Result<PathBuf, ()> {
    let paths = env::var_os("PATH").ok_or(())?;
    for dir in env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(())
}
