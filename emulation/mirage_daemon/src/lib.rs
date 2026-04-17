use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use mirage_container::{
    ContainerHandle, ContainerRuntime, ExecRequest, StartContainerRequest, StartedContainer,
};
use mirage_schema::common::{
    ExecArgs, HealthStatus, ProfileDef, SessionDef, SetEnv, SimulatorMode, Time,
};
use mirage_schema::config::DaemonDef;
use mirage_schema::container::ContainerDef;
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
    container_handle: Option<ContainerHandle>,
}

impl InMemoryMirageDaemon {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State::default()),
            container_runtime: None,
        }
    }

    pub fn with_container_runtime(runtime: Arc<dyn ContainerRuntime>) -> Self {
        Self {
            state: RwLock::new(State::default()),
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

        state
            .sessions
            .insert(session.name.clone(), SessionRecord { session, detail, container_handle: None });

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
            });
        }

        let mut state = self.state.write().await;
        if state.sessions.contains_key(&session.name) {
            return Ok(BootSessionReply {
                ok: false,
                error: Some(format!("session '{}' already exists", session.name)),
                container_id: None,
            });
        }

        let Some(profile) = state.profiles.get(&session.profile).cloned() else {
            return Ok(BootSessionReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", session.profile)),
                container_id: None,
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
            });
        }

        let Some(runtime) = &self.container_runtime else {
            return Ok(BootSessionReply {
                ok: false,
                error: Some("no container runtime configured".to_string()),
                container_id: None,
            });
        };

        // Build a ContainerDef that keeps the container alive for exec calls.
        let container_def = ContainerDef {
            image: session.image.clone(),
            mounts: vec![],
            injected_files: vec![],
            entrypoint: ExecArgs {
                command: "sleep".to_string(),
                args: vec!["infinity".to_string()],
                env: vec![SetEnv {
                    key: "MIRAGE_SESSION".to_string(),
                    value: session.name.clone(),
                }],
            },
            working_dir: None,
            ports: vec![],
            privileged: false,
            resource_limits_json: None,
        };

        let container_name = format!("mirage-{}", session.name);

        // Pull the image first (ignore errors for locally available images).
        let _ = runtime.pull_image(&session.image, None).await;

        let started: StartedContainer = match runtime
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
                return Ok(BootSessionReply {
                    ok: false,
                    error: Some(format!("failed to start container: {err}")),
                    container_id: None,
                });
            }
        };

        let container_id = started.inspection.handle.id.clone();
        let handle = started.inspection.handle.clone();

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
                container_handle: Some(handle),
            },
        );

        Ok(BootSessionReply {
            ok: true,
            error: None,
            container_id: Some(container_id),
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

        let Some(handle) = &record.container_handle else {
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
            exec: request.exec,
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

        if let (Some(handle), Some(runtime)) = (record.container_handle, &self.container_runtime) {
            // Best-effort stop + remove.
            let _ = runtime.stop_container(&handle, 10, None).await;
            let _ = runtime.remove_container(&handle, true, None).await;
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

    use mirage_schema::common::{GpuDef, GpuFamily};

    fn test_simulator() -> SimulatorInfo {
        SimulatorInfo {
            name: "rocjitsu".to_string(),
            version: "1.0.0".to_string(),
            description: Some("functional simulator".to_string()),
            supported_gpus: vec![GpuDef {
                name: "MI300X".to_string(),
                arch: "gfx942".to_string(),
                family: GpuFamily::AmdCdna,
                description: None,
            }],
            supports_custom_gpus: false,
            supported_modes: vec![SimulatorMode::Functional],
        }
    }

    #[tokio::test]
    async fn creates_profile_only_for_registered_simulator() {
        let daemon = InMemoryMirageDaemon::new();

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
        assert!(!reply.ok);

        daemon
            .register_sim(RegisterSimRequest {
                info: test_simulator(),
            })
            .await
            .unwrap();

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
            .register_sim(RegisterSimRequest {
                info: test_simulator(),
            })
            .await
            .unwrap();
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

    async fn daemon_with_mock_runtime() -> (InMemoryMirageDaemon, Arc<mirage_container::MockContainerRuntime>) {
        let mock = Arc::new(mirage_container::MockContainerRuntime::default());
        let daemon = InMemoryMirageDaemon::with_container_runtime(mock.clone());

        daemon
            .register_sim(RegisterSimRequest {
                info: test_simulator(),
            })
            .await
            .unwrap();
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
                    args: vec![
                        "-c".to_string(),
                        "print('vLLM is running')".to_string(),
                    ],
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
}
