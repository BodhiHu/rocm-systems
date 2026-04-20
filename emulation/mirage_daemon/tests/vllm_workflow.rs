//! Integration tests for running vLLM using Mirage with the rocjitsu simulator.
//!
//! These tests exercise the complete daemon lifecycle that the `demo/vllm.sh`
//! script performs: profile creation → session boot → exec chains → shutdown.
//! All container interactions go through [`MockContainerRuntime`] so the tests
//! run without Docker or real GPU hardware.

use std::sync::Arc;

use mirage_container::{ContainerHandle, ExecResult, MockContainerRuntime};
use mirage_daemon::InMemoryMirageDaemon;
use mirage_schema::common::{
    CleanupPolicy, ExecArgs, GpuDef, GpuFamily, HealthStatus, ProfileDef, SimulatorMode,
    WorkloadDef,
};
use mirage_schema::config::DaemonDef;
use mirage_schema::daemon::{
    MirageDaemonBoot, MirageDaemonCreateProfile, MirageDaemonCreateSession,
    MirageDaemonCreateWorkload, MirageDaemonDeleteProfile, MirageDaemonDeleteSession,
    MirageDaemonExec, MirageDaemonGetSessionDetail,
    MirageDaemonGetSimulator, MirageDaemonGetWorkload, MirageDaemonHealth,
    MirageDaemonListProfiles, MirageDaemonListSessions, MirageDaemonListSimulators,
    MirageDaemonListWorkloads, MirageDaemonOverview, MirageDaemonRegistration, MirageDaemonShutdown,
    MirageDaemonTime,
};
use mirage_schema::simulator::SimulatorInfo;
use mirage_schema::socket::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const VLLM_IMAGE: &str =
    "docker.io/rocm/vllm:rocm7.12.0_gfx94X-dcgpu_ubuntu24.04_py3.12_pytorch_2.9.1_vllm_0.16.0";

fn mi300x_profile(name: &str) -> ProfileDef {
    ProfileDef {
        name: name.into(),
        simulator: "rocjitsu".into(),
        mode: SimulatorMode::Functional,
        gpu: "MI300X".into(),
        num_gpus: 8,
        num_nodes: 1,
    }
}

fn create_profile_req(p: ProfileDef) -> CreateProfileRequest {
    CreateProfileRequest {
        name: p.name,
        simulator: p.simulator,
        gpu: p.gpu,
        mode: p.mode,
        gpus_per_node: p.num_gpus,
        nodes: p.num_nodes,
    }
}

fn boot_req(name: &str, profile: &str, image: &str) -> BootSessionRequest {
    BootSessionRequest {
        name: name.into(),
        profile: profile.into(),
        image: image.into(),
    }
}

fn exec_req(session: &str, cmd: &str, args: &[&str]) -> ExecInSessionRequest {
    let mut command = vec![cmd.to_string()];
    command.extend(args.iter().map(|s| s.to_string()));
    ExecInSessionRequest {
        session_name: session.into(),
        command,
    }
}

fn workload_req(workload: WorkloadDef) -> CreateWorkloadRequest {
    let to_csv = |exec: ExecArgs| {
        let mut parts = vec![exec.command];
        parts.extend(exec.args);
        parts.join(",")
    };
    CreateWorkloadRequest {
        name: workload.name,
        profile: workload.profile,
        image: workload.image,
        startup: workload.startup.map(to_csv),
        execs: workload.execs.into_iter().map(to_csv).collect(),
        cleanup: workload.cleanup,
    }
}

async fn vllm_daemon() -> (InMemoryMirageDaemon, Arc<MockContainerRuntime>) {
    let mock = Arc::new(MockContainerRuntime::default());
    let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

    let reply = daemon
        .create_profile(create_profile_req(mi300x_profile("mi300x-func")))
        .await
        .unwrap();
    assert!(reply.ok, "profile creation failed: {:?}", reply.error);

    (daemon, mock)
}

/// Queue a successful exec result returning `stdout` on the head container.
async fn queue_ok(mock: &MockContainerRuntime, boot: &BootSessionReply, stdout: &[u8]) {
    let starts = mock.start_requests().await;
    let handle = ContainerHandle {
        id: boot.container_id.clone().unwrap(),
        name: starts.last().unwrap().name.clone(),
    };
    mock.queue_exec_result(
        &handle,
        ExecResult {
            exit_code: 0,
            stdout: stdout.to_vec(),
            stderr: vec![],
        },
    )
    .await;
}

/// Queue a failing exec result returning `stderr` on the head container.
async fn queue_fail(
    mock: &MockContainerRuntime,
    boot: &BootSessionReply,
    exit_code: i32,
    stderr: &[u8],
) {
    let starts = mock.start_requests().await;
    let handle = ContainerHandle {
        id: boot.container_id.clone().unwrap(),
        name: starts.last().unwrap().name.clone(),
    };
    mock.queue_exec_result(
        &handle,
        ExecResult {
            exit_code,
            stdout: vec![],
            stderr: stderr.to_vec(),
        },
    )
    .await;
}

// ===========================================================================
// 1. vLLM E2E workflow (mirrors demo/vllm.sh steps)
// ===========================================================================

/// Full vLLM end-to-end lifecycle: overview → simulators → profile →
/// boot → multiple execs → session detail → shutdown → verify cleanup.
#[tokio::test]
async fn vllm_full_e2e_lifecycle() {
    let (daemon, mock) = vllm_daemon().await;

    // -- overview -----------------------------------------------------------
    let overview = daemon
        .get_overview(GetOverviewRequest::default())
        .await
        .unwrap();
    assert!(overview.simulator_count >= 1);
    assert_eq!(overview.profile_count, 1);
    assert_eq!(overview.session_count, 0);

    // -- list built-in simulators -------------------------------------------
    let sims = daemon
        .list_simulators(ListSimulatorsRequest::default())
        .await
        .unwrap();
    assert!(!sims.simulators.is_empty());
    let rocjitsu = sims
        .simulators
        .iter()
        .find(|s| s.name.as_deref() == Some("rocjitsu"));
    assert!(rocjitsu.is_some(), "rocjitsu must be a builtin simulator");
    let rocjitsu = rocjitsu.unwrap();
    assert!(rocjitsu
        .supported_gpus
        .iter()
        .any(|g| g.name == "MI300X"));

    // -- boot vLLM session --------------------------------------------------
    let boot = daemon
        .boot_session(boot_req("vllm-e2e", "mi300x-func", VLLM_IMAGE))
        .await
        .unwrap();
    assert!(boot.ok, "boot failed: {:?}", boot.error);
    assert!(boot.container_id.is_some());

    // Overview should now show 1 session.
    let overview = daemon
        .get_overview(GetOverviewRequest::default())
        .await
        .unwrap();
    assert_eq!(overview.session_count, 1);

    // -- exec: Python version -----------------------------------------------
    queue_ok(&mock, &boot, b"Python 3.12.0\n").await;
    let reply = daemon
        .exec_in_session(exec_req(
            "vllm-e2e",
            "python",
            &["-c", "import sys; print(f'Python {sys.version}')"],
        ))
        .await
        .unwrap();
    assert_eq!(reply.exit_code, 0);
    assert!(String::from_utf8_lossy(&reply.stdout).contains("Python"));

    // -- exec: PyTorch version + ROCm info ----------------------------------
    queue_ok(
        &mock,
        &boot,
        b"PyTorch 2.9.1\nCUDA available: True\nROCm built: True\n",
    )
    .await;
    let reply = daemon
        .exec_in_session(exec_req(
            "vllm-e2e",
            "python",
            &[
                "-c",
                "import torch; print(f'PyTorch {torch.__version__}')",
            ],
        ))
        .await
        .unwrap();
    assert_eq!(reply.exit_code, 0);
    assert!(String::from_utf8_lossy(&reply.stdout).contains("PyTorch"));

    // -- exec: vLLM import --------------------------------------------------
    queue_ok(&mock, &boot, b"vLLM 0.16.0\n").await;
    let reply = daemon
        .exec_in_session(exec_req(
            "vllm-e2e",
            "python",
            &["-c", "import vllm; print(f'vLLM {vllm.__version__}')"],
        ))
        .await
        .unwrap();
    assert_eq!(reply.exit_code, 0);
    assert!(String::from_utf8_lossy(&reply.stdout).contains("vLLM"));

    // -- exec: readiness report (JSON) --------------------------------------
    let report = serde_json::json!({
        "vllm_version": "0.16.0",
        "pytorch_version": "2.9.1",
        "rocm_build": true,
        "hip_version": "6.4.0",
        "cuda_available": true,
        "gpu_count": 8,
        "gpu_name": "AMD Instinct MI300X (simulated)",
        "mirage_interceptor": true,
        "status": "ready"
    });
    queue_ok(
        &mock,
        &boot,
        format!("{}\n", serde_json::to_string_pretty(&report).unwrap()).as_bytes(),
    )
    .await;
    let reply = daemon
        .exec_in_session(exec_req(
            "vllm-e2e",
            "python",
            &["-c", "import json, torch, vllm; print(json.dumps({...}))"],
        ))
        .await
        .unwrap();
    assert_eq!(reply.exit_code, 0);
    let parsed: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&reply.stdout).trim()).unwrap();
    assert_eq!(parsed["status"], "ready");
    assert_eq!(parsed["gpu_count"], 8);

    // -- exec: Qwen inference benchmark -------------------------------------
    let bench = serde_json::json!({
        "model": "Qwen/Qwen2.5-0.5B",
        "prompts": 4,
        "total_output_tokens": 512,
        "generation_time_s": 12.34,
        "throughput_tok_s": 41.5
    });
    queue_ok(
        &mock,
        &boot,
        format!("{}\nBenchmark passed.\n", serde_json::to_string_pretty(&bench).unwrap())
            .as_bytes(),
    )
    .await;
    let reply = daemon
        .exec_in_session(exec_req(
            "vllm-e2e",
            "python",
            &["-c", "from vllm import LLM, SamplingParams; ..."],
        ))
        .await
        .unwrap();
    assert_eq!(reply.exit_code, 0);
    assert!(String::from_utf8_lossy(&reply.stdout).contains("Benchmark passed"));

    // -- session detail -----------------------------------------------------
    let detail = daemon
        .get_session_detail(GetSessionDetailRequest {
            name: "vllm-e2e".into(),
        })
        .await
        .unwrap();
    assert_eq!(detail.name.as_deref(), Some("vllm-e2e"));
    assert_eq!(detail.simulator.as_deref(), Some("rocjitsu"));
    assert_eq!(detail.image.as_deref(), Some(VLLM_IMAGE));
    assert_eq!(detail.health, HealthStatus::Healthy);
    assert!(detail.profile.is_some());
    assert_eq!(detail.profile.as_ref().unwrap().gpu, "MI300X");
    assert_eq!(
        detail.profile.as_ref().unwrap().mode,
        SimulatorMode::Functional
    );

    // -- shutdown -----------------------------------------------------------
    let shutdown = daemon
        .shutdown_session(ShutdownSessionRequest {
            name: "vllm-e2e".into(),
        })
        .await
        .unwrap();
    assert!(shutdown.ok);

    // Verify the session is gone.
    let sessions = daemon
        .list_sessions(ListSessionsRequest::default())
        .await
        .unwrap();
    assert!(sessions.sessions.is_empty());

    // Overview should be clean.
    let overview = daemon
        .get_overview(GetOverviewRequest::default())
        .await
        .unwrap();
    assert_eq!(overview.session_count, 0);
}

// ===========================================================================
// 2. Rocjitsu simulator registration and GPU support
// ===========================================================================

#[tokio::test]
async fn rocjitsu_is_registered_at_startup() {
    let daemon = InMemoryMirageDaemon::new();

    let reply = daemon
        .get_simulator(GetSimulatorRequest {
            name: "rocjitsu".into(),
        })
        .await
        .unwrap();

    let sim = reply.simulator.expect("rocjitsu should be registered");
    assert_eq!(sim.name.as_deref(), Some("rocjitsu"));
    assert_eq!(sim.version.as_deref(), Some("0.5.0"));
    assert!(!sim.supports_custom_gpus);
    assert_eq!(sim.active_session_count, 0);

    // Should support MI300X, MI325X, MI350X.
    let gpu_names: Vec<_> = sim.supported_gpus.iter().map(|g| &g.name).collect();
    assert!(gpu_names.contains(&&"MI300X".to_string()));
    assert!(gpu_names.contains(&&"MI325X".to_string()));
    assert!(gpu_names.contains(&&"MI350X".to_string()));
}

#[tokio::test]
async fn rocjitsu_gpus_have_correct_architectures() {
    let daemon = InMemoryMirageDaemon::new();

    let sims = daemon
        .list_simulators(ListSimulatorsRequest::default())
        .await
        .unwrap();
    let rocjitsu = sims
        .simulators
        .iter()
        .find(|s| s.name.as_deref() == Some("rocjitsu"))
        .unwrap();

    for gpu in &rocjitsu.supported_gpus {
        match gpu.name.as_str() {
            "MI300X" | "MI325X" => {
                assert_eq!(gpu.arch, "gfx942", "{} should be gfx942", gpu.name);
                assert_eq!(gpu.family, GpuFamily::AmdCdna);
            }
            "MI350X" => {
                assert_eq!(gpu.arch, "gfx950", "MI350X should be gfx950");
                assert_eq!(gpu.family, GpuFamily::AmdCdna);
            }
            _ => panic!("unexpected GPU: {}", gpu.name),
        }
    }
}

#[tokio::test]
async fn rocjitsu_supports_functional_mode() {
    let daemon = InMemoryMirageDaemon::new();

    // Functional mode should succeed.
    let ok = daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "func".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "MI300X".into(),
            num_gpus: 1,
            num_nodes: 1,
        }))
        .await
        .unwrap();
    assert!(ok.ok);
}

#[tokio::test]
async fn rocjitsu_rejects_unsupported_mode() {
    let daemon = InMemoryMirageDaemon::new();

    // CycleAccurate should be rejected (rocjitsu only advertises Functional).
    let reply = daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "cycle".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::CycleAccurate,
            gpu: "MI300X".into(),
            num_gpus: 1,
            num_nodes: 1,
        }))
        .await
        .unwrap();
    assert!(!reply.ok);
    assert!(reply.error.unwrap().contains("mode"));
}

#[tokio::test]
async fn rocjitsu_rejects_unsupported_gpu() {
    let daemon = InMemoryMirageDaemon::new();

    let reply = daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "bad-gpu".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "RTX4090".into(),
            num_gpus: 1,
            num_nodes: 1,
        }))
        .await
        .unwrap();
    assert!(!reply.ok);
    assert!(reply.error.unwrap().contains("gpu"));
}

// ===========================================================================
// 3. Profile management for vLLM
// ===========================================================================

#[tokio::test]
async fn create_multiple_gpu_profiles() {
    let daemon = InMemoryMirageDaemon::new();

    for (name, gpu) in [("mi300x", "MI300X"), ("mi325x", "MI325X"), ("mi350x", "MI350X")] {
        let reply = daemon
            .create_profile(create_profile_req(ProfileDef {
                name: name.into(),
                simulator: "rocjitsu".into(),
                mode: SimulatorMode::Functional,
                gpu: gpu.into(),
                num_gpus: 8,
                num_nodes: 1,
            }))
            .await
            .unwrap();
        assert!(reply.ok, "profile {name} failed: {:?}", reply.error);
    }

    let profiles = daemon
        .list_profiles(ListProfilesRequest::default())
        .await
        .unwrap();
    assert_eq!(profiles.profiles.len(), 3);
}

#[tokio::test]
async fn profile_filter_by_simulator() {
    let daemon = InMemoryMirageDaemon::new();

    daemon
        .create_profile(create_profile_req(mi300x_profile("p1")))
        .await
        .unwrap();

    // Filter for rocjitsu should return the profile.
    let reply = daemon
        .list_profiles(ListProfilesRequest {
            simulator: Some("rocjitsu".into()),
        })
        .await
        .unwrap();
    assert_eq!(reply.profiles.len(), 1);

    // Filter for nonexistent simulator should return empty.
    let reply = daemon
        .list_profiles(ListProfilesRequest {
            simulator: Some("gem5".into()),
        })
        .await
        .unwrap();
    assert!(reply.profiles.is_empty());
}

#[tokio::test]
async fn profile_validation_edge_cases() {
    let daemon = InMemoryMirageDaemon::new();

    // Empty name.
    let r = daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "MI300X".into(),
            num_gpus: 1,
            num_nodes: 1,
        }))
        .await
        .unwrap();
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("empty"));

    // Zero GPUs.
    let r = daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "zero-gpu".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "MI300X".into(),
            num_gpus: 0,
            num_nodes: 1,
        }))
        .await
        .unwrap();
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("gpu"));

    // Zero nodes.
    let r = daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "zero-nodes".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "MI300X".into(),
            num_gpus: 1,
            num_nodes: 0,
        }))
        .await
        .unwrap();
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("node"));

    // Duplicate name.
    daemon
        .create_profile(create_profile_req(mi300x_profile("dup")))
        .await
        .unwrap();
    let r = daemon
        .create_profile(create_profile_req(mi300x_profile("dup")))
        .await
        .unwrap();
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("already exists"));
}

#[tokio::test]
async fn delete_profile_not_in_use() {
    let daemon = InMemoryMirageDaemon::new();

    daemon
        .create_profile(create_profile_req(mi300x_profile("deletable")))
        .await
        .unwrap();
    let r = daemon
        .delete_profile(DeleteProfileRequest {
            name: "deletable".into(),
        })
        .await
        .unwrap();
    assert!(r.ok);

    let list = daemon
        .list_profiles(ListProfilesRequest::default())
        .await
        .unwrap();
    assert!(list.profiles.is_empty());
}

#[tokio::test]
async fn cannot_delete_profile_with_active_session() {
    let (daemon, _mock) = vllm_daemon().await;

    let boot = daemon
        .boot_session(boot_req("del-profile-s1", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok, "boot failed: {:?}", boot.error);

    let r = daemon
        .delete_profile(DeleteProfileRequest {
            name: "mi300x-func".into(),
        })
        .await
        .unwrap();
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("still referenced"));
}

// ===========================================================================
// 4. Container boot configuration
// ===========================================================================

#[tokio::test]
async fn boot_sets_mirage_session_env() {
    let (daemon, mock) = vllm_daemon().await;

    let boot = daemon
        .boot_session(boot_req("env-test", "mi300x-func", VLLM_IMAGE))
        .await
        .unwrap();
    assert!(boot.ok);

    let starts = mock.start_requests().await;
    assert_eq!(starts.len(), 1);

    let env = &starts[0].container.entrypoint.env;
    let session_env = env.iter().find(|e| e.key == "MIRAGE_SESSION");
    assert!(session_env.is_some(), "MIRAGE_SESSION env should be set");
    assert_eq!(session_env.unwrap().value, "env-test");
}

#[tokio::test]
async fn boot_uses_sleep_infinity_entrypoint() {
    let (daemon, mock) = vllm_daemon().await;

    daemon
        .boot_session(boot_req("entry-test", "mi300x-func", VLLM_IMAGE))
        .await
        .unwrap();

    let starts = mock.start_requests().await;
    assert_eq!(starts[0].container.entrypoint.command, "sleep");
    assert_eq!(starts[0].container.entrypoint.args, vec!["infinity"]);
}

#[tokio::test]
async fn boot_uses_correct_image() {
    let (daemon, mock) = vllm_daemon().await;

    daemon
        .boot_session(boot_req("img-test", "mi300x-func", VLLM_IMAGE))
        .await
        .unwrap();

    let starts = mock.start_requests().await;
    assert_eq!(starts[0].container.image, VLLM_IMAGE);
}

#[tokio::test]
async fn boot_pulls_image() {
    let (daemon, mock) = vllm_daemon().await;

    daemon
        .boot_session(boot_req("pull-test", "mi300x-func", VLLM_IMAGE))
        .await
        .unwrap();

    let pulled = mock.pulled_images().await;
    assert!(pulled.contains(&VLLM_IMAGE.to_string()));
}

#[tokio::test]
async fn boot_rejects_empty_session_name() {
    let (daemon, _mock) = vllm_daemon().await;

    let r = daemon
        .boot_session(boot_req("", "mi300x-func", VLLM_IMAGE))
        .await
        .unwrap();
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("empty"));
}

#[tokio::test]
async fn boot_rejects_duplicate_session_name() {
    let (daemon, _mock) = vllm_daemon().await;

    let r1 = daemon
        .boot_session(boot_req("dup-test", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(r1.ok);

    let r2 = daemon
        .boot_session(boot_req("dup-test", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(!r2.ok);
    assert!(r2.error.unwrap().contains("already exists"));
}

#[tokio::test]
async fn boot_without_container_runtime_fails() {
    let daemon = InMemoryMirageDaemon::new();
    daemon
        .create_profile(create_profile_req(mi300x_profile("p")))
        .await
        .unwrap();

    let r = daemon
        .boot_session(boot_req("s", "p", "img:latest"))
        .await
        .unwrap();
    assert!(!r.ok);
    assert!(r.error.unwrap().contains("container runtime"));
}

// ===========================================================================
// 5. Exec in vLLM session
// ===========================================================================

#[tokio::test]
async fn exec_returns_stdout_and_stderr() {
    let (daemon, mock) = vllm_daemon().await;

    let boot = daemon
        .boot_session(boot_req("exec-io", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok);

    let starts = mock.start_requests().await;
    let handle = ContainerHandle {
        id: boot.container_id.clone().unwrap(),
        name: starts[0].name.clone(),
    };
    mock.queue_exec_result(
        &handle,
        ExecResult {
            exit_code: 0,
            stdout: b"stdout data".to_vec(),
            stderr: b"stderr data".to_vec(),
        },
    )
    .await;

    let reply = daemon
        .exec_in_session(exec_req("exec-io", "echo", &["hello"]))
        .await
        .unwrap();
    assert_eq!(reply.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&reply.stdout), "stdout data");
    assert_eq!(String::from_utf8_lossy(&reply.stderr), "stderr data");
}

#[tokio::test]
async fn exec_propagates_nonzero_exit_code() {
    let (daemon, mock) = vllm_daemon().await;

    let boot = daemon
        .boot_session(boot_req("exit-code", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok);

    queue_fail(&mock, &boot, 42, b"segfault\n").await;

    let reply = daemon
        .exec_in_session(exec_req("exit-code", "bad-program", &[]))
        .await
        .unwrap();
    assert_eq!(reply.exit_code, 42);
    assert!(String::from_utf8_lossy(&reply.stderr).contains("segfault"));
}

#[tokio::test]
async fn exec_multiple_commands_sequentially() {
    let (daemon, mock) = vllm_daemon().await;

    let boot = daemon
        .boot_session(boot_req("seq-exec", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok);

    let commands = vec![
        ("python --version", "Python 3.12.0\n"),
        ("rocminfo", "Agent: MI300X\n"),
        ("pip list", "vllm 0.16.0\ntorch 2.9.1\n"),
    ];

    for (cmd, expected_out) in &commands {
        queue_ok(&mock, &boot, expected_out.as_bytes()).await;
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let reply = daemon
            .exec_in_session(exec_req("seq-exec", parts[0], &parts[1..]))
            .await
            .unwrap();
        assert_eq!(reply.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&reply.stdout), *expected_out);
    }
}

#[tokio::test]
async fn exec_on_nonexistent_session_fails() {
    let (daemon, _mock) = vllm_daemon().await;

    let result = daemon
        .exec_in_session(exec_req("ghost", "echo", &["hello"]))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn exec_on_unbooted_session_fails() {
    let (daemon, _mock) = vllm_daemon().await;

    // Create session without booting.
    daemon
        .create_session(CreateSessionRequest {
            name: "unbooted".into(),
            profile: "mi300x-func".into(),
            image: "img:latest".into(),
        })
        .await
        .unwrap();

    let result = daemon
        .exec_in_session(exec_req("unbooted", "echo", &[]))
        .await;
    assert!(result.is_err());
}

// ===========================================================================
// 6. Session health and detail queries
// ===========================================================================

#[tokio::test]
async fn daemon_health_is_healthy_by_default() {
    let daemon = InMemoryMirageDaemon::new();

    let reply = daemon
        .health(HealthRequest { session_id: None })
        .await
        .unwrap();
    assert!(reply.healthy);
    assert_eq!(reply.status, HealthStatus::Healthy);
}

#[tokio::test]
async fn session_health_for_booted_session() {
    let (daemon, _mock) = vllm_daemon().await;

    let boot = daemon
        .boot_session(boot_req("health-test", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok);

    let reply = daemon
        .health(HealthRequest {
            session_id: Some("health-test".into()),
        })
        .await
        .unwrap();
    assert!(reply.healthy);
    assert_eq!(reply.status, HealthStatus::Healthy);
}

#[tokio::test]
async fn health_for_nonexistent_session_fails() {
    let daemon = InMemoryMirageDaemon::new();

    let result = daemon
        .health(HealthRequest {
            session_id: Some("ghost".into()),
        })
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn time_returns_default_for_daemon() {
    let daemon = InMemoryMirageDaemon::new();

    let reply = daemon
        .time(TimeRequest { session_id: None })
        .await
        .unwrap();
    assert_eq!(reply.time.seconds, 0);
    assert_eq!(reply.time.picoseconds, 0);
}

#[tokio::test]
async fn time_for_nonexistent_session_fails() {
    let daemon = InMemoryMirageDaemon::new();

    let result = daemon
        .time(TimeRequest {
            session_id: Some("ghost".into()),
        })
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn session_detail_shows_profile_and_simulator() {
    let (daemon, _mock) = vllm_daemon().await;

    daemon
        .boot_session(boot_req("detail-test", "mi300x-func", VLLM_IMAGE))
        .await
        .unwrap();

    let detail = daemon
        .get_session_detail(GetSessionDetailRequest {
            name: "detail-test".into(),
        })
        .await
        .unwrap();

    assert_eq!(detail.name.as_deref(), Some("detail-test"));
    assert_eq!(detail.simulator.as_deref(), Some("rocjitsu"));
    assert_eq!(detail.image.as_deref(), Some(VLLM_IMAGE));
    assert_eq!(detail.health, HealthStatus::Healthy);

    let profile = detail.profile.unwrap();
    assert_eq!(profile.name, "mi300x-func");
    assert_eq!(profile.gpu, "MI300X");
    assert_eq!(profile.num_gpus, 8);
    assert_eq!(profile.num_nodes, 1);
    assert_eq!(profile.mode, SimulatorMode::Functional);
}

#[tokio::test]
async fn session_detail_nonexistent_fails() {
    let daemon = InMemoryMirageDaemon::new();

    let result = daemon
        .get_session_detail(GetSessionDetailRequest {
            name: "nope".into(),
        })
        .await;
    assert!(result.is_err());
}

// ===========================================================================
// 7. Session lifecycle: create, list, delete, filter
// ===========================================================================

#[tokio::test]
async fn session_filter_by_profile() {
    let mock = Arc::new(MockContainerRuntime::default());
    let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

    // Create two profiles.
    daemon
        .create_profile(create_profile_req(mi300x_profile("p1")))
        .await
        .unwrap();
    daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "p2".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "MI325X".into(),
            num_gpus: 4,
            num_nodes: 1,
        }))
        .await
        .unwrap();

    // Boot sessions on different profiles.
    daemon
        .boot_session(boot_req("filter-s1", "p1", "img:1"))
        .await
        .unwrap();
    daemon
        .boot_session(boot_req("filter-s2", "p2", "img:2"))
        .await
        .unwrap();

    // Filter by p1.
    let reply = daemon
        .list_sessions(ListSessionsRequest {
            profile: Some("p1".into()),
        })
        .await
        .unwrap();
    assert_eq!(reply.sessions.len(), 1);
    assert_eq!(reply.sessions[0].name.as_deref(), Some("filter-s1"));

    // No filter → both.
    let reply = daemon
        .list_sessions(ListSessionsRequest::default())
        .await
        .unwrap();
    assert_eq!(reply.sessions.len(), 2);
}

#[tokio::test]
async fn delete_session_without_boot() {
    let daemon = InMemoryMirageDaemon::new();
    daemon
        .create_profile(create_profile_req(mi300x_profile("p")))
        .await
        .unwrap();
    daemon
        .create_session(CreateSessionRequest {
            name: "del".into(),
            profile: "p".into(),
            image: "img".into(),
        })
        .await
        .unwrap();

    let r = daemon
        .delete_session(DeleteSessionRequest {
            name: "del".into(),
        })
        .await
        .unwrap();
    assert!(r.ok);

    let sessions = daemon
        .list_sessions(ListSessionsRequest::default())
        .await
        .unwrap();
    assert!(sessions.sessions.is_empty());
}

// ===========================================================================
// 8. Shutdown and cleanup
// ===========================================================================

#[tokio::test]
async fn shutdown_cleans_up_container() {
    let (daemon, _mock) = vllm_daemon().await;

    let boot = daemon
        .boot_session(boot_req("cleanup", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok);

    let shutdown = daemon
        .shutdown_session(ShutdownSessionRequest {
            name: "cleanup".into(),
        })
        .await
        .unwrap();
    assert!(shutdown.ok);

    // Session list should be empty after shutdown.
    let sessions = daemon
        .list_sessions(ListSessionsRequest::default())
        .await
        .unwrap();
    assert!(sessions.sessions.is_empty());

    // Overview should reflect no sessions.
    let overview = daemon
        .get_overview(GetOverviewRequest::default())
        .await
        .unwrap();
    assert_eq!(overview.session_count, 0);
}

#[tokio::test]
async fn shutdown_then_reboot_same_name() {
    let (daemon, _mock) = vllm_daemon().await;

    // First boot + shutdown.
    let boot1 = daemon
        .boot_session(boot_req("reuse-test", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot1.ok);
    daemon
        .shutdown_session(ShutdownSessionRequest {
            name: "reuse-test".into(),
        })
        .await
        .unwrap();

    // Reboot with same name should succeed.
    let boot2 = daemon
        .boot_session(boot_req("reuse-test", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot2.ok, "reboot failed: {:?}", boot2.error);
}

// ===========================================================================
// 9. Multi-node vLLM clusters
// ===========================================================================

#[tokio::test]
async fn multinode_vllm_cluster_boot() {
    let mock = Arc::new(MockContainerRuntime::default());
    let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

    daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "mi300x-2x8".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "MI300X".into(),
            num_gpus: 8,
            num_nodes: 2,
        }))
        .await
        .unwrap();

    let boot = daemon
        .boot_session(boot_req("vllm-cluster", "mi300x-2x8", VLLM_IMAGE))
        .await
        .unwrap();
    assert!(boot.ok, "cluster boot failed: {:?}", boot.error);
    assert_eq!(boot.container_ids.len(), 2);

    let starts = mock.start_requests().await;
    assert_eq!(starts.len(), 2);

    // Verify node naming.
    assert_eq!(starts[0].name, "mirage-vllm-cluster-node0");
    assert_eq!(starts[1].name, "mirage-vllm-cluster-node1");

    // Verify both are on the same Docker network.
    assert_eq!(
        starts[0].container.network.as_deref(),
        Some("mirage-vllm-cluster")
    );
    assert_eq!(
        starts[1].container.network.as_deref(),
        Some("mirage-vllm-cluster")
    );

    // Verify distributed env vars on head node.
    let head_env = &starts[0].container.entrypoint.env;
    let find_env = |envs: &[mirage_schema::common::SetEnv], key: &str| -> Option<String> {
        envs.iter().find(|e| e.key == key).map(|e| e.value.clone())
    };
    assert_eq!(find_env(head_env, "MIRAGE_NUM_NODES"), Some("2".into()));
    assert_eq!(find_env(head_env, "MIRAGE_NODE_RANK"), Some("0".into()));
    assert_eq!(
        find_env(head_env, "MIRAGE_HEAD_ADDR"),
        Some("mirage-vllm-cluster-node0".into())
    );
    assert_eq!(
        find_env(head_env, "MIRAGE_HEAD_PORT"),
        Some("29500".into())
    );

    // Verify distributed env vars on worker node.
    let worker_env = &starts[1].container.entrypoint.env;
    assert_eq!(find_env(worker_env, "MIRAGE_NUM_NODES"), Some("2".into()));
    assert_eq!(find_env(worker_env, "MIRAGE_NODE_RANK"), Some("1".into()));
    assert_eq!(
        find_env(worker_env, "MIRAGE_HEAD_ADDR"),
        Some("mirage-vllm-cluster-node0".into())
    );
}

#[tokio::test]
async fn multinode_shutdown_removes_network() {
    let mock = Arc::new(MockContainerRuntime::default());
    let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

    daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "multi".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "MI300X".into(),
            num_gpus: 8,
            num_nodes: 3,
        }))
        .await
        .unwrap();

    let boot = daemon
        .boot_session(boot_req("multi-sd", "multi", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok);
    assert_eq!(boot.container_ids.len(), 3);

    // Network should exist.
    let networks = mock.networks().await;
    assert_eq!(networks.len(), 1);

    // Shutdown.
    let sd = daemon
        .shutdown_session(ShutdownSessionRequest {
            name: "multi-sd".into(),
        })
        .await
        .unwrap();
    assert!(sd.ok);

    // Network should be cleaned up.
    let networks = mock.networks().await;
    assert!(networks.is_empty());
}

// ===========================================================================
// 10. Simulator registration
// ===========================================================================

#[tokio::test]
async fn register_custom_simulator() {
    let daemon = InMemoryMirageDaemon::new();

    let reply = daemon
        .register_sim(RegisterSimRequest {
            info: SimulatorInfo {
                name: "gem5-gpu".into(),
                version: "1.0.0".into(),
                description: Some("gem5 GPU sim".into()),
                supported_gpus: vec![GpuDef {
                    name: "TestGPU".into(),
                    arch: "gfx900".into(),
                    family: GpuFamily::AmdCdna,
                    description: None,
                }],
                supports_custom_gpus: true,
                supported_modes: vec![
                    SimulatorMode::Functional,
                    SimulatorMode::CycleAccurate,
                ],
            },
        })
        .await
        .unwrap();
    assert!(reply.ok);

    // Should now be listed.
    let sims = daemon
        .list_simulators(ListSimulatorsRequest::default())
        .await
        .unwrap();
    assert!(sims
        .simulators
        .iter()
        .any(|s| s.name.as_deref() == Some("gem5-gpu")));

    // Can create profiles on the new simulator.
    let r = daemon
        .create_profile(create_profile_req(ProfileDef {
            name: "gem5-test".into(),
            simulator: "gem5-gpu".into(),
            mode: SimulatorMode::CycleAccurate,
            gpu: "TestGPU".into(),
            num_gpus: 1,
            num_nodes: 1,
        }))
        .await
        .unwrap();
    assert!(r.ok);
}

#[tokio::test]
async fn register_simulator_with_empty_name_fails() {
    let daemon = InMemoryMirageDaemon::new();

    let reply = daemon
        .register_sim(RegisterSimRequest {
            info: SimulatorInfo {
                name: "".into(),
                version: "1.0".into(),
                description: None,
                supported_gpus: vec![],
                supports_custom_gpus: false,
                supported_modes: vec![],
            },
        })
        .await
        .unwrap();
    assert!(!reply.ok);
}

#[tokio::test]
async fn active_session_count_tracks_sessions() {
    let (daemon, _mock) = vllm_daemon().await;

    // No sessions yet.
    let sim = daemon
        .get_simulator(GetSimulatorRequest {
            name: "rocjitsu".into(),
        })
        .await
        .unwrap()
        .simulator
        .unwrap();
    assert_eq!(sim.active_session_count, 0);

    // Boot a session.
    daemon
        .boot_session(boot_req("count-s1", "mi300x-func", "img:latest"))
        .await
        .unwrap();

    let sim = daemon
        .get_simulator(GetSimulatorRequest {
            name: "rocjitsu".into(),
        })
        .await
        .unwrap()
        .simulator
        .unwrap();
    assert_eq!(sim.active_session_count, 1);

    // Shutdown.
    daemon
        .shutdown_session(ShutdownSessionRequest {
            name: "count-s1".into(),
        })
        .await
        .unwrap();

    let sim = daemon
        .get_simulator(GetSimulatorRequest {
            name: "rocjitsu".into(),
        })
        .await
        .unwrap()
        .simulator
        .unwrap();
    assert_eq!(sim.active_session_count, 0);
}

// ===========================================================================
// 11. DaemonDef config-based initialization
// ===========================================================================

#[tokio::test]
async fn from_config_preloads_profiles() {
    let config = DaemonDef {
        profiles: vec![
            ProfileDef {
                name: "mi300x-func".into(),
                simulator: "rocjitsu".into(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".into(),
                num_gpus: 8,
                num_nodes: 1,
            },
            ProfileDef {
                name: "mi350x-func".into(),
                simulator: "rocjitsu".into(),
                mode: SimulatorMode::Functional,
                gpu: "MI350X".into(),
                num_gpus: 4,
                num_nodes: 1,
            },
        ],
    };

    let daemon = InMemoryMirageDaemon::from_config(config);
    let profiles = daemon
        .list_profiles(ListProfilesRequest::default())
        .await
        .unwrap();
    assert_eq!(profiles.profiles.len(), 2);
}

#[tokio::test]
async fn from_config_with_runtime_can_boot() {
    let mock = Arc::new(MockContainerRuntime::default());
    let config = DaemonDef {
        profiles: vec![ProfileDef {
            name: "auto".into(),
            simulator: "rocjitsu".into(),
            mode: SimulatorMode::Functional,
            gpu: "MI300X".into(),
            num_gpus: 1,
            num_nodes: 1,
        }],
    };

    let daemon = InMemoryMirageDaemon::from_config_with_runtime(config, mock.clone());
    let boot = daemon
        .boot_session(boot_req("auto-boot", "auto", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok, "boot failed: {:?}", boot.error);
}

// ===========================================================================
// 12. vLLM workload definitions
// ===========================================================================

#[tokio::test]
async fn create_vllm_benchmark_workload() {
    let (daemon, _mock) = vllm_daemon().await;

    let workload = WorkloadDef {
        name: "vllm-qwen-benchmark".into(),
        profile: "mi300x-func".into(),
        image: VLLM_IMAGE.into(),
        startup: Some(ExecArgs {
            command: "python".into(),
            args: vec![
                "-c".into(),
                "import vllm; print(f'vLLM {vllm.__version__}')".into(),
            ],
            env: vec![],
        }),
        execs: vec![
            ExecArgs {
                command: "python".into(),
                args: vec![
                    "-c".into(),
                    "import torch; print(torch.cuda.is_available())".into(),
                ],
                env: vec![],
            },
            ExecArgs {
                command: "python".into(),
                args: vec![
                    "-c".into(),
                    "from vllm import LLM; llm = LLM('Qwen/Qwen2.5-0.5B')".into(),
                ],
                env: vec![],
            },
        ],
        cleanup: CleanupPolicy::OnSuccess,
    };

    let reply = daemon
        .create_workload(workload_req(workload))
        .await
        .unwrap();
    assert!(reply.ok, "workload creation failed: {:?}", reply.error);

    // Verify the workload can be retrieved with all fields.
    let got = daemon
        .get_workload(GetWorkloadRequest {
            name: "vllm-qwen-benchmark".into(),
        })
        .await
        .unwrap()
        .workload
        .unwrap();
    assert_eq!(got.name, "vllm-qwen-benchmark");
    assert_eq!(got.profile, "mi300x-func");
    assert_eq!(got.image, VLLM_IMAGE);
    assert!(got.startup.is_some());
    assert_eq!(got.execs.len(), 2);
    assert_eq!(got.cleanup, CleanupPolicy::OnSuccess);
}

#[tokio::test]
async fn workload_summary_fields() {
    let (daemon, _mock) = vllm_daemon().await;

    daemon
        .create_workload(workload_req(WorkloadDef {
            name: "summary-test".into(),
            profile: "mi300x-func".into(),
            image: VLLM_IMAGE.into(),
            startup: Some(ExecArgs {
                command: "init".into(),
                args: vec![],
                env: vec![],
            }),
            execs: vec![
                ExecArgs {
                    command: "step1".into(),
                    args: vec![],
                    env: vec![],
                },
                ExecArgs {
                    command: "step2".into(),
                    args: vec![],
                    env: vec![],
                },
                ExecArgs {
                    command: "step3".into(),
                    args: vec![],
                    env: vec![],
                },
            ],
            cleanup: CleanupPolicy::Never,
        }))
        .await
        .unwrap();

    let list = daemon
        .list_workloads(ListWorkloadsRequest::default())
        .await
        .unwrap();
    assert_eq!(list.workloads.len(), 1);

    let summary = &list.workloads[0];
    assert_eq!(summary.name, "summary-test");
    assert_eq!(summary.profile, "mi300x-func");
    assert_eq!(summary.image, VLLM_IMAGE);
    assert!(summary.has_startup);
    assert_eq!(summary.exec_count, 3);
    assert_eq!(summary.cleanup, CleanupPolicy::Never);
}

// ===========================================================================
// 13. Error injection (container runtime failures)
// ===========================================================================

#[tokio::test]
async fn boot_handles_start_container_failure() {
    let mock = Arc::new(MockContainerRuntime::default());
    let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

    daemon
        .create_profile(create_profile_req(mi300x_profile("p")))
        .await
        .unwrap();

    mock.fail_next_start("simulated OOM").await;

    let reply = daemon
        .boot_session(boot_req("fail-boot", "p", "img:latest"))
        .await
        .unwrap();
    assert!(!reply.ok);
    assert!(reply.error.unwrap().contains("failed to start"));
}

#[tokio::test]
async fn exec_handles_runtime_error() {
    let (daemon, mock) = vllm_daemon().await;

    let boot = daemon
        .boot_session(boot_req("exec-fail", "mi300x-func", "img:latest"))
        .await
        .unwrap();
    assert!(boot.ok);

    let starts = mock.start_requests().await;
    let handle = ContainerHandle {
        id: boot.container_id.clone().unwrap(),
        name: starts[0].name.clone(),
    };
    mock.fail_next_exec(&handle, "container died").await;

    let result = daemon
        .exec_in_session(exec_req("exec-fail", "python", &["-c", "import vllm"]))
        .await;
    assert!(result.is_err());
}

// ===========================================================================
// 14. Concurrent sessions
// ===========================================================================

#[tokio::test]
async fn multiple_concurrent_vllm_sessions() {
    let (daemon, mock) = vllm_daemon().await;

    // Boot 3 sessions concurrently.
    let mut boots = vec![];
    for i in 0..3 {
        let boot = daemon
            .boot_session(boot_req(
                &format!("vllm-{i}"),
                "mi300x-func",
                VLLM_IMAGE,
            ))
            .await
            .unwrap();
        assert!(boot.ok, "boot vllm-{i} failed: {:?}", boot.error);
        boots.push(boot);
    }

    // All 3 sessions should be listed.
    let sessions = daemon
        .list_sessions(ListSessionsRequest::default())
        .await
        .unwrap();
    assert_eq!(sessions.sessions.len(), 3);

    // Overview should show 3.
    let overview = daemon
        .get_overview(GetOverviewRequest::default())
        .await
        .unwrap();
    assert_eq!(overview.session_count, 3);

    // Simulator should show 3 active sessions.
    let sim = daemon
        .get_simulator(GetSimulatorRequest {
            name: "rocjitsu".into(),
        })
        .await
        .unwrap()
        .simulator
        .unwrap();
    assert_eq!(sim.active_session_count, 3);

    // Each session should have its own container.
    let starts = mock.start_requests().await;
    assert_eq!(starts.len(), 3);

    // Shutdown all.
    for i in 0..3 {
        let sd = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: format!("vllm-{i}"),
            })
            .await
            .unwrap();
        assert!(sd.ok);
    }

    let sessions = daemon
        .list_sessions(ListSessionsRequest::default())
        .await
        .unwrap();
    assert!(sessions.sessions.is_empty());
}

// ===========================================================================
// 15. Session list summary fields
// ===========================================================================

#[tokio::test]
async fn session_summary_contains_expected_fields() {
    let (daemon, _mock) = vllm_daemon().await;

    daemon
        .boot_session(boot_req("summary-s", "mi300x-func", VLLM_IMAGE))
        .await
        .unwrap();

    let sessions = daemon
        .list_sessions(ListSessionsRequest::default())
        .await
        .unwrap();
    assert_eq!(sessions.sessions.len(), 1);

    let s = &sessions.sessions[0];
    assert_eq!(s.name.as_deref(), Some("summary-s"));
    assert_eq!(s.profile.as_deref(), Some("mi300x-func"));
    assert_eq!(s.simulator.as_deref(), Some("rocjitsu"));
    assert_eq!(s.image.as_deref(), Some(VLLM_IMAGE));
    assert_eq!(s.health_status, HealthStatus::Healthy);
}
