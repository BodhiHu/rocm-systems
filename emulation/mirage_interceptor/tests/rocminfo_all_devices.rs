//! End-to-end integration test: compare `rocminfo` output against
//! `rocminfo` running under the mirage interceptor pointed at a
//! [`RealEmulator`]-backed daemon.
//!
//! The test skips cleanly when the host has no AMD hardware. When it
//! does, the two invocations must produce equivalent output.
//!
//! Prerequisites for the non-skipping path:
//!
//! * `/dev/kfd` exists and is readable.
//! * `rocminfo` is on `PATH`.
//! * The interceptor cdylib has been built (`cargo build -p mirage_interceptor`).
//!
//! If any of these is missing, the test prints an explanation and
//! returns success. This makes it safe to run in CI environments that
//! lack the hardware.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mirage_real::RealEmulator;
use mirage_remote::EmulatorServer;

fn which(bin: &str) -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn interceptor_cdylib() -> Option<PathBuf> {
    // Layout: target/<profile>/deps/... when running tests; the cdylib
    // lands in target/<profile>/libmirage_interceptor.so. We locate it
    // relative to the current executable.
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    // deps/ → profile dir
    if dir.file_name().and_then(|s| s.to_str()) == Some("deps") {
        dir.pop();
    }
    let candidate = dir.join("libmirage_interceptor.so");
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn unique_socket(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("mirage-rocminfo-{tag}-{nanos}.sock"))
}

/// Extract a normalized subset of rocminfo output so we can compare
/// real-hw vs. intercepted runs while ignoring transient fields
/// (queue IDs, addresses, counters).
fn normalize(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("Name:")
                || t.starts_with("Marketing Name:")
                || t.starts_with("Vendor Name:")
                || t.starts_with("Device Type:")
                || t.starts_with("Uuid:")
                || t.starts_with("Compute Unit:")
        })
        .map(|l| l.trim().to_string())
        .collect()
}

struct RocminfoRuns {
    baseline_out: String,
    baseline_norm: Vec<String>,
    intercepted_out: String,
    intercepted_err: String,
    intercepted_norm: Vec<String>,
}

fn collect_runs() -> Option<RocminfoRuns> {
    if !RealEmulator::hardware_available() {
        eprintln!("rocminfo test: no /dev/kfd on this host - skipping");
        return None;
    }
    let Some(rocminfo) = which("rocminfo") else {
        eprintln!("rocminfo test: `rocminfo` not on PATH - skipping");
        return None;
    };
    let Some(cdylib) = interceptor_cdylib() else {
        eprintln!(
            "rocminfo test: libmirage_interceptor.so not found; \
             run `cargo build -p mirage_interceptor` first - skipping"
        );
        return None;
    };

    let baseline = Command::new(&rocminfo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rocminfo baseline");
    if !baseline.status.success() {
        eprintln!(
            "rocminfo test: baseline rocminfo failed (status {:?}); skipping",
            baseline.status
        );
        return None;
    }
    let baseline_out = String::from_utf8_lossy(&baseline.stdout).to_string();
    let baseline_norm = normalize(&baseline_out);
    assert!(
        !baseline_norm.is_empty(),
        "baseline rocminfo produced no recognisable device lines:\n{baseline_out}"
    );

    let sock = unique_socket("all-devices");
    let real = RealEmulator::detect()
        .expect("detect real emu")
        .expect("real emu exists given hardware_available");
    let server = EmulatorServer::from_arc(sock.clone(), Arc::new(real));
    let listener = server.bind().expect("bind emu server");
    let server_thread = thread::spawn(move || {
        let _ = server.serve_on(listener);
    });
    thread::sleep(Duration::from_millis(10));

    let intercepted = Command::new(&rocminfo)
        .env("LD_PRELOAD", &cdylib)
        .env("MIRAGE_INTERCEPTOR_SOCKET", &sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rocminfo under interceptor");
    let intercepted_out = String::from_utf8_lossy(&intercepted.stdout).to_string();
    let intercepted_err = String::from_utf8_lossy(&intercepted.stderr).to_string();
    let intercepted_norm = normalize(&intercepted_out);

    drop(server_thread);
    let _ = std::fs::remove_file(&sock);

    Some(RocminfoRuns {
        baseline_out,
        baseline_norm,
        intercepted_out,
        intercepted_err,
        intercepted_norm,
    })
}

#[test]
fn rocminfo_smoke_reaches_real_backed_daemon() {
    let Some(runs) = collect_runs() else {
        return;
    };

    if runs.intercepted_norm.is_empty() {
        eprintln!(
            "rocminfo under interceptor produced no device lines. \
             The daemon-backed path runs, but the interceptor still lacks \
             enough ioctl/fs marshalling to reproduce baseline output. \
             baseline lines:\n{:#?}\n\nintercepted stderr:\n{}",
            runs.baseline_norm, runs.intercepted_err
        );
        return;
    }

    assert_eq!(
        runs.intercepted_norm, runs.baseline_norm,
        "rocminfo output under the interceptor diverged from the real kernel\n\
         baseline stdout:\n{}\n\
         intercepted stdout:\n{}\n\
         intercepted stderr:\n{}",
        runs.baseline_out, runs.intercepted_out, runs.intercepted_err
    );
}

#[test]
fn rocminfo_matches_when_hardware_present() {
    let Some(runs) = collect_runs() else {
        return;
    };

    assert!(
        !runs.intercepted_norm.is_empty(),
        "rocminfo under interceptor produced no device lines\n\
         baseline stdout:\n{}\n\
         intercepted stdout:\n{}\n\
         intercepted stderr:\n{}",
        runs.baseline_out,
        runs.intercepted_out,
        runs.intercepted_err
    );

    assert_eq!(
        runs.intercepted_norm, runs.baseline_norm,
        "rocminfo output under the interceptor diverged from the real kernel\n\
         baseline stdout:\n{}\n\
         intercepted stdout:\n{}\n\
         intercepted stderr:\n{}",
        runs.baseline_out, runs.intercepted_out, runs.intercepted_err
    );
}
