use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use async_trait::async_trait;
use tokio::sync::RwLock;

use mirage_container::{
    ContainerHandle, ContainerRuntime, ExecRequest, StartContainerRequest, StartedContainer,
};
use mirage_schema::common::{
    ExecArgs, GpuDef, GpuFamily, HealthStatus, ProfileDef, SessionDef, SetEnv, SimulatorMode, Time,
};
use mirage_schema::config::DaemonDef;
use mirage_schema::container::{BindMount, ContainerDef};
use mirage_schema::daemon::{
    MirageDaemonAttach, MirageDaemonBoot, MirageDaemonExec, MirageDaemonHealth,
    MirageDaemonOverview, MirageDaemonProfiles, MirageDaemonRegistration, MirageDaemonResult,
    MirageDaemonSessions, MirageDaemonShutdown, MirageDaemonSimulators, MirageDaemonTime,
};
use mirage_schema::simulator::SimulatorInfo;
use mirage_schema::socket::{
    AttachReply, AttachRequest, BootSessionReply, BootSessionRequest, CreateProfileReply,
    CreateProfileRequest, DashboardCreateSessionReply, DashboardCreateSessionRequest,
    DashboardDeleteSessionReply, DashboardDeleteSessionRequest, DeleteProfileReply,
    DeleteProfileRequest, ExecInSessionReply, ExecInSessionRequest, GetOverviewReply,
    GetOverviewRequest, GetSessionDetailReply, GetSessionDetailRequest, GetSimulatorReply,
    GetSimulatorRequest, HealthReply, HealthRequest, ListProfilesReply, ListProfilesRequest,
    ListSessionsReply, ListSessionsRequest, ListSimulatorsReply, ListSimulatorsRequest,
    RegisterSimReply, RegisterSimRequest, SessionSummary, ShutdownSessionReply,
    ShutdownSessionRequest, SimulatorSummary, TimeReply, TimeRequest,
};

pub struct InMemoryMirageDaemon {
    state: RwLock<State>,
    container_runtime: Option<Arc<dyn ContainerRuntime>>,
}

impl fmt::Debug for InMemoryMirageDaemon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InMemoryMirageDaemon")
            .field("state", &self.state)
            .field(
                "container_runtime",
                &self.container_runtime.as_ref().map(|_| "<runtime>"),
            )
            .finish()
    }
}

#[derive(Debug, Default)]
struct State {
    simulators: BTreeMap<String, SimulatorInfo>,
    profiles: BTreeMap<String, ProfileDef>,
    sessions: BTreeMap<String, SessionRecord>,
}

#[derive(Debug, Clone)]
struct SessionRecord {
    session: SessionDef,
    detail: GetSessionDetailReply,
    /// Container handles for all nodes (head node first, then workers).
    container_handles: Vec<ContainerHandle>,
    /// Docker network name for multi-node sessions.
    network_name: Option<String>,
    /// Path to the emulator socket inside the container.
    emulator_socket_container: Option<String>,
    /// Path to the interceptor library inside the container.
    interceptor_path_container: Option<String>,
}

/// Returns the set of simulators that are always available in the daemon.
fn builtin_simulators() -> BTreeMap<String, SimulatorInfo> {
    let sims = vec![SimulatorInfo {
        name: "rocjitsu".to_string(),
        version: "0.5.0".to_string(),
        description: Some("AMD CDNA functional simulator".to_string()),
        supported_gpus: vec![
            GpuDef {
                name: "MI300X".to_string(),
                arch: "gfx942".to_string(),
                family: GpuFamily::AmdCdna,
                description: Some("AMD Instinct MI300X".to_string()),
            },
            GpuDef {
                name: "MI325X".to_string(),
                arch: "gfx942".to_string(),
                family: GpuFamily::AmdCdna,
                description: Some("AMD Instinct MI325X".to_string()),
            },
            GpuDef {
                name: "MI350X".to_string(),
                arch: "gfx950".to_string(),
                family: GpuFamily::AmdCdna,
                description: Some("AMD Instinct MI350X".to_string()),
            },
        ],
        supports_custom_gpus: false,
        supported_modes: vec![SimulatorMode::Functional],
    }];
    sims.into_iter().map(|s| (s.name.clone(), s)).collect()
}

impl InMemoryMirageDaemon {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State {
                simulators: builtin_simulators(),
                ..State::default()
            }),
            container_runtime: None,
        }
    }

    pub fn with_container_runtime(runtime: Arc<dyn ContainerRuntime>) -> Self {
        Self {
            state: RwLock::new(State {
                simulators: builtin_simulators(),
                ..State::default()
            }),
            container_runtime: Some(runtime),
        }
    }

    pub fn from_config(config: DaemonDef) -> Self {
        let profiles = config
            .profiles
            .into_iter()
            .map(|profile| (profile.name.clone(), profile))
            .collect();
        Self {
            state: RwLock::new(State {
                simulators: builtin_simulators(),
                profiles,
                ..State::default()
            }),
            container_runtime: None,
        }
    }

    pub fn from_config_with_runtime(config: DaemonDef, runtime: Arc<dyn ContainerRuntime>) -> Self {
        let profiles = config
            .profiles
            .into_iter()
            .map(|profile| (profile.name.clone(), profile))
            .collect();
        Self {
            state: RwLock::new(State {
                simulators: builtin_simulators(),
                profiles,
                ..State::default()
            }),
            container_runtime: Some(runtime),
        }
    }

    fn active_session_count(state: &State, simulator: &str) -> u32 {
        state
            .sessions
            .values()
            .filter(|session| session.detail.simulator.as_deref() == Some(simulator))
            .count() as u32
    }

    fn simulator_summary(state: &State, info: &SimulatorInfo) -> SimulatorSummary {
        SimulatorSummary {
            name: Some(info.name.clone()),
            version: Some(info.version.clone()),
            description: info.description.clone(),
            supported_gpus: info.supported_gpus.clone(),
            supports_custom_gpus: info.supports_custom_gpus,
            supported_modes: info.supported_modes.clone(),
            active_session_count: Self::active_session_count(state, &info.name),
        }
    }

    fn simulator_supports_mode(info: &SimulatorInfo, mode: SimulatorMode) -> bool {
        info.supported_modes.is_empty() || info.supported_modes.contains(&mode)
    }

    fn simulator_supports_gpu(info: &SimulatorInfo, gpu_name: &str) -> bool {
        info.supported_gpus.iter().any(|gpu| gpu.name == gpu_name)
    }
}

#[async_trait]
impl MirageDaemonHealth for InMemoryMirageDaemon {
    async fn health(&self, request: HealthRequest) -> MirageDaemonResult<HealthReply> {
        let state = self.state.read().await;
        let status = if let Some(session_id) = request.session_id {
            state
                .sessions
                .get(&session_id)
                .map(|session| session.detail.health)
                .ok_or_else(|| {
                    mirage_schema::daemon::MirageDaemonError::Remote(format!(
                        "session '{}' does not exist",
                        session_id
                    ))
                })?
        } else {
            HealthStatus::Healthy
        };

        Ok(HealthReply {
            healthy: matches!(status, HealthStatus::Healthy),
            status,
        })
    }
}

#[async_trait]
impl MirageDaemonTime for InMemoryMirageDaemon {
    async fn time(&self, request: TimeRequest) -> MirageDaemonResult<TimeReply> {
        if let Some(session_id) = request.session_id {
            let state = self.state.read().await;
            if !state.sessions.contains_key(&session_id) {
                return Err(mirage_schema::daemon::MirageDaemonError::Remote(format!(
                    "session '{}' does not exist",
                    session_id
                )));
            }
        }

        Ok(TimeReply {
            time: Time::default(),
        })
    }
}

#[async_trait]
impl MirageDaemonAttach for InMemoryMirageDaemon {
    async fn attach(&self, _request: AttachRequest) -> MirageDaemonResult<Vec<AttachReply>> {
        Err(mirage_schema::daemon::MirageDaemonError::Remote(
            "attach is not implemented by the in-memory daemon".to_string(),
        ))
    }
}

#[async_trait]
impl MirageDaemonRegistration for InMemoryMirageDaemon {
    async fn register_sim(
        &self,
        request: RegisterSimRequest,
    ) -> MirageDaemonResult<RegisterSimReply> {
        if request.info.name.trim().is_empty() {
            return Ok(RegisterSimReply {
                ok: false,
                error: Some("simulator name must not be empty".to_string()),
            });
        }

        let mut state = self.state.write().await;
        state
            .simulators
            .insert(request.info.name.clone(), request.info);
        Ok(RegisterSimReply {
            ok: true,
            error: None,
        })
    }
}

#[async_trait]
impl MirageDaemonOverview for InMemoryMirageDaemon {
    async fn get_overview(
        &self,
        _request: GetOverviewRequest,
    ) -> MirageDaemonResult<GetOverviewReply> {
        let state = self.state.read().await;
        Ok(GetOverviewReply {
            simulator_count: state.simulators.len() as u32,
            profile_count: state.profiles.len() as u32,
            session_count: state.sessions.len() as u32,
        })
    }
}

#[async_trait]
impl MirageDaemonSimulators for InMemoryMirageDaemon {
    async fn list_simulators(
        &self,
        _request: ListSimulatorsRequest,
    ) -> MirageDaemonResult<ListSimulatorsReply> {
        let state = self.state.read().await;
        let simulators = state
            .simulators
            .values()
            .map(|info| Self::simulator_summary(&state, info))
            .collect();
        Ok(ListSimulatorsReply { simulators })
    }

    async fn get_simulator(
        &self,
        request: GetSimulatorRequest,
    ) -> MirageDaemonResult<GetSimulatorReply> {
        let state = self.state.read().await;
        let simulator = state
            .simulators
            .get(&request.name)
            .map(|info| Self::simulator_summary(&state, info));
        Ok(GetSimulatorReply { simulator })
    }
}

#[async_trait]
impl MirageDaemonProfiles for InMemoryMirageDaemon {
    async fn list_profiles(
        &self,
        request: ListProfilesRequest,
    ) -> MirageDaemonResult<ListProfilesReply> {
        let state = self.state.read().await;
        let profiles = state
            .profiles
            .values()
            .filter(|profile| {
                request
                    .simulator_filter
                    .as_ref()
                    .is_none_or(|filter| profile.simulator == *filter)
            })
            .cloned()
            .collect();
        Ok(ListProfilesReply { profiles })
    }

    async fn create_profile(
        &self,
        request: CreateProfileRequest,
    ) -> MirageDaemonResult<CreateProfileReply> {
        let profile = request.profile;
        if profile.name.trim().is_empty() {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some("profile name must not be empty".to_string()),
            });
        }
        if profile.num_gpus == 0 {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some("gpus per node must be greater than zero".to_string()),
            });
        }
        if profile.num_nodes == 0 {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some("node count must be greater than zero".to_string()),
            });
        }

        let mut state = self.state.write().await;
        if state.profiles.contains_key(&profile.name) {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some(format!("profile '{}' already exists", profile.name)),
            });
        }

        let Some(simulator) = state.simulators.get(&profile.simulator) else {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some(format!(
                    "simulator '{}' is not registered",
                    profile.simulator
                )),
            });
        };

        if !Self::simulator_supports_gpu(simulator, &profile.gpu) {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some(format!(
                    "gpu '{}' is not supported by simulator '{}'",
                    profile.gpu, profile.simulator
                )),
            });
        }

        if !Self::simulator_supports_mode(simulator, profile.mode) {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some(format!(
                    "mode '{:?}' is not supported by simulator '{}'",
                    profile.mode, profile.simulator
                )),
            });
        }

        state.profiles.insert(profile.name.clone(), profile);
        Ok(CreateProfileReply {
            ok: true,
            error: None,
        })
    }

    async fn delete_profile(
        &self,
        request: DeleteProfileRequest,
    ) -> MirageDaemonResult<DeleteProfileReply> {
        let mut state = self.state.write().await;
        if !state.profiles.contains_key(&request.name) {
            return Ok(DeleteProfileReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", request.name)),
            });
        }

        if state
            .sessions
            .values()
            .any(|session| session.session.profile == request.name)
        {
            return Ok(DeleteProfileReply {
                ok: false,
                error: Some(format!(
                    "profile '{}' is still referenced by an active session",
                    request.name
                )),
            });
        }

        state.profiles.remove(&request.name);
        Ok(DeleteProfileReply {
            ok: true,
            error: None,
        })
    }
}

#[async_trait]
impl MirageDaemonSessions for InMemoryMirageDaemon {
    async fn list_sessions(
        &self,
        request: ListSessionsRequest,
    ) -> MirageDaemonResult<ListSessionsReply> {
        let state = self.state.read().await;
        let sessions = state
            .sessions
            .values()
            .filter(|session| {
                request
                    .profile_filter
                    .as_ref()
                    .is_none_or(|filter| session.session.profile == *filter)
            })
            .map(|session| SessionSummary {
                name: Some(session.session.name.clone()),
                profile: Some(session.session.profile.clone()),
                simulator: session.detail.simulator.clone(),
                image: session.detail.image.clone(),
                health_status: session.detail.health,
            })
            .collect();
        Ok(ListSessionsReply { sessions })
    }

    async fn create_session(
        &self,
        request: DashboardCreateSessionRequest,
    ) -> MirageDaemonResult<DashboardCreateSessionReply> {
        let session = request.session;
        if session.name.trim().is_empty() {
            return Ok(DashboardCreateSessionReply {
                ok: false,
                error: Some("session name must not be empty".to_string()),
            });
        }

        let mut state = self.state.write().await;
        if state.sessions.contains_key(&session.name) {
            return Ok(DashboardCreateSessionReply {
                ok: false,
                error: Some(format!("session '{}' already exists", session.name)),
            });
        }

        let Some(profile) = state.profiles.get(&session.profile).cloned() else {
            return Ok(DashboardCreateSessionReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", session.profile)),
            });
        };

        if !state.simulators.contains_key(&profile.simulator) {
            return Ok(DashboardCreateSessionReply {
                ok: false,
                error: Some(format!(
                    "simulator '{}' is not registered",
                    profile.simulator
                )),
            });
        }

        let detail = GetSessionDetailReply {
            name: Some(session.name.clone()),
            profile: Some(profile.clone()),
            simulator: Some(profile.simulator.clone()),
            image: Some(session.image.clone()),
            health: HealthStatus::Healthy,
            uptime: None,
            error_message: None,
            ticks: 0,
            ipc: 0.0,
            simulation_speed: 0.0,
            active_contexts: 0,
        };

        state.sessions.insert(
            session.name.clone(),
            SessionRecord {
                session,
                detail,
                container_handles: vec![],
                emulator_socket_container: None,
                interceptor_path_container: None,
                network_name: None,
            },
        );

        Ok(DashboardCreateSessionReply {
            ok: true,
            error: None,
        })
    }

    async fn delete_session(
        &self,
        request: DashboardDeleteSessionRequest,
    ) -> MirageDaemonResult<DashboardDeleteSessionReply> {
        let mut state = self.state.write().await;
        if state.sessions.remove(&request.name).is_none() {
            return Ok(DashboardDeleteSessionReply {
                ok: false,
                error: Some(format!("session '{}' does not exist", request.name)),
            });
        }

        Ok(DashboardDeleteSessionReply {
            ok: true,
            error: None,
        })
    }

    async fn get_session_detail(
        &self,
        request: GetSessionDetailRequest,
    ) -> MirageDaemonResult<GetSessionDetailReply> {
        let state = self.state.read().await;
        let detail = state
            .sessions
            .get(&request.name)
            .map(|session| session.detail.clone())
            .ok_or_else(|| {
                mirage_schema::daemon::MirageDaemonError::Remote(format!(
                    "session '{}' does not exist",
                    request.name
                ))
            })?;
        Ok(detail)
    }
}

/// Default port used for head-node communication (matches NCCL/torch defaults).
const MIRAGE_HEAD_PORT: u16 = 29500;

#[async_trait]
impl MirageDaemonBoot for InMemoryMirageDaemon {
    async fn boot_session(
        &self,
        request: BootSessionRequest,
    ) -> MirageDaemonResult<BootSessionReply> {
        let session = request.session;
        if session.name.trim().is_empty() {
            return Ok(BootSessionReply {
                ok: false,
                error: Some("session name must not be empty".to_string()),
                container_id: None,
                container_ids: vec![],
            });
        }

        let mut state = self.state.write().await;
        if state.sessions.contains_key(&session.name) {
            return Ok(BootSessionReply {
                ok: false,
                error: Some(format!("session '{}' already exists", session.name)),
                container_id: None,
                container_ids: vec![],
            });
        }

        let Some(profile) = state.profiles.get(&session.profile).cloned() else {
            return Ok(BootSessionReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", session.profile)),
                container_id: None,
                container_ids: vec![],
            });
        };

        if !state.simulators.contains_key(&profile.simulator) {
            return Ok(BootSessionReply {
                ok: false,
                error: Some(format!(
                    "simulator '{}' is not registered",
                    profile.simulator
                )),
                container_id: None,
                container_ids: vec![],
            });
        }

        let Some(runtime) = &self.container_runtime else {
            return Ok(BootSessionReply {
                ok: false,
                error: Some("no container runtime configured".to_string()),
                container_id: None,
                container_ids: vec![],
            });
        };

        // --- Emulator + interceptor setup ---
        let interceptor_so = find_interceptor_so();
        let mut base_mounts = Vec::new();
        let mut base_env = vec![SetEnv {
            key: "MIRAGE_SESSION".to_string(),
            value: session.name.clone(),
        }];
        let devices = Vec::new();
        let mut emu_socket_container: Option<String> = None;
        let mut interceptor_container: Option<String> = None;

        if let Some(ref so_path) = interceptor_so {
            if mirage_real::RealEmulator::hardware_available() {
                if let Ok(Some(real)) = mirage_real::RealEmulator::detect() {
                    // Fetch the topology from the emulator before moving it
                    // into the server (the server takes ownership).
                    use mirage_schema::topology::ProvideTopology;
                    let topology = real.get_topology().ok();

                    let socket_path = unique_emulator_socket(&session.name);
                    let server = mirage_remote::EmulatorServer::new(socket_path.clone(), real);
                    let listener = match server.bind() {
                        Ok(l) => l,
                        Err(e) => {
                            return Ok(BootSessionReply {
                                ok: false,
                                error: Some(format!("failed to bind emulator socket: {e}")),
                                container_id: None,
                                container_ids: vec![],
                            });
                        }
                    };
                    thread::spawn(move || {
                        let _ = server.serve_on(listener);
                    });

                    let container_so = "/opt/mirage/libmirage_interceptor.so".to_string();
                    let container_sock = "/opt/mirage/emulator.sock".to_string();

                    base_mounts.push(BindMount {
                        host_path: so_path.to_string_lossy().to_string(),
                        container_path: container_so.clone(),
                        readonly: true,
                    });
                    base_mounts.push(BindMount {
                        host_path: socket_path.to_string_lossy().to_string(),
                        container_path: container_sock.clone(),
                        readonly: false,
                    });

                    // Create synthetic sysfs topology and device stubs so
                    // HIP discovers the simulated GPUs without real device
                    // nodes.  Use the Topology fetched from the emulator.
                    if let Some(ref topo) = topology {
                        if let Ok(topo_dir) = create_synthetic_topology(&session.name, topo) {
                            base_mounts.push(BindMount {
                                host_path: topo_dir
                                    .join("sys/class/kfd")
                                    .to_string_lossy()
                                    .to_string(),
                                container_path: "/sys/class/kfd".to_string(),
                                readonly: true,
                            });
                            // hsakmt reads topology from /sys/devices/virtual/kfd/kfd/topology
                            base_mounts.push(BindMount {
                                host_path: topo_dir
                                    .join("sys/class/kfd/kfd/topology")
                                    .to_string_lossy()
                                    .to_string(),
                                container_path: "/sys/devices/virtual/kfd/kfd/topology".to_string(),
                                readonly: true,
                            });
                            base_mounts.push(BindMount {
                                host_path: topo_dir.join("dev/dri").to_string_lossy().to_string(),
                                container_path: "/dev/dri".to_string(),
                                readonly: true,
                            });
                            base_mounts.push(BindMount {
                                host_path: topo_dir.join("dev/kfd").to_string_lossy().to_string(),
                                container_path: "/dev/kfd".to_string(),
                                readonly: true,
                            });
                        }
                    }

                    base_env.push(SetEnv {
                        key: "LD_PRELOAD".to_string(),
                        value: container_so.clone(),
                    });
                    base_env.push(SetEnv {
                        key: "MIRAGE_INTERCEPTOR_SOCKET".to_string(),
                        value: container_sock.clone(),
                    });

                    emu_socket_container = Some(container_sock);
                    interceptor_container = Some(container_so);
                }
            }
        }

        // Pull the image (ignore errors for locally available images).
        let _ = runtime.pull_image(&session.image, None).await;

        let num_nodes = profile.num_nodes.max(1);
        let multi_node = num_nodes > 1;
        let head_container_name = if multi_node {
            format!("mirage-{}-node0", session.name)
        } else {
            format!("mirage-{}", session.name)
        };

        // Create a Docker network for multi-node sessions.
        let network_name = if multi_node {
            let name = format!("mirage-{}", session.name);
            if let Err(err) = runtime.create_network(&name, None).await {
                return Ok(BootSessionReply {
                    ok: false,
                    error: Some(format!("failed to create network: {err}")),
                    container_id: None,
                    container_ids: vec![],
                });
            }
            Some(name)
        } else {
            None
        };

        let mut container_handles: Vec<ContainerHandle> = Vec::with_capacity(num_nodes as usize);
        let mut container_ids: Vec<String> = Vec::with_capacity(num_nodes as usize);

        for node_index in 0..num_nodes {
            let container_name = if multi_node {
                format!("mirage-{}-node{}", session.name, node_index)
            } else {
                format!("mirage-{}", session.name)
            };

            let mut node_env = base_env.clone();
            if multi_node {
                node_env.push(SetEnv {
                    key: "MIRAGE_NUM_NODES".to_string(),
                    value: num_nodes.to_string(),
                });
                node_env.push(SetEnv {
                    key: "MIRAGE_NODE_RANK".to_string(),
                    value: node_index.to_string(),
                });
                node_env.push(SetEnv {
                    key: "MIRAGE_HEAD_ADDR".to_string(),
                    value: head_container_name.clone(),
                });
                node_env.push(SetEnv {
                    key: "MIRAGE_HEAD_PORT".to_string(),
                    value: MIRAGE_HEAD_PORT.to_string(),
                });
            }

            let container_def = ContainerDef {
                image: session.image.clone(),
                mounts: base_mounts.clone(),
                injected_files: vec![],
                entrypoint: ExecArgs {
                    command: "sleep".to_string(),
                    args: vec!["infinity".to_string()],
                    env: node_env,
                },
                working_dir: None,
                ports: vec![],
                devices: devices.clone(),
                privileged: false,
                resource_limits_json: None,
                network: network_name.clone(),
            };

            let started = match runtime
                .start_container(
                    StartContainerRequest {
                        name: container_name,
                        container: container_def,
                    },
                    None,
                )
                .await
            {
                Ok(s) => s,
                Err(err) => {
                    // Clean up any containers we already started.
                    for handle in &container_handles {
                        let _ = runtime.stop_container(handle, 5, None).await;
                        let _ = runtime.remove_container(handle, true, None).await;
                    }
                    if let Some(ref net) = network_name {
                        let _ = runtime.remove_network(net, None).await;
                    }
                    return Ok(BootSessionReply {
                        ok: false,
                        error: Some(format!("failed to start container: {err}")),
                        container_id: None,
                        container_ids: vec![],
                    });
                }
            };

            container_ids.push(started.inspection.handle.id.clone());
            container_handles.push(started.inspection.handle);
        }

        let head_container_id = container_ids.first().cloned();

        let detail = GetSessionDetailReply {
            name: Some(session.name.clone()),
            profile: Some(profile.clone()),
            simulator: Some(profile.simulator.clone()),
            image: Some(session.image.clone()),
            health: HealthStatus::Healthy,
            uptime: None,
            error_message: None,
            ticks: 0,
            ipc: 0.0,
            simulation_speed: 0.0,
            active_contexts: 0,
        };

        state.sessions.insert(
            session.name.clone(),
            SessionRecord {
                session,
                detail,
                container_handles,
                network_name,
                emulator_socket_container: emu_socket_container,
                interceptor_path_container: interceptor_container,
            },
        );

        Ok(BootSessionReply {
            ok: true,
            error: None,
            container_id: head_container_id,
            container_ids,
        })
    }
}

#[async_trait]
impl MirageDaemonExec for InMemoryMirageDaemon {
    async fn exec_in_session(
        &self,
        request: ExecInSessionRequest,
    ) -> MirageDaemonResult<ExecInSessionReply> {
        let state = self.state.read().await;
        let Some(record) = state.sessions.get(&request.session_name) else {
            return Err(mirage_schema::daemon::MirageDaemonError::Remote(format!(
                "session '{}' does not exist",
                request.session_name
            )));
        };

        let Some(handle) = record.container_handles.first() else {
            return Err(mirage_schema::daemon::MirageDaemonError::Remote(format!(
                "session '{}' has no running container (was it booted?)",
                request.session_name
            )));
        };

        let Some(runtime) = &self.container_runtime else {
            return Err(mirage_schema::daemon::MirageDaemonError::Remote(
                "no container runtime configured".to_string(),
            ));
        };

        let exec_request = ExecRequest {
            container: handle.clone(),
            exec: {
                let mut exec = request.exec;
                // Force GPU access through the interceptor.
                if let Some(ref so_path) = record.interceptor_path_container {
                    exec.env.push(SetEnv {
                        key: "LD_PRELOAD".to_string(),
                        value: so_path.clone(),
                    });
                }
                if let Some(ref sock_path) = record.emulator_socket_container {
                    exec.env.push(SetEnv {
                        key: "MIRAGE_INTERCEPTOR_SOCKET".to_string(),
                        value: sock_path.clone(),
                    });
                }
                exec
            },
            working_dir: None,
        };

        match runtime.exec(exec_request, None).await {
            Ok(result) => Ok(ExecInSessionReply {
                exit_code: result.exit_code,
                stdout: result.stdout,
                stderr: result.stderr,
            }),
            Err(err) => Err(mirage_schema::daemon::MirageDaemonError::Remote(format!(
                "exec failed: {err}"
            ))),
        }
    }
}

#[async_trait]
impl MirageDaemonShutdown for InMemoryMirageDaemon {
    async fn shutdown_session(
        &self,
        request: ShutdownSessionRequest,
    ) -> MirageDaemonResult<ShutdownSessionReply> {
        let mut state = self.state.write().await;
        let Some(record) = state.sessions.remove(&request.name) else {
            return Ok(ShutdownSessionReply {
                ok: false,
                error: Some(format!("session '{}' does not exist", request.name)),
            });
        };

        if let Some(runtime) = &self.container_runtime {
            // Stop and remove all node containers.
            for handle in &record.container_handles {
                let _ = runtime.stop_container(handle, 10, None).await;
                let _ = runtime.remove_container(handle, true, None).await;
            }
            // Remove the session network if one was created.
            if let Some(ref net) = record.network_name {
                let _ = runtime.remove_network(net, None).await;
            }
        }

        Ok(ShutdownSessionReply {
            ok: true,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_profile_only_for_registered_simulator() {
        let daemon = InMemoryMirageDaemon::new();

        // Unknown simulator should fail.
        let reply = daemon
            .create_profile(CreateProfileRequest {
                profile: ProfileDef {
                    name: "custom".to_string(),
                    simulator: "nonexistent".to_string(),
                    mode: SimulatorMode::Functional,
                    gpu: "MI300X".to_string(),
                    num_gpus: 1,
                    num_nodes: 1,
                },
            })
            .await
            .unwrap();
        assert!(!reply.ok);

        // Built-in rocjitsu should succeed.
        let reply = daemon
            .create_profile(CreateProfileRequest {
                profile: ProfileDef {
                    name: "mi300x".to_string(),
                    simulator: "rocjitsu".to_string(),
                    mode: SimulatorMode::Functional,
                    gpu: "MI300X".to_string(),
                    num_gpus: 1,
                    num_nodes: 1,
                },
            })
            .await
            .unwrap();
        assert!(reply.ok);
    }

    #[tokio::test]
    async fn creates_and_lists_sessions() {
        let daemon = InMemoryMirageDaemon::new();
        daemon
            .create_profile(CreateProfileRequest {
                profile: ProfileDef {
                    name: "mi300x".to_string(),
                    simulator: "rocjitsu".to_string(),
                    mode: SimulatorMode::Functional,
                    gpu: "MI300X".to_string(),
                    num_gpus: 1,
                    num_nodes: 1,
                },
            })
            .await
            .unwrap();

        let reply = daemon
            .create_session(DashboardCreateSessionRequest {
                session: SessionDef {
                    name: "session-a".to_string(),
                    profile: "mi300x".to_string(),
                    image: "ghcr.io/example/image:latest".to_string(),
                },
            })
            .await
            .unwrap();
        assert!(reply.ok);

        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 1);
        assert_eq!(sessions.sessions[0].name.as_deref(), Some("session-a"));
    }

    async fn daemon_with_mock_runtime() -> (
        InMemoryMirageDaemon,
        Arc<mirage_container::MockContainerRuntime>,
    ) {
        let mock = Arc::new(mirage_container::MockContainerRuntime::default());
        let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

        daemon
            .create_profile(CreateProfileRequest {
                profile: ProfileDef {
                    name: "mi300x".to_string(),
                    simulator: "rocjitsu".to_string(),
                    mode: SimulatorMode::Functional,
                    gpu: "MI300X".to_string(),
                    num_gpus: 1,
                    num_nodes: 1,
                },
            })
            .await
            .unwrap();

        (daemon, mock)
    }

    #[tokio::test]
    async fn boot_creates_session_and_starts_container() {
        let (daemon, mock) = daemon_with_mock_runtime().await;

        let reply = daemon
            .boot_session(BootSessionRequest {
                session: SessionDef {
                    name: "vllm-test".to_string(),
                    profile: "mi300x".to_string(),
                    image: "ghcr.io/rocm/vllm:latest".to_string(),
                },
            })
            .await
            .unwrap();

        assert!(reply.ok, "boot should succeed: {:?}", reply.error);
        assert!(reply.container_id.is_some());

        // The session should be listed.
        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 1);
        assert_eq!(sessions.sessions[0].name.as_deref(), Some("vllm-test"));

        // The container should have been started.
        let starts = mock.start_requests().await;
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].name, "mirage-vllm-test");
        assert_eq!(starts[0].container.image, "ghcr.io/rocm/vllm:latest");
    }

    #[tokio::test]
    async fn boot_rejects_missing_profile() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;

        let reply = daemon
            .boot_session(BootSessionRequest {
                session: SessionDef {
                    name: "test".to_string(),
                    profile: "nonexistent".to_string(),
                    image: "img:latest".to_string(),
                },
            })
            .await
            .unwrap();

        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("does not exist"));
    }

    #[tokio::test]
    async fn exec_runs_command_in_booted_session() {
        let (daemon, mock) = daemon_with_mock_runtime().await;

        // Boot first.
        let boot = daemon
            .boot_session(BootSessionRequest {
                session: SessionDef {
                    name: "exec-test".to_string(),
                    profile: "mi300x".to_string(),
                    image: "img:latest".to_string(),
                },
            })
            .await
            .unwrap();
        assert!(boot.ok);

        // Queue a mock exec result.
        let starts = mock.start_requests().await;
        let handle = mirage_container::ContainerHandle {
            id: boot.container_id.clone().unwrap(),
            name: starts[0].name.clone(),
        };
        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: b"GPU available\n".to_vec(),
                stderr: vec![],
            },
        )
        .await;

        // Exec.
        let reply = daemon
            .exec_in_session(ExecInSessionRequest {
                session_name: "exec-test".to_string(),
                exec: ExecArgs {
                    command: "python".to_string(),
                    args: vec![
                        "-c".to_string(),
                        "import torch; print('GPU available')".to_string(),
                    ],
                    env: vec![],
                },
            })
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&reply.stdout), "GPU available\n");
    }

    #[tokio::test]
    async fn exec_fails_for_non_booted_session() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;

        let result = daemon
            .exec_in_session(ExecInSessionRequest {
                session_name: "nonexistent".to_string(),
                exec: ExecArgs {
                    command: "echo".to_string(),
                    args: vec!["hello".to_string()],
                    env: vec![],
                },
            })
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_stops_and_removes_session() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;

        // Boot.
        let boot = daemon
            .boot_session(BootSessionRequest {
                session: SessionDef {
                    name: "shutdown-test".to_string(),
                    profile: "mi300x".to_string(),
                    image: "img:latest".to_string(),
                },
            })
            .await
            .unwrap();
        assert!(boot.ok);

        // Session should exist.
        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 1);

        // Shutdown.
        let reply = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "shutdown-test".to_string(),
            })
            .await
            .unwrap();
        assert!(reply.ok);

        // Session should be gone.
        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 0);
    }

    #[tokio::test]
    async fn shutdown_nonexistent_session_fails() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;

        let reply = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "nonexistent".to_string(),
            })
            .await
            .unwrap();
        assert!(!reply.ok);
    }

    #[tokio::test]
    async fn boot_exec_shutdown_e2e() {
        let (daemon, mock) = daemon_with_mock_runtime().await;

        // 1. Boot a vLLM session.
        let boot = daemon
            .boot_session(BootSessionRequest {
                session: SessionDef {
                    name: "vllm-e2e".to_string(),
                    profile: "mi300x".to_string(),
                    image: "ghcr.io/rocm/vllm:latest".to_string(),
                },
            })
            .await
            .unwrap();
        assert!(boot.ok);
        let container_id = boot.container_id.unwrap();

        // 2. Exec a health check.
        let starts = mock.start_requests().await;
        let handle = mirage_container::ContainerHandle {
            id: container_id.clone(),
            name: starts[0].name.clone(),
        };
        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: b"vLLM is running\n".to_vec(),
                stderr: vec![],
            },
        )
        .await;

        let exec_reply = daemon
            .exec_in_session(ExecInSessionRequest {
                session_name: "vllm-e2e".to_string(),
                exec: ExecArgs {
                    command: "python".to_string(),
                    args: vec!["-c".to_string(), "print('vLLM is running')".to_string()],
                    env: vec![],
                },
            })
            .await
            .unwrap();
        assert_eq!(exec_reply.exit_code, 0);
        assert_eq!(
            String::from_utf8_lossy(&exec_reply.stdout),
            "vLLM is running\n"
        );

        // 3. Shutdown.
        let shutdown = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "vllm-e2e".to_string(),
            })
            .await
            .unwrap();
        assert!(shutdown.ok);

        // Verify cleanup.
        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert!(sessions.sessions.is_empty());
    }

    async fn daemon_with_multinode_profile() -> (
        InMemoryMirageDaemon,
        Arc<mirage_container::MockContainerRuntime>,
    ) {
        let mock = Arc::new(mirage_container::MockContainerRuntime::default());
        let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

        daemon
            .create_profile(CreateProfileRequest {
                profile: ProfileDef {
                    name: "mi300x-2node".to_string(),
                    simulator: "rocjitsu".to_string(),
                    mode: SimulatorMode::Functional,
                    gpu: "MI300X".to_string(),
                    num_gpus: 8,
                    num_nodes: 2,
                },
            })
            .await
            .unwrap();

        (daemon, mock)
    }

    #[tokio::test]
    async fn boot_multinode_starts_multiple_containers_with_network() {
        let (daemon, mock) = daemon_with_multinode_profile().await;

        let reply = daemon
            .boot_session(BootSessionRequest {
                session: SessionDef {
                    name: "multi-test".to_string(),
                    profile: "mi300x-2node".to_string(),
                    image: "ghcr.io/rocm/vllm:latest".to_string(),
                },
            })
            .await
            .unwrap();

        assert!(reply.ok, "boot should succeed: {:?}", reply.error);
        assert_eq!(reply.container_ids.len(), 2, "should have 2 containers");
        assert_eq!(
            reply.container_id,
            Some(reply.container_ids[0].clone()),
            "container_id should be head node"
        );

        // Verify two start_container calls were made.
        let starts = mock.start_requests().await;
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0].name, "mirage-multi-test-node0");
        assert_eq!(starts[1].name, "mirage-multi-test-node1");

        // Both containers should be on the same network.
        assert_eq!(
            starts[0].container.network.as_deref(),
            Some("mirage-multi-test")
        );
        assert_eq!(
            starts[1].container.network.as_deref(),
            Some("mirage-multi-test")
        );

        // A Docker network should have been created.
        let networks = mock.networks().await;
        assert_eq!(networks, vec!["mirage-multi-test"]);

        // Verify env vars on head node (node0).
        let head_env = &starts[0].container.entrypoint.env;
        let find_env = |envs: &[SetEnv], key: &str| -> Option<String> {
            envs.iter().find(|e| e.key == key).map(|e| e.value.clone())
        };
        assert_eq!(
            find_env(head_env, "MIRAGE_NUM_NODES"),
            Some("2".to_string())
        );
        assert_eq!(
            find_env(head_env, "MIRAGE_NODE_RANK"),
            Some("0".to_string())
        );
        assert_eq!(
            find_env(head_env, "MIRAGE_HEAD_ADDR"),
            Some("mirage-multi-test-node0".to_string())
        );
        assert_eq!(
            find_env(head_env, "MIRAGE_HEAD_PORT"),
            Some("29500".to_string())
        );

        // Verify env vars on worker node (node1).
        let worker_env = &starts[1].container.entrypoint.env;
        assert_eq!(
            find_env(worker_env, "MIRAGE_NUM_NODES"),
            Some("2".to_string())
        );
        assert_eq!(
            find_env(worker_env, "MIRAGE_NODE_RANK"),
            Some("1".to_string())
        );
        assert_eq!(
            find_env(worker_env, "MIRAGE_HEAD_ADDR"),
            Some("mirage-multi-test-node0".to_string())
        );
        assert_eq!(
            find_env(worker_env, "MIRAGE_HEAD_PORT"),
            Some("29500".to_string())
        );
    }

    #[tokio::test]
    async fn shutdown_multinode_cleans_up_all_containers_and_network() {
        let (daemon, mock) = daemon_with_multinode_profile().await;

        let boot = daemon
            .boot_session(BootSessionRequest {
                session: SessionDef {
                    name: "multi-shutdown".to_string(),
                    profile: "mi300x-2node".to_string(),
                    image: "img:latest".to_string(),
                },
            })
            .await
            .unwrap();
        assert!(boot.ok);

        let shutdown = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "multi-shutdown".to_string(),
            })
            .await
            .unwrap();
        assert!(shutdown.ok);

        // Session should be gone.
        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert!(sessions.sessions.is_empty());

        // Network should have been removed.
        let networks = mock.networks().await;
        assert!(networks.is_empty(), "network should be removed on shutdown");
    }
}

/// Find the interceptor shared library, trying the binary's directory and
/// common build output paths.
fn find_interceptor_so() -> Option<PathBuf> {
    let candidates = [
        // Next to the running binary.
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("libmirage_interceptor.so"))),
        // Fallback: cargo target/debug.
        Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/debug/libmirage_interceptor.so"
        ))),
    ];
    candidates.into_iter().flatten().find(|p| p.exists())
}

/// Return a unique socket path for an emulator instance.
fn unique_emulator_socket(session_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("mirage");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("emu-{session_name}.sock"))
}

/// Recursively copy the contents of a directory.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ty.is_file() {
            // sysfs files may fail to read; ignore errors.
            if let Ok(data) = std::fs::read(&src_path) {
                let _ = std::fs::write(&dst_path, &data);
            }
        }
    }
    Ok(())
}

/// Write a [`Topology`] to a session-specific temp directory so it can be
/// bind-mounted into a container.  Also creates stub render-node files
/// under `dev/dri/` and a stub `dev/kfd` file so that `open()` calls from
/// the interceptor find something to classify.
///
/// Returns the root of the temp directory tree.
fn create_synthetic_topology(
    session_name: &str,
    topology: &mirage_schema::topology::Topology,
) -> std::io::Result<PathBuf> {
    let root = std::env::temp_dir()
        .join("mirage")
        .join(format!("topo-{session_name}"));

    let topo_base = root.join("sys/class/kfd/kfd/topology");

    // Write every file from the Topology map.
    let mut render_minors: Vec<u32> = Vec::new();
    for (rel_path, data) in &topology.files {
        let dst = topo_base.join(rel_path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dst, data)?;

        // Collect render minors from node properties files.
        if rel_path.ends_with("/properties") {
            if let Ok(text) = std::str::from_utf8(data) {
                for line in text.lines() {
                    if let Some(rest) = line.strip_prefix("drm_render_minor ") {
                        if let Ok(minor) = rest.trim().parse::<u32>() {
                            if minor > 0 {
                                render_minors.push(minor);
                            }
                        }
                    }
                }
            }
        }
    }

    // Create /dev/dri/renderDxxx stubs (empty files — the interceptor
    // intercepts the actual open()).
    let dri = root.join("dev/dri");
    std::fs::create_dir_all(&dri)?;
    for minor in &render_minors {
        std::fs::write(dri.join(format!("renderD{minor}")), b"")?;
    }

    // Create /dev/kfd stub.
    let dev = root.join("dev");
    std::fs::create_dir_all(&dev)?;
    std::fs::write(dev.join("kfd"), b"")?;

    Ok(root)
}
