use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use async_trait::async_trait;
use tokio::sync::RwLock;

use mirage_container::{
    ContainerHandle, ContainerRuntime, ExecRequest as ContainerExecRequest, StartContainerRequest,
};
use mirage_schema::common::{
    ExecArgs, GpuDef, GpuFamily, HealthStatus, ProfileDef, SessionDef, SetEnv, SimulatorMode, Time,
    WorkloadDef,
};
use mirage_schema::config::DaemonDef;
use mirage_schema::container::{BindMount, ContainerDef};
use mirage_schema::daemon::{
    MirageDaemonAttach, MirageDaemonBoot, MirageDaemonCreateProfile, MirageDaemonCreateSession,
    MirageDaemonCreateWorkload, MirageDaemonDeleteProfile, MirageDaemonDeleteSession,
    MirageDaemonDeleteWorkload, MirageDaemonExec, MirageDaemonStatus,
    MirageDaemonShowSimulator, MirageDaemonShowWorkload, MirageDaemonHealth,
    MirageDaemonListProfiles, MirageDaemonListSessions, MirageDaemonListSimulators,
    MirageDaemonListWorkloads, MirageDaemonOverview, MirageDaemonRegistration,
    MirageDaemonResult, MirageDaemonShutdown, MirageDaemonTime,
};
use mirage_schema::simulator::SimulatorInfo;
use mirage_schema::socket::{
    AttachInput, AttachOutput, AttachReply, AttachRequest, BootReply, BootRequest,
    CreateProfileReply, CreateProfileRequest, CreateSessionReply, CreateSessionRequest,
    CreateWorkloadReply, CreateWorkloadRequest, DeleteProfileReply, DeleteProfileRequest,
    DeleteSessionReply, DeleteSessionRequest, DeleteWorkloadReply, DeleteWorkloadRequest,
    ExecReply, ExecRequest, GetOverviewReply, GetOverviewRequest,
    HealthReply, HealthRequest, ListProfilesReply,
    ListProfilesRequest, ListSessionsReply, ListSessionsRequest, ListSimulatorsReply,
    ListSimulatorsRequest, ListWorkloadsReply, ListWorkloadsRequest, RegisterSimReply,
    RegisterSimRequest, SessionSummary, ShowSimulatorReply, ShowSimulatorRequest,
    ShowWorkloadReply, ShowWorkloadRequest, ShutdownReply, ShutdownRequest,
    SimulatorSummary, StatusReply, StatusRequest, TimeReply, TimeRequest, WorkloadSummary,
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
    workloads: BTreeMap<String, WorkloadDef>,
}

#[derive(Debug, Clone)]
struct SessionRecord {
    session: SessionDef,
    detail: StatusReply,
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

    fn profile_from_request(request: CreateProfileRequest) -> ProfileDef {
        ProfileDef {
            name: request.name,
            simulator: request.simulator,
            mode: request.mode,
            gpu: request.gpu,
            num_gpus: request.gpus_per_node,
            num_nodes: request.nodes,
        }
    }

    fn session_from_parts(name: String, profile: String, image: String) -> SessionDef {
        SessionDef {
            name,
            profile,
            image,
        }
    }

    fn exec_from_command(command: Vec<String>) -> MirageDaemonResult<ExecArgs> {
        let Some((program, args)) = command.split_first() else {
            return Err(mirage_schema::daemon::MirageDaemonError::Remote(
                "no command provided".to_string(),
            ));
        };

        Ok(ExecArgs {
            command: program.clone(),
            args: args.to_vec(),
            env: vec![],
        })
    }

    fn parse_csv_exec(spec: &str) -> ExecArgs {
        let parts: Vec<&str> = spec.split(',').collect();
        ExecArgs {
            command: parts.first().copied().unwrap_or_default().to_string(),
            args: parts[1..].iter().map(|value| (*value).to_string()).collect(),
            env: vec![],
        }
    }

    fn workload_from_request(request: CreateWorkloadRequest) -> WorkloadDef {
        let CreateWorkloadRequest {
            name,
            profile,
            image,
            startup,
            execs,
            cleanup,
        } = request;
        WorkloadDef {
            name,
            profile,
            image,
            startup: startup.as_deref().map(Self::parse_csv_exec),
            execs: execs.iter().map(|value| Self::parse_csv_exec(value)).collect(),
            cleanup,
        }
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
    async fn attach(
        &self,
        _request: AttachRequest,
        mut input: tokio::sync::mpsc::Receiver<AttachInput>,
        _output: tokio::sync::mpsc::Sender<AttachOutput>,
    ) -> MirageDaemonResult<AttachReply> {
        while input.recv().await.is_some() {}
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
impl MirageDaemonListSimulators for InMemoryMirageDaemon {
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
}

#[async_trait]
impl MirageDaemonShowSimulator for InMemoryMirageDaemon {
    async fn show_simulator(
        &self,
        request: ShowSimulatorRequest,
    ) -> MirageDaemonResult<ShowSimulatorReply> {
        let state = self.state.read().await;
        let simulator = state
            .simulators
            .get(&request.name)
            .map(|info| Self::simulator_summary(&state, info));
        Ok(ShowSimulatorReply { simulator })
    }
}

#[async_trait]
impl MirageDaemonListProfiles for InMemoryMirageDaemon {
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
                    .simulator
                    .as_ref()
                    .is_none_or(|filter| profile.simulator == *filter)
            })
            .cloned()
            .collect();
        Ok(ListProfilesReply { profiles })
    }
}

#[async_trait]
impl MirageDaemonCreateProfile for InMemoryMirageDaemon {
    async fn create_profile(
        &self,
        request: CreateProfileRequest,
    ) -> MirageDaemonResult<CreateProfileReply> {
        let profile = Self::profile_from_request(request);
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
}

#[async_trait]
impl MirageDaemonDeleteProfile for InMemoryMirageDaemon {
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
impl MirageDaemonListSessions for InMemoryMirageDaemon {
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
                    .profile
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
}

#[async_trait]
impl MirageDaemonCreateSession for InMemoryMirageDaemon {
    async fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> MirageDaemonResult<CreateSessionReply> {
        let session = Self::session_from_parts(request.name, request.profile, request.image);
        if session.name.trim().is_empty() {
            return Ok(CreateSessionReply {
                ok: false,
                error: Some("session name must not be empty".to_string()),
            });
        }

        let mut state = self.state.write().await;
        if state.sessions.contains_key(&session.name) {
            return Ok(CreateSessionReply {
                ok: false,
                error: Some(format!("session '{}' already exists", session.name)),
            });
        }

        let Some(profile) = state.profiles.get(&session.profile).cloned() else {
            return Ok(CreateSessionReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", session.profile)),
            });
        };

        if !state.simulators.contains_key(&profile.simulator) {
            return Ok(CreateSessionReply {
                ok: false,
                error: Some(format!(
                    "simulator '{}' is not registered",
                    profile.simulator
                )),
            });
        }

        let detail = StatusReply {
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

        Ok(CreateSessionReply {
            ok: true,
            error: None,
        })
    }
}

#[async_trait]
impl MirageDaemonDeleteSession for InMemoryMirageDaemon {
    async fn delete_session(
        &self,
        request: DeleteSessionRequest,
    ) -> MirageDaemonResult<DeleteSessionReply> {
        let mut state = self.state.write().await;
        if state.sessions.remove(&request.name).is_none() {
            return Ok(DeleteSessionReply {
                ok: false,
                error: Some(format!("session '{}' does not exist", request.name)),
            });
        }

        Ok(DeleteSessionReply {
            ok: true,
            error: None,
        })
    }
}

#[async_trait]
impl MirageDaemonStatus for InMemoryMirageDaemon {
    async fn status(
        &self,
        request: StatusRequest,
    ) -> MirageDaemonResult<StatusReply> {
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
    async fn boot(
        &self,
        request: BootRequest,
    ) -> MirageDaemonResult<BootReply> {
        let session = Self::session_from_parts(request.name, request.profile, request.image);
        if session.name.trim().is_empty() {
            return Ok(BootReply {
                ok: false,
                error: Some("session name must not be empty".to_string()),
                container_id: None,
                container_ids: vec![],
            });
        }

        let mut state = self.state.write().await;
        if state.sessions.contains_key(&session.name) {
            return Ok(BootReply {
                ok: false,
                error: Some(format!("session '{}' already exists", session.name)),
                container_id: None,
                container_ids: vec![],
            });
        }

        let Some(profile) = state.profiles.get(&session.profile).cloned() else {
            return Ok(BootReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", session.profile)),
                container_id: None,
                container_ids: vec![],
            });
        };

        if !state.simulators.contains_key(&profile.simulator) {
            return Ok(BootReply {
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
            return Ok(BootReply {
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
                            return Ok(BootReply {
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
                return Ok(BootReply {
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
                    return Ok(BootReply {
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

        let detail = StatusReply {
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

        Ok(BootReply {
            ok: true,
            error: None,
            container_id: head_container_id,
            container_ids,
        })
    }
}

#[async_trait]
impl MirageDaemonExec for InMemoryMirageDaemon {
    async fn exec(
        &self,
        request: ExecRequest,
    ) -> MirageDaemonResult<ExecReply> {
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

        let exec_request = ContainerExecRequest {
            container: handle.clone(),
            exec: {
                let mut exec = Self::exec_from_command(request.command)?;
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
            Ok(result) => Ok(ExecReply {
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
    async fn shutdown(
        &self,
        request: ShutdownRequest,
    ) -> MirageDaemonResult<ShutdownReply> {
        let mut state = self.state.write().await;
        let Some(record) = state.sessions.remove(&request.name) else {
            return Ok(ShutdownReply {
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

        Ok(ShutdownReply {
            ok: true,
            error: None,
        })
    }
}

#[async_trait]
impl MirageDaemonCreateWorkload for InMemoryMirageDaemon {
    async fn create_workload(
        &self,
        request: CreateWorkloadRequest,
    ) -> MirageDaemonResult<CreateWorkloadReply> {
        let workload = Self::workload_from_request(request);

        if workload.name.trim().is_empty() {
            return Ok(CreateWorkloadReply {
                ok: false,
                error: Some("workload name must not be empty".to_string()),
            });
        }

        if workload.execs.is_empty() {
            return Ok(CreateWorkloadReply {
                ok: false,
                error: Some("workload must contain at least one exec step".to_string()),
            });
        }

        let mut state = self.state.write().await;

        if state.workloads.contains_key(&workload.name) {
            return Ok(CreateWorkloadReply {
                ok: false,
                error: Some(format!("workload '{}' already exists", workload.name)),
            });
        }

        // Validate the referenced profile exists.
        if !state.profiles.contains_key(&workload.profile) {
            return Ok(CreateWorkloadReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", workload.profile)),
            });
        }

        state.workloads.insert(workload.name.clone(), workload);
        Ok(CreateWorkloadReply {
            ok: true,
            error: None,
        })
    }
}

#[async_trait]
impl MirageDaemonListWorkloads for InMemoryMirageDaemon {
    async fn list_workloads(
        &self,
        _request: ListWorkloadsRequest,
    ) -> MirageDaemonResult<ListWorkloadsReply> {
        let state = self.state.read().await;
        let workloads = state
            .workloads
            .values()
            .map(|w| WorkloadSummary {
                name: w.name.clone(),
                profile: w.profile.clone(),
                image: w.image.clone(),
                has_startup: w.startup.is_some(),
                exec_count: w.execs.len() as u32,
                cleanup: w.cleanup,
            })
            .collect();
        Ok(ListWorkloadsReply { workloads })
    }
}

#[async_trait]
impl MirageDaemonShowWorkload for InMemoryMirageDaemon {
    async fn show_workload(
        &self,
        request: ShowWorkloadRequest,
    ) -> MirageDaemonResult<ShowWorkloadReply> {
        let state = self.state.read().await;
        let workload = state.workloads.get(&request.name).cloned();
        Ok(ShowWorkloadReply { workload })
    }
}

#[async_trait]
impl MirageDaemonDeleteWorkload for InMemoryMirageDaemon {
    async fn delete_workload(
        &self,
        request: DeleteWorkloadRequest,
    ) -> MirageDaemonResult<DeleteWorkloadReply> {
        let mut state = self.state.write().await;
        if state.workloads.remove(&request.name).is_none() {
            return Ok(DeleteWorkloadReply {
                ok: false,
                error: Some(format!("workload '{}' does not exist", request.name)),
            });
        }

        Ok(DeleteWorkloadReply {
            ok: true,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_profile_request(profile: ProfileDef) -> CreateProfileRequest {
        CreateProfileRequest {
            name: profile.name,
            simulator: profile.simulator,
            gpu: profile.gpu,
            mode: profile.mode,
            gpus_per_node: profile.num_gpus,
            nodes: profile.num_nodes,
        }
    }

    fn create_session_request(session: SessionDef) -> CreateSessionRequest {
        CreateSessionRequest {
            name: session.name,
            profile: session.profile,
            image: session.image,
        }
    }

    fn boot_request(session: SessionDef) -> BootRequest {
        BootRequest {
            name: session.name,
            profile: session.profile,
            image: session.image,
        }
    }

    fn exec_request(session_name: impl Into<String>, exec: ExecArgs) -> ExecRequest {
        let mut command = vec![exec.command];
        command.extend(exec.args);
        ExecRequest {
            session_name: session_name.into(),
            command,
        }
    }

    fn create_workload_request(workload: WorkloadDef) -> CreateWorkloadRequest {
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

    #[tokio::test]
    async fn creates_profile_only_for_registered_simulator() {
        let daemon = InMemoryMirageDaemon::new();

        // Unknown simulator should fail.
        let reply = daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "custom".to_string(),
                simulator: "nonexistent".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();
        assert!(!reply.ok);

        // Built-in rocjitsu should succeed.
        let reply = daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();
        assert!(reply.ok);
    }

    #[tokio::test]
    async fn creates_and_lists_sessions() {
        let daemon = InMemoryMirageDaemon::new();
        daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();

        let reply = daemon
            .create_session(create_session_request(SessionDef {
                name: "session-a".to_string(),
                profile: "mi300x".to_string(),
                image: "ghcr.io/example/image:latest".to_string(),
            }))
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
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();

        (daemon, mock)
    }

    #[tokio::test]
    async fn boot_creates_session_and_starts_container() {
        let (daemon, mock) = daemon_with_mock_runtime().await;

        let reply = daemon
            .boot(boot_request(SessionDef {
                name: "vllm-test".to_string(),
                profile: "mi300x".to_string(),
                image: "ghcr.io/rocm/vllm:latest".to_string(),
            }))
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
            .boot(boot_request(SessionDef {
                name: "test".to_string(),
                profile: "nonexistent".to_string(),
                image: "img:latest".to_string(),
            }))
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
            .boot(boot_request(SessionDef {
                name: "exec-test".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
            }))
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
            .exec(exec_request(
                "exec-test",
                ExecArgs {
                    command: "python".to_string(),
                    args: vec![
                        "-c".to_string(),
                        "import torch; print('GPU available')".to_string(),
                    ],
                    env: vec![],
                },
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&reply.stdout), "GPU available\n");
    }

    #[tokio::test]
    async fn exec_fails_for_non_booted_session() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;

        let result = daemon
            .exec(exec_request(
                "nonexistent",
                ExecArgs {
                    command: "echo".to_string(),
                    args: vec!["hello".to_string()],
                    env: vec![],
                },
            ))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_stops_and_removes_session() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;

        // Boot.
        let boot = daemon
            .boot(boot_request(SessionDef {
                name: "shutdown-test".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
            }))
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
            .shutdown(ShutdownRequest {
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
            .shutdown(ShutdownRequest {
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
            .boot(boot_request(SessionDef {
                name: "vllm-e2e".to_string(),
                profile: "mi300x".to_string(),
                image: "ghcr.io/rocm/vllm:latest".to_string(),
            }))
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
            .exec(exec_request(
                "vllm-e2e",
                ExecArgs {
                    command: "python".to_string(),
                    args: vec!["-c".to_string(), "print('vLLM is running')".to_string()],
                    env: vec![],
                },
            ))
            .await
            .unwrap();
        assert_eq!(exec_reply.exit_code, 0);
        assert_eq!(
            String::from_utf8_lossy(&exec_reply.stdout),
            "vLLM is running\n"
        );

        // 3. Shutdown.
        let shutdown = daemon
            .shutdown(ShutdownRequest {
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
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x-2node".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 8,
                num_nodes: 2,
            }))
            .await
            .unwrap();

        (daemon, mock)
    }

    #[tokio::test]
    async fn boot_multinode_starts_multiple_containers_with_network() {
        let (daemon, mock) = daemon_with_multinode_profile().await;

        let reply = daemon
            .boot(boot_request(SessionDef {
                name: "multi-test".to_string(),
                profile: "mi300x-2node".to_string(),
                image: "ghcr.io/rocm/vllm:latest".to_string(),
            }))
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
            .boot(boot_request(SessionDef {
                name: "multi-shutdown".to_string(),
                profile: "mi300x-2node".to_string(),
                image: "img:latest".to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok);

        let shutdown = daemon
            .shutdown(ShutdownRequest {
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

    async fn daemon_with_profile() -> InMemoryMirageDaemon {
        let daemon = InMemoryMirageDaemon::new();
        daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();
        daemon
    }

    fn sample_exec(cmd: &str) -> ExecArgs {
        ExecArgs {
            command: cmd.to_string(),
            args: vec![],
            env: vec![],
        }
    }

    #[tokio::test]
    async fn workload_create_and_list() {
        let daemon = daemon_with_profile().await;

        let reply = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "torch-smoke".to_string(),
                profile: "mi300x".to_string(),
                image: "ghcr.io/example/img:latest".to_string(),
                startup: None,
                execs: vec![sample_exec("rocminfo")],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();
        assert!(reply.ok, "create should succeed: {:?}", reply.error);

        let list = daemon
            .list_workloads(ListWorkloadsRequest::default())
            .await
            .unwrap();
        assert_eq!(list.workloads.len(), 1);
        assert_eq!(list.workloads[0].name, "torch-smoke");
        assert_eq!(list.workloads[0].exec_count, 1);
        assert!(!list.workloads[0].has_startup);
    }

    #[tokio::test]
    async fn workload_create_with_startup() {
        let daemon = daemon_with_profile().await;

        let reply = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "with-startup".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
                startup: Some(sample_exec("init.sh")),
                execs: vec![sample_exec("step1"), sample_exec("step2")],
                cleanup: mirage_schema::common::CleanupPolicy::OnSuccess,
            }))
            .await
            .unwrap();
        assert!(reply.ok);

        let list = daemon
            .list_workloads(ListWorkloadsRequest::default())
            .await
            .unwrap();
        assert_eq!(list.workloads[0].has_startup, true);
        assert_eq!(list.workloads[0].exec_count, 2);
        assert_eq!(
            list.workloads[0].cleanup,
            mirage_schema::common::CleanupPolicy::OnSuccess
        );
    }

    #[tokio::test]
    async fn workload_create_rejects_empty_name() {
        let daemon = daemon_with_profile().await;

        let reply = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
                startup: None,
                execs: vec![sample_exec("rocminfo")],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("must not be empty"));
    }

    #[tokio::test]
    async fn workload_create_rejects_empty_execs() {
        let daemon = daemon_with_profile().await;

        let reply = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "no-execs".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
                startup: None,
                execs: vec![],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("at least one exec"));
    }

    #[tokio::test]
    async fn workload_create_rejects_missing_profile() {
        let daemon = InMemoryMirageDaemon::new();

        let reply = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "bad-profile".to_string(),
                profile: "nonexistent".to_string(),
                image: "img:latest".to_string(),
                startup: None,
                execs: vec![sample_exec("rocminfo")],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("does not exist"));
    }

    #[tokio::test]
    async fn workload_create_rejects_duplicate_name() {
        let daemon = daemon_with_profile().await;

        let workload = WorkloadDef {
            name: "dup".to_string(),
            profile: "mi300x".to_string(),
            image: "img:latest".to_string(),
            startup: None,
            execs: vec![sample_exec("rocminfo")],
            cleanup: mirage_schema::common::CleanupPolicy::Always,
        };

        let r1 = daemon
            .create_workload(create_workload_request(workload.clone()))
            .await
            .unwrap();
        assert!(r1.ok);

        let r2 = daemon.create_workload(create_workload_request(workload)).await.unwrap();
        assert!(!r2.ok);
        assert!(r2.error.unwrap().contains("already exists"));
    }

    #[tokio::test]
    async fn workload_get_and_show() {
        let daemon = daemon_with_profile().await;

        daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "showme".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
                startup: Some(sample_exec("setup")),
                execs: vec![sample_exec("run")],
                cleanup: mirage_schema::common::CleanupPolicy::Never,
            }))
            .await
            .unwrap();

        let reply = daemon
            .show_workload(ShowWorkloadRequest {
                name: "showme".to_string(),
            })
            .await
            .unwrap();
        let w = reply.workload.expect("workload should exist");
        assert_eq!(w.name, "showme");
        assert_eq!(w.profile, "mi300x");
        assert!(w.startup.is_some());
        assert_eq!(w.execs.len(), 1);
        assert_eq!(w.cleanup, mirage_schema::common::CleanupPolicy::Never);

        // Non-existent workload returns None.
        let reply = daemon
            .show_workload(ShowWorkloadRequest {
                name: "nope".to_string(),
            })
            .await
            .unwrap();
        assert!(reply.workload.is_none());
    }

    #[tokio::test]
    async fn workload_delete() {
        let daemon = daemon_with_profile().await;

        daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "deleteme".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
                startup: None,
                execs: vec![sample_exec("run")],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();

        let reply = daemon
            .delete_workload(DeleteWorkloadRequest {
                name: "deleteme".to_string(),
            })
            .await
            .unwrap();
        assert!(reply.ok);

        // Should be gone.
        let list = daemon
            .list_workloads(ListWorkloadsRequest::default())
            .await
            .unwrap();
        assert!(list.workloads.is_empty());
    }

    #[tokio::test]
    async fn workload_delete_nonexistent_fails() {
        let daemon = InMemoryMirageDaemon::new();

        let reply = daemon
            .delete_workload(DeleteWorkloadRequest {
                name: "nope".to_string(),
            })
            .await
            .unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("does not exist"));
    }

    // -----------------------------------------------------------------------
    //  MNIST training E2E tests
    // -----------------------------------------------------------------------

    const MNIST_IMAGE: &str = "docker.io/rocm/pytorch:rocm6.4_ubuntu24.04_py3.12_pytorch_release_2.6.0";

    fn mnist_exec(cmd: &str, args: &[&str]) -> ExecArgs {
        ExecArgs {
            command: cmd.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            env: vec![],
        }
    }

    fn _mnist_exec_with_env(cmd: &str, args: &[&str], env: Vec<SetEnv>) -> ExecArgs {
        ExecArgs {
            command: cmd.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            env,
        }
    }

    fn mnist_result_json(accuracy: f64, passed: bool) -> String {
        format!(
            r#"{{"status":"success","device":"cuda","cuda_available":true,"epochs":2,"training_time_s":12.34,"test_loss":0.0456,"test_accuracy":{accuracy},"model_parameters":206922,"passed":{passed}}}"#
        )
    }

    async fn mnist_daemon_with_mock() -> (
        InMemoryMirageDaemon,
        Arc<mirage_container::MockContainerRuntime>,
    ) {
        let mock = Arc::new(mirage_container::MockContainerRuntime::default());
        let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

        daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x-mnist".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();

        (daemon, mock)
    }

    async fn boot_mnist_session(
        daemon: &InMemoryMirageDaemon,
        mock: &Arc<mirage_container::MockContainerRuntime>,
    ) -> (String, mirage_container::ContainerHandle) {
        let boot = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-test".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok, "MNIST boot failed: {:?}", boot.error);

        let container_id = boot.container_id.clone().unwrap();
        let starts = mock.start_requests().await;
        let handle = mirage_container::ContainerHandle {
            id: container_id.clone(),
            name: starts.last().unwrap().name.clone(),
        };
        (container_id, handle)
    }

    #[tokio::test]
    async fn mnist_profile_creation_with_single_gpu() {
        let daemon = InMemoryMirageDaemon::new();

        let reply = daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x-mnist".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();
        assert!(reply.ok, "profile creation should succeed");

        let profiles = daemon
            .list_profiles(ListProfilesRequest::default())
            .await
            .unwrap();
        assert_eq!(profiles.profiles.len(), 1);
        assert_eq!(profiles.profiles[0].name, "mi300x-mnist");
        assert_eq!(profiles.profiles[0].simulator, "rocjitsu");
        assert_eq!(profiles.profiles[0].gpu, "MI300X");
        assert_eq!(profiles.profiles[0].num_gpus, 1);
        assert_eq!(profiles.profiles[0].num_nodes, 1);
        assert_eq!(profiles.profiles[0].mode, SimulatorMode::Functional);
    }

    #[tokio::test]
    async fn mnist_profile_rejects_unsupported_gpu() {
        let daemon = InMemoryMirageDaemon::new();

        let reply = daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "bad-gpu".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "RTX4090".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();
        assert!(!reply.ok, "should reject unsupported GPU");
    }

    #[tokio::test]
    async fn mnist_boot_session_starts_container() {
        let (daemon, mock) = mnist_daemon_with_mock().await;

        let boot = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-boot".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();

        assert!(boot.ok, "boot should succeed: {:?}", boot.error);
        assert!(boot.container_id.is_some());

        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 1);
        assert_eq!(sessions.sessions[0].name.as_deref(), Some("mnist-boot"));
        assert_eq!(
            sessions.sessions[0].image.as_deref(),
            Some(MNIST_IMAGE)
        );

        let starts = mock.start_requests().await;
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].container.image, MNIST_IMAGE);
    }

    #[tokio::test]
    async fn mnist_boot_rejects_duplicate_session_name() {
        let (daemon, _mock) = mnist_daemon_with_mock().await;

        let boot1 = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-dup".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot1.ok);

        let boot2 = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-dup".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(!boot2.ok, "duplicate session should fail");
    }

    #[tokio::test]
    async fn mnist_exec_python_version_check() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: b"Python 3.12.4\n".to_vec(),
                stderr: vec![],
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["-c", "import sys; print(f'Python {sys.version}')"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 0);
        assert!(String::from_utf8_lossy(&reply.stdout).contains("Python"));
    }

    #[tokio::test]
    async fn mnist_exec_pytorch_gpu_detection() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        let gpu_report = serde_json::json!({
            "pytorch_version": "2.6.0",
            "cuda_available": true,
            "hip_version": "6.4.0",
            "gpu_count": 1,
            "gpu_name": "AMD Instinct MI300X"
        });

        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: format!("{}\n", gpu_report).into_bytes(),
                stderr: vec![],
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["-c", "import torch, json; print(json.dumps({'pytorch_version': torch.__version__}))"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 0);
        let stdout = String::from_utf8_lossy(&reply.stdout);
        let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
        assert_eq!(parsed["cuda_available"], true);
        assert_eq!(parsed["gpu_count"], 1);
        assert_eq!(parsed["gpu_name"], "AMD Instinct MI300X");
    }

    #[tokio::test]
    async fn mnist_exec_torchvision_check() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: b"torchvision 0.21.0\nMNIST dataset and transforms: OK\n".to_vec(),
                stderr: vec![],
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["-c", "import torchvision; print('OK')"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 0);
        let stdout = String::from_utf8_lossy(&reply.stdout);
        assert!(stdout.contains("torchvision"));
    }

    #[tokio::test]
    async fn mnist_exec_training_succeeds_with_high_accuracy() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        let result_json = mnist_result_json(97.5, true);
        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: format!("Training...\n--- RESULT ---\n{result_json}\nMNIST training test passed.\n").into_bytes(),
                stderr: vec![],
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["-c", "# MNIST training code"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 0);
        let stdout = String::from_utf8_lossy(&reply.stdout);
        assert!(stdout.contains("MNIST training test passed"));
        assert!(stdout.contains("--- RESULT ---"));

        let result_line = stdout
            .lines()
            .find(|l| l.starts_with('{'))
            .expect("should contain JSON result");
        let result: serde_json::Value = serde_json::from_str(result_line).unwrap();
        assert_eq!(result["status"], "success");
        assert_eq!(result["passed"], true);
        assert!(result["test_accuracy"].as_f64().unwrap() > 85.0);
    }

    #[tokio::test]
    async fn mnist_exec_training_fails_with_low_accuracy() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        let result_json = mnist_result_json(42.0, false);
        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 1,
                stdout: format!("--- RESULT ---\n{result_json}\n").into_bytes(),
                stderr: b"FAILED: test accuracy 42.0% < 85.0% threshold\n".to_vec(),
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["-c", "# training fails"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 1);
        let stderr = String::from_utf8_lossy(&reply.stderr);
        assert!(stderr.contains("FAILED"));
        assert!(stderr.contains("42.0%"));
    }

    #[tokio::test]
    async fn mnist_full_e2e_boot_train_shutdown() {
        let (daemon, mock) = mnist_daemon_with_mock().await;

        // 1. Boot session.
        let boot = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-e2e".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok);
        let container_id = boot.container_id.unwrap();
        let starts = mock.start_requests().await;
        let handle = mirage_container::ContainerHandle {
            id: container_id,
            name: starts[0].name.clone(),
        };

        // 2. Pre-flight: Python version.
        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: b"Python 3.12.4\n".to_vec(),
                stderr: vec![],
            },
        )
        .await;
        let r = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-e2e",
                mnist_exec("python", &["--version"]),
            ))
            .await
            .unwrap();
        assert_eq!(r.exit_code, 0);

        // 3. Pre-flight: PyTorch + GPU.
        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: b"{\"pytorch\": \"2.6.0\", \"cuda\": true, \"gpu\": \"MI300X\"}\n".to_vec(),
                stderr: vec![],
            },
        )
        .await;
        let r = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-e2e",
                mnist_exec("python", &["-c", "import torch; print('ok')"]),
            ))
            .await
            .unwrap();
        assert_eq!(r.exit_code, 0);

        // 4. Training run.
        let result_json = mnist_result_json(98.1, true);
        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: format!(
                    "Device: cuda\nGPU: AMD Instinct MI300X\n\
                     Epoch 1: loss=0.1234 acc=95.2%\n\
                     Epoch 2: loss=0.0567 acc=98.1%\n\
                     --- RESULT ---\n{result_json}\n\
                     MNIST training test passed.\n"
                )
                .into_bytes(),
                stderr: vec![],
            },
        )
        .await;
        let r = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-e2e",
                mnist_exec("python", &["/workspace/mnist_train.py", "--epochs", "2"]),
            ))
            .await
            .unwrap();
        assert_eq!(r.exit_code, 0);
        let stdout = String::from_utf8_lossy(&r.stdout);
        assert!(stdout.contains("MNIST training test passed"));
        assert!(stdout.contains("Device: cuda"));

        // 5. Shutdown.
        let shutdown = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "mnist-e2e".to_string(),
            })
            .await
            .unwrap();
        assert!(shutdown.ok);

        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert!(sessions.sessions.is_empty());
    }

    #[tokio::test]
    async fn mnist_session_detail_shows_correct_info() {
        let (daemon, _mock) = mnist_daemon_with_mock().await;

        let boot = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-detail".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok);

        let detail = daemon
            .get_session_detail(GetSessionDetailRequest {
                name: "mnist-detail".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(detail.name.as_deref(), Some("mnist-detail"));
        assert_eq!(detail.simulator.as_deref(), Some("rocjitsu"));
        assert_eq!(detail.image.as_deref(), Some(MNIST_IMAGE));
        let profile = detail.profile.unwrap();
        assert_eq!(profile.gpu, "MI300X");
        assert_eq!(profile.mode, SimulatorMode::Functional);
    }

    #[tokio::test]
    async fn mnist_workload_create_single_step() {
        let daemon = InMemoryMirageDaemon::new();
        daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x-mnist".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();

        let reply = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "mnist-train".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
                startup: None,
                execs: vec![mnist_exec(
                    "python",
                    &["/workspace/mnist_train.py", "--epochs", "2"],
                )],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();
        assert!(reply.ok, "workload create should succeed: {:?}", reply.error);

        let list = daemon
            .list_workloads(ListWorkloadsRequest::default())
            .await
            .unwrap();
        assert_eq!(list.workloads.len(), 1);
        assert_eq!(list.workloads[0].name, "mnist-train");
        assert_eq!(list.workloads[0].profile, "mi300x-mnist");
        assert_eq!(list.workloads[0].image, MNIST_IMAGE);
        assert!(!list.workloads[0].has_startup);
        assert_eq!(list.workloads[0].exec_count, 1);
    }

    #[tokio::test]
    async fn mnist_workload_create_multi_step_with_startup() {
        let daemon = InMemoryMirageDaemon::new();
        daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x-mnist".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();

        let reply = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "mnist-full".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
                startup: Some(mnist_exec("pip", &["install", "torchvision"])),
                execs: vec![
                    mnist_exec("python", &["-c", "import torch; assert torch.cuda.is_available()"]),
                    mnist_exec("python", &["-c", "import torchvision; print('OK')"]),
                    mnist_exec("python", &["/workspace/mnist_train.py", "--epochs", "2"]),
                ],
                cleanup: mirage_schema::common::CleanupPolicy::OnSuccess,
            }))
            .await
            .unwrap();
        assert!(reply.ok);

        let workload = daemon
            .get_workload(GetWorkloadRequest {
                name: "mnist-full".to_string(),
            })
            .await
            .unwrap()
            .workload
            .unwrap();
        assert_eq!(workload.name, "mnist-full");
        assert!(workload.startup.is_some());
        assert_eq!(workload.startup.unwrap().command, "pip");
        assert_eq!(workload.execs.len(), 3);
        assert_eq!(workload.execs[2].command, "python");
        assert_eq!(
            workload.cleanup,
            mirage_schema::common::CleanupPolicy::OnSuccess
        );
    }

    #[tokio::test]
    async fn mnist_workload_delete_and_recreate() {
        let daemon = InMemoryMirageDaemon::new();
        daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x-mnist".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();

        // Create.
        let r = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "mnist-temp".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
                startup: None,
                execs: vec![mnist_exec("python", &["train.py"])],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();
        assert!(r.ok);

        // Delete.
        let r = daemon
            .delete_workload(DeleteWorkloadRequest {
                name: "mnist-temp".to_string(),
            })
            .await
            .unwrap();
        assert!(r.ok);
        assert!(
            daemon
                .list_workloads(ListWorkloadsRequest::default())
                .await
                .unwrap()
                .workloads
                .is_empty()
        );

        // Recreate with same name.
        let r = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "mnist-temp".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
                startup: None,
                execs: vec![mnist_exec("python", &["train_v2.py"])],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();
        assert!(r.ok, "recreating workload after delete should succeed");
    }

    #[tokio::test]
    async fn mnist_exec_with_import_error_returns_nonzero() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 1,
                stdout: vec![],
                stderr: b"ModuleNotFoundError: No module named 'torchvision'\n".to_vec(),
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["-c", "import torchvision"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 1);
        assert!(String::from_utf8_lossy(&reply.stderr).contains("ModuleNotFoundError"));
    }

    #[tokio::test]
    async fn mnist_exec_gpu_oom_returns_nonzero() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 1,
                stdout: b"Device: cuda\n".to_vec(),
                stderr: b"RuntimeError: HIP out of memory. Tried to allocate 2.00 GiB\n".to_vec(),
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["train.py", "--batch-size", "65536"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 1);
        assert!(String::from_utf8_lossy(&reply.stderr).contains("out of memory"));
    }

    #[tokio::test]
    async fn mnist_multiple_sequential_execs() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        let steps: Vec<(&str, Vec<u8>)> = vec![
            ("python --version", b"Python 3.12.4\n".to_vec()),
            ("rocminfo", b"ROCk module loaded\n1 agent(s)\n".to_vec()),
            ("nvidia-smi", b"AMD Instinct MI300X\n".to_vec()),
            ("python -c 'import torch'", b"torch OK\n".to_vec()),
            ("python train.py", b"Training complete. Accuracy: 97.5%\n".to_vec()),
        ];

        for (_i, (_desc, output)) in steps.iter().enumerate() {
            mock.queue_exec_result(
                &handle,
                mirage_container::ExecResult {
                    exit_code: 0,
                    stdout: output.clone(),
                    stderr: vec![],
                },
            )
            .await;
        }

        for (i, (_desc, expected_output)) in steps.iter().enumerate() {
            let reply = daemon
                .exec_in_session(exec_in_session_request(
                    "mnist-test",
                    mnist_exec("python", &["-c", &format!("step {i}")]),
                ))
                .await
                .unwrap();
            assert_eq!(reply.exit_code, 0, "step {i} should succeed");
            assert_eq!(reply.stdout, *expected_output, "step {i} output mismatch");
        }

        let exec_requests = mock.exec_requests().await;
        assert_eq!(exec_requests.len(), 5, "should have 5 exec requests");
    }

    #[tokio::test]
    async fn mnist_shutdown_after_failed_training_cleans_up() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        // Training fails.
        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 1,
                stdout: vec![],
                stderr: b"CUDA error: device-side assert triggered\n".to_vec(),
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["train.py"]),
            ))
            .await
            .unwrap();
        assert_eq!(reply.exit_code, 1);

        // Shutdown should still succeed.
        let shutdown = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "mnist-test".to_string(),
            })
            .await
            .unwrap();
        assert!(shutdown.ok, "shutdown after failed training should succeed");

        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert!(sessions.sessions.is_empty(), "session should be cleaned up");
    }

    #[tokio::test]
    async fn mnist_profile_with_multi_gpu() {
        let daemon = InMemoryMirageDaemon::new();

        let reply = daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x-8gpu".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 8,
                num_nodes: 1,
            }))
            .await
            .unwrap();
        assert!(reply.ok);

        let profiles = daemon
            .list_profiles(ListProfilesRequest::default())
            .await
            .unwrap();
        assert_eq!(profiles.profiles[0].num_gpus, 8);
    }

    #[tokio::test]
    async fn mnist_profile_with_mi350x() {
        let daemon = InMemoryMirageDaemon::new();

        let reply = daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi350x-mnist".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI350X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();
        assert!(reply.ok, "MI350X should be a supported GPU");
    }

    #[tokio::test]
    async fn mnist_exec_captures_both_stdout_and_stderr() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: b"Epoch 1: loss=0.12 acc=95.2%\nEpoch 2: loss=0.05 acc=98.1%\n".to_vec(),
                stderr: b"UserWarning: plan_cache is deprecated\n".to_vec(),
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["train.py"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 0);
        let stdout = String::from_utf8_lossy(&reply.stdout);
        let stderr = String::from_utf8_lossy(&reply.stderr);
        assert!(stdout.contains("Epoch 1"));
        assert!(stdout.contains("Epoch 2"));
        assert!(stderr.contains("UserWarning"));
    }

    #[tokio::test]
    async fn mnist_container_image_passed_correctly() {
        let (daemon, mock) = mnist_daemon_with_mock().await;

        let boot = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-image-check".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok);

        let starts = mock.start_requests().await;
        assert_eq!(starts[0].container.image, MNIST_IMAGE);

        let pulled = mock.pulled_images().await;
        assert!(
            pulled.contains(&MNIST_IMAGE.to_string()),
            "image should have been pulled"
        );
    }

    #[tokio::test]
    async fn mnist_overview_reflects_active_session() {
        let (daemon, _mock) = mnist_daemon_with_mock().await;

        let overview_before = daemon
            .get_overview(GetOverviewRequest::default())
            .await
            .unwrap();
        let session_count_before = overview_before.session_count;

        let boot = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-overview".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok);

        let overview_after = daemon
            .get_overview(GetOverviewRequest::default())
            .await
            .unwrap();
        assert_eq!(
            overview_after.session_count,
            session_count_before + 1,
            "session count should increase after boot"
        );
    }

    #[tokio::test]
    async fn mnist_exec_training_result_json_parsing() {
        let (daemon, mock) = mnist_daemon_with_mock().await;
        let (_cid, handle) = boot_mnist_session(&daemon, &mock).await;

        let expected_result = serde_json::json!({
            "status": "success",
            "device": "cuda",
            "cuda_available": true,
            "gpu_name": "AMD Instinct MI300X",
            "hip_version": "6.4.0",
            "epochs": 2,
            "batch_size": 256,
            "learning_rate": 0.01,
            "training_time_s": 8.42,
            "epoch_results": [
                {"epoch": 1, "train_loss": 0.1234, "train_acc": 95.2},
                {"epoch": 2, "train_loss": 0.0567, "train_acc": 98.1}
            ],
            "test_loss": 0.0456,
            "test_accuracy": 97.5,
            "model_parameters": 206922,
            "passed": true
        });

        mock.queue_exec_result(
            &handle,
            mirage_container::ExecResult {
                exit_code: 0,
                stdout: format!(
                    "Device: cuda\nLoading MNIST...\nTraining...\n--- RESULT ---\n{}\nMNIST training test passed.\n",
                    serde_json::to_string(&expected_result).unwrap()
                )
                .into_bytes(),
                stderr: vec![],
            },
        )
        .await;

        let reply = daemon
            .exec_in_session(exec_in_session_request(
                "mnist-test",
                mnist_exec("python", &["mnist_train.py"]),
            ))
            .await
            .unwrap();

        assert_eq!(reply.exit_code, 0);
        let stdout = String::from_utf8_lossy(&reply.stdout);

        // Extract and parse the JSON result block.
        let json_str = stdout
            .lines()
            .skip_while(|l| !l.contains("--- RESULT ---"))
            .skip(1)
            .take_while(|l| !l.contains("MNIST training test"))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed: serde_json::Value = serde_json::from_str(json_str.trim()).unwrap();

        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["device"], "cuda");
        assert_eq!(parsed["cuda_available"], true);
        assert_eq!(parsed["epochs"], 2);
        assert_eq!(parsed["test_accuracy"], 97.5);
        assert_eq!(parsed["passed"], true);
        assert_eq!(parsed["model_parameters"], 206922);

        let epoch_results = parsed["epoch_results"].as_array().unwrap();
        assert_eq!(epoch_results.len(), 2);
        assert!(epoch_results[1]["train_acc"].as_f64().unwrap() > epoch_results[0]["train_acc"].as_f64().unwrap());
    }

    #[tokio::test]
    async fn mnist_exec_on_nonexistent_session_fails() {
        let (daemon, _mock) = mnist_daemon_with_mock().await;

        let result = daemon
            .exec_in_session(exec_in_session_request(
                "no-such-session",
                mnist_exec("python", &["train.py"]),
            ))
            .await;

        assert!(result.is_err(), "exec on missing session should error");
    }

    #[tokio::test]
    async fn mnist_double_shutdown_second_fails() {
        let (daemon, _mock) = mnist_daemon_with_mock().await;

        let boot = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-dbl-shut".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok);

        let r1 = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "mnist-dbl-shut".to_string(),
            })
            .await
            .unwrap();
        assert!(r1.ok);

        let r2 = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "mnist-dbl-shut".to_string(),
            })
            .await
            .unwrap();
        assert!(!r2.ok, "second shutdown should fail");
    }

    #[tokio::test]
    async fn mnist_workload_accepts_any_valid_image() {
        let daemon = InMemoryMirageDaemon::new();
        daemon
            .create_profile(create_profile_request(ProfileDef {
                name: "mi300x-mnist".to_string(),
                simulator: "rocjitsu".to_string(),
                mode: SimulatorMode::Functional,
                gpu: "MI300X".to_string(),
                num_gpus: 1,
                num_nodes: 1,
            }))
            .await
            .unwrap();

        let reply = daemon
            .create_workload(create_workload_request(WorkloadDef {
                name: "custom-image".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: "ghcr.io/custom/pytorch:latest".to_string(),
                startup: None,
                execs: vec![mnist_exec("python", &["train.py"])],
                cleanup: mirage_schema::common::CleanupPolicy::Always,
            }))
            .await
            .unwrap();
        assert!(reply.ok, "workload with custom image should succeed");

        let w = daemon
            .get_workload(GetWorkloadRequest {
                name: "custom-image".to_string(),
            })
            .await
            .unwrap()
            .workload
            .unwrap();
        assert_eq!(w.image, "ghcr.io/custom/pytorch:latest");
    }

    #[tokio::test]
    async fn mnist_concurrent_sessions_independent() {
        let (daemon, _mock) = mnist_daemon_with_mock().await;

        // Boot two sessions.
        let boot_a = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-a".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot_a.ok);

        let boot_b = daemon
            .boot_session(boot_session_request(SessionDef {
                name: "mnist-b".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot_b.ok);

        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 2);

        // Shutdown only one.
        let shutdown = daemon
            .shutdown_session(ShutdownSessionRequest {
                name: "mnist-a".to_string(),
            })
            .await
            .unwrap();
        assert!(shutdown.ok);

        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 1);
        assert_eq!(sessions.sessions[0].name.as_deref(), Some("mnist-b"));
    }

    #[tokio::test]
    async fn mnist_simulator_listed_with_correct_capabilities() {
        let daemon = InMemoryMirageDaemon::new();

        let sims = daemon
            .list_simulators(ListSimulatorsRequest::default())
            .await
            .unwrap();

        let rocjitsu = sims
            .simulators
            .iter()
            .find(|s| s.name.as_deref() == Some("rocjitsu"))
            .expect("rocjitsu should be registered");

        assert!(rocjitsu.version.is_some());
        assert!(
            rocjitsu
                .supported_gpus
                .iter()
                .any(|g| g.name == "MI300X"),
            "MI300X should be supported"
        );
        assert!(
            rocjitsu
                .supported_gpus
                .iter()
                .any(|g| g.name == "MI350X"),
            "MI350X should be supported"
        );
        assert!(
            rocjitsu
                .supported_modes
                .contains(&SimulatorMode::Functional),
            "functional mode should be supported"
        );
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
        // Fallback: cargo target/debug for the repo-root workspace.
        Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/debug/libmirage_interceptor.so"
        ))),
        // Compatibility fallback for pre-move builds rooted under emulation/.
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
