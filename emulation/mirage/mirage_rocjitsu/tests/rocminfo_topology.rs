use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mirage_interceptor::MIRAGE_SOCKET_ENV;
use mirage_remote::EmulatorServer;
use mirage_rocjitsu::RocjitsuEmulator;

#[test]
fn rocminfo_lists_simulated_gpu_agents() {
    let Some((config_path, schema_path)) = rocjitsu_kmd_paths() else {
        eprintln!("rocjitsu source tree not found; skipping rocminfo topology test");
        return;
    };
    let Some(interceptor_so) = find_interceptor_so() else {
        eprintln!("libmirage_interceptor.so not found; skipping rocminfo topology test");
        return;
    };

    let emulator = match RocjitsuEmulator::from_kmd_config(&config_path, &schema_path) {
        Ok(emulator) => emulator,
        Err(err) => {
            eprintln!("rj_kmd_create failed ({err}); likely linked against stub; skipping");
            return;
        }
    };
    let socket_path = unique_socket("rocminfo-topology");
    let server = EmulatorServer::new(socket_path.clone(), emulator);
    let listener = server.bind().expect("bind rocminfo test socket");
    let _server_thread = thread::spawn(move || {
        let _ = server.serve_on(listener);
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut attempts = 0;
    let last_output = loop {
        attempts += 1;
        let output = match Command::new("rocminfo")
            .env("LD_PRELOAD", &interceptor_so)
            .env(MIRAGE_SOCKET_ENV, &socket_path)
            .env_remove("HSA_MODEL_TOPOLOGY")
            .output()
        {
            Ok(output) => output,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("rocminfo not found; skipping rocminfo topology test");
                return;
            }
            Err(err) => panic!("failed to run rocminfo: {err}"),
        };

        let combined_output = format!(
            "status: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if output.status.success() && rocminfo_has_simulated_gpu(&combined_output) {
            return;
        }
        if Instant::now() >= deadline {
            break combined_output;
        }
        thread::sleep(Duration::from_millis(250));
    };

    panic!(
        "rocminfo did not list the simulated MI300X GPU after {attempts} attempts\n{}",
        trim_output(&last_output)
    );
}

fn rocminfo_has_simulated_gpu(output: &str) -> bool {
    output.contains("AMD Instinct MI300X") || output.contains("gfx942")
}

fn trim_output(output: &str) -> String {
    const MAX: usize = 16 * 1024;
    if output.len() <= MAX {
        output.to_string()
    } else {
        format!("{}\n... truncated ...", &output[..MAX])
    }
}

fn rocjitsu_kmd_paths() -> Option<(PathBuf, PathBuf)> {
    for ancestor in PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors() {
        let config = ancestor.join("experimental/rocjitsu/configs/amdgpu_cdna3_kmd.json");
        let schema = ancestor.join("experimental/rocjitsu/schemas/simulation_config.fbs");
        if config.is_file() && schema.is_file() {
            return Some((config, schema));
        }
    }
    None
}

fn find_interceptor_so() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(deps_dir) = exe.parent()
    {
        candidates.push(deps_dir.join("libmirage_interceptor.so"));
        if let Some(target_dir) = deps_dir.parent() {
            candidates.push(target_dir.join("libmirage_interceptor.so"));
        }
        candidates.extend(find_matching_interceptors(deps_dir));
    }

    for ancestor in PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors() {
        candidates.push(ancestor.join("target/debug/libmirage_interceptor.so"));
        candidates.push(ancestor.join("target/debug/deps/libmirage_interceptor.so"));
        candidates.push(ancestor.join("target/release/libmirage_interceptor.so"));
        candidates.push(ancestor.join("target/release/deps/libmirage_interceptor.so"));
    }

    candidates.into_iter().find(|path| path.is_file())
}

fn find_matching_interceptors(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("libmirage_interceptor") && name.ends_with(".so")
                })
        })
        .collect()
}

fn unique_socket(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("mirage-{tag}-{}-{nanos}.sock", std::process::id()))
}
