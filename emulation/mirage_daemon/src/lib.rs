use std::collections::BTreeMap;

use async_trait::async_trait;
use tokio::sync::RwLock;

use mirage_schema::common::{HealthStatus, ProfileDef, SessionDef, SimulatorMode, Time};
use mirage_schema::config::DaemonDef;
use mirage_schema::daemon::{
    MirageDaemonAttach, MirageDaemonHealth, MirageDaemonOverview, MirageDaemonProfiles,
    MirageDaemonRegistration, MirageDaemonResult, MirageDaemonSessions, MirageDaemonSimulators,
    MirageDaemonTime,
};
use mirage_schema::simulator::SimulatorInfo;
use mirage_schema::socket::{
    AttachReply, AttachRequest, CreateProfileReply, CreateProfileRequest,
    DashboardCreateSessionReply, DashboardCreateSessionRequest, DashboardDeleteSessionReply,
    DashboardDeleteSessionRequest, DeleteProfileReply, DeleteProfileRequest, GetOverviewReply,
    GetOverviewRequest, GetSessionDetailReply, GetSessionDetailRequest, GetSimulatorReply,
    GetSimulatorRequest, HealthReply, HealthRequest, ListProfilesReply, ListProfilesRequest,
    ListSessionsReply, ListSessionsRequest, ListSimulatorsReply, ListSimulatorsRequest,
    RegisterSimReply, RegisterSimRequest, SessionSummary, SimulatorSummary, TimeReply, TimeRequest,
};

#[derive(Debug, Default)]
pub struct InMemoryMirageDaemon {
    state: RwLock<State>,
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
}

impl InMemoryMirageDaemon {
    pub fn new() -> Self {
        Self::default()
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
            .insert(session.name.clone(), SessionRecord { session, detail });

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
}
