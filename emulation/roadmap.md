# Mirage Roadmap

## Overview

Mirage is an AMD GPU simulator that runs unmodified HIP, PyTorch, and vLLM applications in containerized simulated nodes without physical AMD GPU hardware. This roadmap defines the phased feature plan, acceptance criteria, and test requirements for bringing Mirage from its current state to a scope-complete v1.

**Architecture target:** CDNA v1 (MI300X-class), scalable to 8+ nodes × 8 GPUs/node.
**Fidelity modes:** functional ISA-correct execution, plus cycle-accurate mode for selected hot kernels (≤1000× slowdown budget).
**Primary ROCm seam:** patched `libhsakmt` over `amdgpu_lite`-style device contract.

---

## Current State

The following subsystems are implemented:

- **Schema & core types** — `mirage_schema` with serde-compatible profile, daemon, socket, container, and syscall definitions.
- **Hardware forwarding** — `mirage_real` passes ioctls to real `/dev/kfd` and `/dev/dri` devices.
- **Rocjitsu bridge** — `mirage_rocjitsu` + `rocjitsu_sys` FFI to a C `SimulatedDriver` supporting MI300X/MI325X/MI350X in functional mode.
- **LD_PRELOAD interceptor** — `mirage_interceptor` replaces device opens with memfd cookies, proxies ioctls over a Unix socket to a remote emulator.
- **Socket transport** — `mirage_remote` with BARE-encoded client/server protocol.
- **Daemon** — `mirage_daemon` with in-memory state, simulator/profile/session management, container lifecycle, and async RPC.
- **CLI** — `mirage_ctl` with `simulators`, `profile`, `session`, `boot`, `exec`, `shutdown` commands.
- **Container runtime** — `mirage_container` Docker CLI wrapper with mock support.
- **Dashboard** — TypeScript/React skeleton with page routes for overview, simulators, profiles, sessions, topology editor.
- **UAPI bindings** — `mirage_uapi` bindgen from kernel KFD/DRM headers with marshal traits.
- **Integration test** — `rocminfo_all_devices` comparing real vs intercepted output.

---

## Phase A — Control Plane & Schemas (Months 0–3)

### Feature A1: Device Profile Schema v1

Define the canonical GPU/node/fabric topology schema used by all downstream components.

| Field | Description |
|-------|-------------|
| `device_id`, `family`, `marketing_sku` | Architectural identity |
| `cu_count`, `lds_size`, `sgpr_count`, `vgpr_count` | Compute topology |
| `hbm_capacity`, `hbm_bandwidth`, `hbm_stacks` | Memory config |
| `xgmi_links`, `pcie_lanes`, `fabric_bandwidth` | Fabric config |
| `num_gpus`, `num_nodes`, `node_layout` | Cluster shape |
| `timing_mode`, `timing_model_params` | Timing toggles |

**Acceptance criteria:**
- [ ] Schema defined as JSON Schema with `$schema` and `$id` fields.
- [ ] Profiles for MI300X (1×1, 1×8, 2×8) and MI350X (1×1) pass validation.
- [ ] Schema changes go through a versioning policy (breaking = major bump).
- [ ] `mirage_schema` Rust types generated or hand-maintained with serde round-trip tests.

**Tests:**
- `test_profile_schema_validates_golden_mi300x_1x8` — golden MI300X 8-GPU profile validates.
- `test_profile_schema_rejects_invalid_cu_count` — negative CU count is rejected.
- `test_profile_schema_round_trip` — serialize → deserialize preserves all fields.
- `test_profile_schema_version_compat` — v1 schema loads v1 profiles.

---

### Feature A2: Workload Manifest Schema

Define the schema for workload execution requests.

| Field | Description |
|-------|-------------|
| `container_image` | OCI image reference |
| `rocm_version_pin` | Required ROCm runtime version |
| `launch_command`, `args` | Entrypoint |
| `env_vars` | Environment overrides |
| `mounted_artifacts` | Host bind mounts |
| `tracing_flags` | Trace capture toggles |
| `replay_checkpoint` | Optional replay restore point |

**Acceptance criteria:**
- [ ] Schema defined as JSON Schema.
- [ ] Golden manifests for vLLM inference and PyTorch training validate.
- [ ] `mirage_ctl exec` accepts manifest files.

**Tests:**
- `test_manifest_schema_validates_vllm_golden` — vLLM manifest passes.
- `test_manifest_schema_rejects_missing_image` — missing `container_image` is rejected.
- `test_manifest_env_var_override` — env vars are correctly merged.

---

### Feature A3: Daemon Lifecycle & gRPC/REST API

Harden `miraged` as the central orchestration service.

**Acceptance criteria:**
- [ ] Daemon exposes health, time, simulator, profile, and session endpoints.
- [ ] Boot state machine covers: `Created → Booting → Ready → Running → ShuttingDown → Stopped → Failed`.
- [ ] Daemon persists profiles and session state across restarts (file-backed store).
- [ ] Structured logging with correlation IDs per session.
- [ ] Graceful shutdown drains active sessions.

**Tests:**
- `test_daemon_boot_shutdown_lifecycle` — create profile → boot session → exec → shutdown succeeds.
- `test_daemon_concurrent_sessions` — two sessions with different profiles coexist.
- `test_daemon_persistence_across_restart` — profiles survive daemon restart.
- `test_daemon_graceful_shutdown` — active sessions are drained on SIGTERM.
- `test_daemon_health_check` — health endpoint returns OK when daemon is ready.

---

### Feature A4: CLI Completeness

Ensure `miragectl` covers all daemon operations.

**Acceptance criteria:**
- [ ] All daemon endpoints reachable from CLI.
- [ ] `--json` flag on every command for scripting.
- [ ] Exit codes: 0 success, 1 operation failure, 2 CLI error.
- [ ] Shell completion generation (bash, zsh, fish).
- [ ] `miragectl logs --follow` streams session logs.

**Tests:**
- `test_cli_profile_crud` — create, list, show, delete profile round-trip.
- `test_cli_session_lifecycle` — boot, exec, show, shutdown via CLI.
- `test_cli_json_output_parseable` — JSON output parses as valid JSON with expected fields.
- `test_cli_exit_codes` — invalid commands return exit code 2.

---

### Feature A5: Monorepo Scaffolding & Wheel Runtime Contract

Bootstrap the development and runtime environment.

**Acceptance criteria:**
- [ ] `emulation/` builds independently from the top-level TheRock CMake graph.
- [ ] A bootstrap script creates a venv, installs pinned ROCm/PyTorch/vLLM wheels, and the Mirage `libhsakmt` overlay.
- [ ] CI smoke: environment creation + CLI startup + no-GPU host-only simulator smoke run.
- [ ] CODEOWNERS and CI configuration are present.

**Tests:**
- `test_bootstrap_creates_venv` — bootstrap script creates a working virtualenv.
- `test_bootstrap_installs_wheels` — required wheels are installed at pinned versions.
- `test_daemon_starts_without_gpu` — daemon starts successfully on a host without `/dev/kfd`.

---

## Phase B — Single-GPU Functional Execution (Months 3–7)

### Feature B1: SoC/Package Model & Topology Identity

Build a simulator-native MI300X/MI355X-class package model.

| Component | Description |
|-----------|-------------|
| `PackageModel` | Die/chiplet grouping, HBM stacks, SDMA engines |
| `DevicePersona` | Architectural identity (device_id, SKU, family) |
| Stable IDs | `package_uuid`, `gpu_uuid`, node-local IDs, synthetic BDF |

**Acceptance criteria:**
- [ ] `PackageModel` captures compute complexes, HBM stacks, SDMA engines, doorbells, fabric endpoints.
- [ ] Architectural identity is separated from runtime identity.
- [ ] Golden topology fixtures for 1×1, 1×8, and 2-node profiles.

**Tests:**
- `test_package_model_mi300x_topology` — MI300X model has expected CU/HBM/SDMA counts.
- `test_stable_ids_across_reboot` — `gpu_uuid` is deterministic across topology materializations.
- `test_golden_topology_1x8` — 1×8 fixture materializes 8 GPUs with correct inter-GPU links.

---

### Feature B2: Virtual GPU Device Model (Milestone B)

Create a self-contained virtual GPU resource model.

**Acceptance criteria:**
- [ ] `VirtualGpuDevice` owns immutable architectural properties and mutable runtime state.
- [ ] Memory subsystem: `HbmRegion`, `LdsRegion`, `ScratchRegion`, `GpuVaSpace`, `AllocationHandle`.
- [ ] Queue subsystem: `ComputeQueueState`, `DmaQueueState`, `SignalState`, doorbell notify.
- [ ] Deterministic GPU VA allocator with page-granularity accounting.
- [ ] All subsystem tests are simulator-native (no HSA/HIP dependency).

**Tests:**
- `test_gpu_alloc_map_unmap_free` — allocate HBM → map GPU VA → read/write → unmap → free lifecycle.
- `test_gpu_va_allocator_deterministic` — same allocation sequence yields same VA layout.
- `test_queue_create_submit_complete` — create queue → submit packet → poll completion.
- `test_signal_set_wait` — create signal → set → wait returns immediately.
- `test_signal_wait_timeout` — wait on unset signal times out correctly.
- `test_resource_lifecycle_teardown` — destroying a GPU frees all child resources.

---

### Feature B3: CDNA Functional ISA Core (Milestone C)

Implement ISA decode/dispatch/execute for the first CDNA workloads.

**Acceptance criteria:**
- [ ] Offline ISA decoder generated from GPUOpen machine-readable ISA data (gfx942/gfx950).
- [ ] Wavefront-granularity execution (not per-thread object granularity).
- [ ] Fast interpreter with IR caching and superinstructions as the default path.
- [ ] Code object loader for standard ROCm toolchain output.
- [ ] Correct memory semantics: global, LDS, register file, atomics, barriers.
- [ ] First workloads execute: no-op dispatch, write-one kernel, vector-add.

**Tests:**
- `test_decode_all_gfx942_opcodes` — every gfx942 opcode decodes without panic.
- `test_noop_dispatch` — empty kernel dispatches and completes.
- `test_write_one_kernel` — kernel writes `1` to output buffer; CPU verifies.
- `test_vector_add` — `C[i] = A[i] + B[i]` matches CPU reference for 1024 elements.
- `test_lds_barrier_sync` — workgroup barrier with LDS produces correct reduction result.
- `test_atomic_add_global` — global atomic add from multiple wavefronts produces expected sum.
- `test_dispatch_trace_ordering` — trace shows submit → execute → complete in order.
- `test_deterministic_replay` — same kernel produces identical trace across two runs.

---

### Feature B4: Dashboard MVP

Ship a functional web dashboard for local development.

**Acceptance criteria:**
- [ ] Simulator list with status indicators.
- [ ] Profile CRUD with GPU model selection.
- [ ] Session list with boot/running/stopped status badges.
- [ ] Session detail with logs and terminal view.
- [ ] Connects to daemon via WebSocket or polling.

**Tests:**
- `test_dashboard_loads_overview` — overview page renders without errors.
- `test_dashboard_create_profile` — creating a profile via UI reflects in daemon state.
- `test_dashboard_session_lifecycle` — boot/shutdown via UI transitions session state.

---

## Phase C — Multi-Node & Developer Workflow (Months 7–12)

### Feature C1: Minimal Clustered Execution (Milestone D)

Extend the simulator to multi-GPU and multi-node topologies.

**Acceptance criteria:**
- [ ] `FabricModel` with `FabricEndpoint` and `TransferRoute` for intra-node and inter-node links.
- [ ] Configurable latency, bandwidth, and contention per fabric link.
- [ ] Deterministic cross-node event ordering with replay support.
- [ ] Point-to-point GPU-to-GPU copy across nodes.
- [ ] At least one collective-style primitive (all-reduce or broadcast).

**Tests:**
- `test_intra_node_gpu_copy` — GPU0 → GPU1 within a node produces correct output.
- `test_inter_node_gpu_copy` — GPU0/node0 → GPU0/node1 transfer completes correctly.
- `test_fabric_latency_model` — transfer time scales with configured latency and size.
- `test_collective_allreduce_2_nodes` — all-reduce across 2 nodes × 2 GPUs matches reference.
- `test_cluster_trace_replay` — cluster-level trace captures and replays deterministically.
- `test_cluster_boot_8x8` — 8-node × 8-GPU topology boots successfully.

---

### Feature C2: `rocminfo` Bring-Up (Milestone E)

Prove the `libhsakmt` → `amdgpu_lite` path enumerates simulated GPUs.

**Acceptance criteria:**
- [ ] Mirage provides device identity and topology at the `amdgpu_lite` boundary.
- [ ] `libhsakmt` opens the Mirage device path and queries topology without `/dev/kfd`.
- [ ] `rocminfo` runs in a node container and reports one simulated CDNA GPU.
- [ ] Reported properties match profile: GPU name, memory size, CU count, agent properties.

**Tests:**
- `test_rocminfo_single_gpu` — `rocminfo` reports expected GPU name and memory.
- `test_rocminfo_8_gpu` — `rocminfo` on 8-GPU profile lists all 8 agents.
- `test_rocminfo_startup_trace` — startup trace is captured for replay/debug.
- `test_rocminfo_no_kfd_required` — `rocminfo` succeeds without `/dev/kfd` on host.

---

### Feature C3: HSA Queue & Memory Bring-Up (Milestone F)

Prove raw HSA operations work over the Mirage device contract.

**Acceptance criteria:**
- [ ] HSA memory allocation and free through `libhsakmt` → `amdgpu_lite`.
- [ ] GPU VA assignment and mapping.
- [ ] Queue creation, doorbell notification, and destruction.
- [ ] Signal allocation, wait, and completion.
- [ ] Queue write/read pointer handling and synchronization.

**Tests:**
- `test_hsa_alloc_map_free` — allocate → map → write → read → unmap → free succeeds.
- `test_hsa_queue_create_submit_sync` — create queue → submit minimal packet → signal completes.
- `test_hsa_signal_lifecycle` — alloc → wait (timeout) → set → wait (immediate) → destroy.
- `test_hsa_queue_reuse` — queue handles reuse after destroy/recreate.
- `test_hsa_error_on_unsupported` — unsupported operations return clean error, not crash.

---

### Feature C4: First HIP Kernel (Milestone G)

Run one HIP kernel end-to-end through the full stack.

**Acceptance criteria:**
- [ ] HIP runtime starts on top of adapted `libhsakmt`.
- [ ] `hipMalloc`, `hipMemcpy`, kernel launch, and `hipDeviceSynchronize` succeed.
- [ ] Kernel output matches CPU expectations.
- [ ] Trace shows queue submission, dispatch, and completion.
- [ ] Failure modes surface actionable diagnostics.

**Tests:**
- `test_hip_vector_add` — `hipMalloc` + `hipMemcpy` + vector-add kernel + verify on CPU.
- `test_hip_device_query` — `hipGetDeviceProperties` returns profile-consistent values.
- `test_hip_memcpy_h2d_d2h` — host-to-device and device-to-host copies round-trip correctly.
- `test_hip_dispatch_trace` — trace captures full dispatch lifecycle.
- `test_hip_error_reporting` — unsupported HIP call returns meaningful error string.

---

### Feature C5: PyTorch Wheel Smoke (Milestone H)

Prove the wheel-based runtime contract works for real frameworks.

**Acceptance criteria:**
- [ ] Published ROCm wheel set + Mirage `libhsakmt` overlay installs in a clean venv.
- [ ] `import torch; torch.cuda.is_available()` returns `True` (ROCm HIP backend).
- [ ] Tensor allocation on simulated GPU succeeds.
- [ ] One eager op and one small matmul execute and verify on CPU.
- [ ] No manual source builds required.

**Tests:**
- `test_pytorch_import` — `import torch` succeeds without errors.
- `test_pytorch_device_detection` — simulated GPU is detected as a CUDA/ROCm device.
- `test_pytorch_tensor_alloc` — `torch.zeros(100, device='cuda')` succeeds.
- `test_pytorch_matmul` — small matmul result matches CPU reference within tolerance.
- `test_pytorch_install_from_wheels` — fresh venv install completes without build steps.

---

### Feature C6: RCCL & Distributed Training Path

Enable multi-rank collective communication.

**Acceptance criteria:**
- [ ] RCCL topology discovery works against Mirage fabric model.
- [ ] AllReduce, AllGather, and Broadcast collectives complete across simulated ranks.
- [ ] Distributed training with 2+ ranks produces convergent loss.

**Tests:**
- `test_rccl_allreduce_4_ranks` — all-reduce across 4 ranks matches reference.
- `test_rccl_topology_detection` — RCCL detects expected ring/tree topology.
- `test_distributed_training_2_node` — 2-node data-parallel training runs for 10 steps.

---

### Feature C7: Record/Replay

Capture deterministic traces for debugging and CI.

**Acceptance criteria:**
- [ ] Trace captures: queue submissions, dispatches, memory ops, signals, collectives, checkpoints.
- [ ] Replay from saved trace reproduces identical execution state.
- [ ] Traces and snapshots are CI-friendly artifacts.
- [ ] `miragectl snapshot` and `miragectl replay` commands work.

**Tests:**
- `test_record_dispatch_trace` — dispatch trace is captured with expected event types.
- `test_replay_deterministic` — replay from trace produces identical output.
- `test_replay_from_checkpoint` — replay from mid-execution checkpoint succeeds.
- `test_trace_format_stable` — v1 trace format loads across Mirage versions.

---

### Feature C8: GUI Launcher

Desktop GUI for the iOS-Simulator-style developer experience.

**Acceptance criteria:**
- [ ] Profile selection and creation.
- [ ] Boot state visualization with progress indicators.
- [ ] Live log streaming.
- [ ] Debugger/profiler attach controls.
- [ ] Replay controls (load trace, step, scrub).

**Tests:**
- `test_gui_boot_session` — GUI can boot a session and display running state.
- `test_gui_log_streaming` — logs appear in real-time during session execution.
- `test_gui_replay_controls` — replay loads a trace and steps through events.

---

## Phase D — Performance & Hardening (Months 12+)

### Feature D1: Cycle-Accurate Timing Model

Pluggable calibrated timing for hot kernel classes.

**Acceptance criteria:**
- [ ] Timing model interface is separate from functional execution.
- [ ] GEMM, reduction, and collective primitives have calibrated models.
- [ ] Calibration harnesses run against MI300X hardware counters.
- [ ] Per-kernel error thresholds defined and enforced in CI.
- [ ] Timing is opt-in via profile `timing_mode` field.

**Tests:**
- `test_timing_gemm_within_bounds` — GEMM timing estimate within ±20% of hardware.
- `test_timing_allreduce_scaling` — collective timing scales correctly with node count.
- `test_timing_model_regression` — CI gates timing model changes on threshold checks.
- `test_functional_mode_unaffected` — enabling timing does not change functional output.

---

### Feature D2: Scale-Out Validation (8+ × 8)

Prove the simulator handles production-scale topologies.

**Acceptance criteria:**
- [ ] 8-node × 8-GPU topology boots and runs workloads.
- [ ] Memory and scheduling overhead stay within budget (< 64 GB host RAM for 64 simulated GPUs).
- [ ] Deterministic replay works at scale.

**Tests:**
- `test_scaleout_64_gpu_boot` — 8×8 topology boots within 60 seconds.
- `test_scaleout_allreduce_64_gpu` — all-reduce across 64 GPUs produces correct result.
- `test_scaleout_memory_budget` — host memory stays under 64 GB during 8×8 workload.
- `test_scaleout_replay_deterministic` — 8×8 trace replays identically.

---

### Feature D3: Performance Benchmarking Infrastructure

Continuous performance tracking for the simulator itself.

**Acceptance criteria:**
- [ ] Benchmark buckets: empty dispatch, ALU-heavy, memory-heavy, LDS/barrier, fabric transfer.
- [ ] Metrics history with regression detection.
- [ ] Benchmarks gated in CI with reproducible outputs.

**Tests:**
- `test_bench_empty_dispatch_throughput` — dispatches/sec above minimum threshold.
- `test_bench_vector_add_slowdown` — slowdown vs native stays within budget.
- `test_bench_regression_detection` — 10% regression from baseline triggers CI failure.

---

## Cross-Cutting Concerns

### Observability

| Concern | Requirement |
|---------|-------------|
| Structured logging | JSON logs with session correlation IDs |
| Metrics | Dispatch count, decode cache hit rate, queue depth, allocation pressure, scheduler utilization |
| Instruction coverage | Report of supported vs unsupported CDNA4 instructions |
| Trace export | Trace sink with configurable verbosity levels |

### CI & Packaging

| Layer | Coverage |
|-------|----------|
| Schema | Validation tests, golden examples, round-trip serde |
| Unit | Per-crate `cargo test` |
| Integration | Host-only daemon + CLI lifecycle |
| Conformance | ISA correctness, HSA/HIP compatibility |
| Replay regression | Trace determinism across versions |
| Performance | Benchmark buckets with threshold gates |

### Compatibility Matrix

| Dimension | Supported Values |
|-----------|-----------------|
| Host OS | Linux (x86_64) |
| GPU architecture | CDNA (gfx942, gfx950) |
| ROCm version | Pinned per release (e.g., 6.x wheels) |
| Container runtime | Docker, Podman |
| Profile schema | v1 (forward-compatible) |

### Security

- Daemon listens on Unix socket only (local auth first).
- Remote control behind an interface boundary (deferred).
- No credential passthrough into simulated nodes.
- Container isolation via standard OCI runtime.

---

## Milestone Summary

| Milestone | Description | Phase | Key Exit Criteria |
|-----------|-------------|-------|-------------------|
| **A** | Cluster topology boot | A | `miragectl` boots/shuts down multi-node topology repeatably |
| **B** | Virtual GPU device model | B | Simulator-native queue/memory smoke tests pass |
| **C** | First synthetic dispatch | B | Non-ROCm compute workload executes end-to-end |
| **D** | Minimal clustered execution | C | Cluster-level synthetic workload runs across simulated nodes |
| **E** | `rocminfo` bring-up | C | `rocminfo` reports simulated GPU through `libhsakmt` path |
| **F** | HSA queue & memory | C | HSA-level smoke test passes without HIP |
| **G** | First HIP kernel | C | One HIP kernel executes correctly end-to-end |
| **H** | PyTorch wheel smoke | C | Published PyTorch wheel runs minimal GPU workload |
