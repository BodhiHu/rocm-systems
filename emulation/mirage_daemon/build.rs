//! Build script: ensures the embedded dashboard bundle exists.
//!
//! The daemon embeds `../dashboard/dist/**` into the binary via `rust-embed`.
//! If the `dist/` directory is missing, we try to invoke `npm run build` in
//! the dashboard directory. If `npm` is unavailable or the build fails, we
//! fall back to materialising a minimal placeholder so the crate still
//! compiles (useful for CI contexts that don't have Node.js).
//!
//! Set `MIRAGE_SKIP_DASHBOARD_BUILD=1` to never invoke npm.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dashboard_dir = manifest_dir.join("../dashboard").canonicalize().unwrap();
    let dist_dir = dashboard_dir.join("dist");

    // Rebuild if the dashboard sources or the dist change.
    println!("cargo:rerun-if-changed={}", dashboard_dir.join("src").display());
    println!("cargo:rerun-if-changed={}", dashboard_dir.join("index.html").display());
    println!("cargo:rerun-if-changed={}", dashboard_dir.join("package.json").display());
    println!("cargo:rerun-if-changed={}", dashboard_dir.join("vite.config.ts").display());
    println!("cargo:rerun-if-changed={}", dist_dir.join("index.html").display());
    println!("cargo:rerun-if-env-changed=MIRAGE_SKIP_DASHBOARD_BUILD");

    let dist_has_index = dist_dir.join("index.html").is_file();
    let skip = std::env::var_os("MIRAGE_SKIP_DASHBOARD_BUILD").is_some();

    if !dist_has_index && !skip {
        if let Err(e) = build_dashboard(&dashboard_dir) {
            println!("cargo:warning=dashboard build failed: {e}; falling back to placeholder");
            write_placeholder(&dist_dir);
        }
    } else if !dist_has_index {
        write_placeholder(&dist_dir);
    }
}

fn build_dashboard(dir: &Path) -> Result<(), String> {
    if !dir.join("node_modules").is_dir() {
        run(dir, "npm", &["ci", "--silent"])?;
    }
    run(dir, "npm", &["run", "build", "--silent"])
}

fn run(dir: &Path, bin: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(bin)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("failed to spawn `{bin}`: {e}"))?;
    if !status.success() {
        return Err(format!("`{bin} {}` exited with {status}", args.join(" ")));
    }
    Ok(())
}

fn write_placeholder(dist_dir: &Path) {
    std::fs::create_dir_all(dist_dir).ok();
    let html = "<!doctype html><html><head><meta charset=\"utf-8\"><title>mirage</title></head>\
        <body><h1>Mirage daemon</h1>\
        <p>Dashboard assets were not built. Run <code>npm run build</code> in \
        <code>emulation/dashboard/</code> and rebuild the daemon.</p></body></html>";
    std::fs::write(dist_dir.join("index.html"), html).ok();
}
