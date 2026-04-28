#![forbid(unsafe_code)]
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use async_trait::async_trait;
use tokio::sync::RwLock;

use mirage_container::{ContainerHandle, ContainerRuntime, StartContainerRequest};
use mirage_schema::common::{
    ExecArgs, GpuDef, GpuFamily, HealthStatus, ProfileDef, SessionPhase, SetEnv, SimulatorMode,
    Time, WorkloadDef,
};
use mirage_schema::container::{BindMount, ContainerDef};
use mirage_schema::daemon::{
    MirageDaemonAttach, MirageDaemonBoot, MirageDaemonCreateProfile, MirageDaemonCreateWorkload,
    MirageDaemonDeleteProfile, MirageDaemonDeleteWorkload, MirageDaemonExec, MirageDaemonHealth,
    MirageDaemonListProfiles, MirageDaemonListSessions, MirageDaemonListSimulators,
    MirageDaemonListWorkloads, MirageDaemonOverview, MirageDaemonRegistration, MirageDaemonResult,
    MirageDaemonShowSimulator, MirageDaemonShowWorkload, MirageDaemonShutdown, MirageDaemonStatus,
    MirageDaemonTime,
};
use mirage_schema::paths;
use mirage_schema::simulator::SimulatorInfo;
use mirage_schema::socket::{
    AttachInput, AttachOutput, AttachReply, AttachRequest, BootReply, BootRequest,
    CreateProfileReply, CreateProfileRequest, CreateWorkloadReply, CreateWorkloadRequest,
    DeleteProfileReply, DeleteProfileRequest, DeleteWorkloadReply, DeleteWorkloadRequest,
    ExecReply, ExecRequest, GetOverviewReply, GetOverviewRequest, HealthReply, HealthRequest,
    ListProfilesReply, ListProfilesRequest, ListSessionsReply, ListSessionsRequest,
    ListSimulatorsReply, ListSimulatorsRequest, ListWorkloadsReply, ListWorkloadsRequest,
    RegisterSimReply, RegisterSimRequest, SessionSummary, ShowSimulatorReply, ShowSimulatorRequest,
    ShowWorkloadReply, ShowWorkloadRequest, ShutdownReply, ShutdownRequest, SimulatorSummary,
    StatusReply, StatusRequest, TimeReply, TimeRequest, WorkloadSummary,
};
use tokio::sync::watch;

pub mod dashboard;

// ---------------------------------------------------------------------------
//  Docker label keys used to tag managed containers.
// ---------------------------------------------------------------------------

/// Every container started by Mirage carries this label.
const LABEL_MANAGED: &str = "mirage.managed";
/// Session name stored as a container label.
const LABEL_SESSION: &str = "mirage.session";
/// Profile name used by the session.
const LABEL_PROFILE: &str = "mirage.profile";
/// Container image used for the session.
const LABEL_IMAGE: &str = "mirage.image";
/// Simulator serving the session.
const LABEL_SIMULATOR: &str = "mirage.simulator";
/// Opaque emulator-session id used to correlate containers with their
/// emulator process so the daemon can detect simulator crashes.
const LABEL_EMULATOR_SESSION: &str = "mirage.emulator_session";
/// Node index within a multi-node session.
const LABEL_NODE_INDEX: &str = "mirage.node_index";

/// Default port used for head-node communication (matches NCCL/torch defaults).
const MIRAGE_HEAD_PORT: u16 = 29500;

/// Maximum number of bytes kept in the rolling output buffer per exec.
const EXEC_BUFFER_BYTES: usize = 256 * 1024;

// ---------------------------------------------------------------------------
//  In-memory exec state
// ---------------------------------------------------------------------------

/// Live in-memory state for a running or completed exec.
#[derive(Debug)]
struct ExecState {
    /// Combined stdout+stderr bytes, capped at [`EXEC_BUFFER_BYTES`].
    buffer: Vec<u8>,
    /// Whether each chunk was from stdout (true) or stderr (false).
    buffer_flags: Vec<(usize, bool)>,
    /// Sends a notification whenever the buffer grows or the exec finishes.
    /// Attached clients wait on this.
    notify: watch::Sender<()>,
    /// Exit code once the exec finishes; `None` while still running.
    exit_code: Option<i32>,
    /// stdin sender; present while the process is alive.
    stdin_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
}

// ---------------------------------------------------------------------------
//  Daemon
// ---------------------------------------------------------------------------

/// Mirage daemon that keeps minimal in-memory state.
///
/// * **Simulators** are registered in memory (they connect dynamically).
/// * **Profiles** are persisted as individual JSON files under
///   `$XDG_CONFIG_HOME/mirage/profile/`.
/// * **Sessions** are discovered by listing Docker containers that carry
///   the `mirage.session` label — no in-memory session map.
/// * **Execs** create FIFO (interactive) or regular-file (non-interactive)
///   I/O channels under `$XDG_RUNTIME_DIR/mirage/session/<session>/exec/`.
pub struct MirageDaemon {
    /// Registered simulators (in memory — they connect at runtime).
    state: Arc<RwLock<State>>,
    /// Container runtime used for Docker operations.
    container_runtime: Option<Arc<dyn ContainerRuntime>>,
    /// Monotonic exec counter used to generate unique exec ids.
    next_exec_id: AtomicU64,
    /// Root directory for on-disk configuration (profiles, workloads).
    /// Defaults to `paths::config_dir()` in production.
    config_root: PathBuf,
    /// Root directory for per-session runtime state.
    /// Defaults to `paths::runtime_dir()` in production; tests override it
    /// to isolate state between runs.
    runtime_root: PathBuf,
}

impl fmt::Debug for MirageDaemon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MirageDaemon")
            .field("state", &self.state)
            .field(
                "container_runtime",
                &self.container_runtime.as_ref().map(|_| "<runtime>"),
            )
            .field("config_root", &self.config_root)
            .finish()
    }
}

/// Minimal in-memory state.
#[derive(Debug, Default)]
struct State {
    /// Registered simulator plugins (name → info).
    simulators: BTreeMap<String, SimulatorInfo>,
    /// Maps session name → opaque emulator session id so the daemon can
    /// detect when a simulator process crashes and taint associated sessions.
    emulator_sessions: BTreeMap<String, String>,
    /// Sessions currently being booted (image pull + container start) or
    /// that finished in a failed state. Successfully booted sessions are
    /// removed from this map and discovered via the container runtime.
    pending_sessions: BTreeMap<String, PendingSession>,
    /// Sessions successfully booted by *this* daemon instance. Containers
    /// belonging to sessions not in this set were started by a previous
    /// daemon instance and are treated as [`SessionPhase::Stale`].
    booted_sessions: BTreeSet<String>,
    /// Live exec state keyed by `exec_id` (`session/<s>/exec/<n>`).
    execs: BTreeMap<String, Arc<tokio::sync::Mutex<ExecState>>>,
}

/// In-memory record describing a session that has been accepted for boot
/// but whose containers are not yet running.
#[derive(Debug, Clone)]
struct PendingSession {
    profile: String,
    simulator: String,
    image: String,
    phase: SessionPhase,
    progress_message: Option<String>,
    error_message: Option<String>,
}

// ---------------------------------------------------------------------------
//  Built-in simulators
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
//  Profile disk persistence helpers
// ---------------------------------------------------------------------------

impl MirageDaemon {
    fn profile_dir(&self) -> PathBuf {
        self.config_root.join("profile")
    }

    fn profile_path(&self, name: &str) -> PathBuf {
        self.profile_dir().join(format!("{name}.json"))
    }

    fn load_profiles_from_disk(&self) -> BTreeMap<String, ProfileDef> {
        let dir = self.profile_dir();
        let mut profiles = BTreeMap::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => return profiles,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = match std::fs::read_to_string(&path) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(?path, %e, "failed to read profile file");
                    continue;
                }
            };
            match serde_json::from_str::<ProfileDef>(&data) {
                Ok(profile) => {
                    profiles.insert(profile.name.clone(), profile);
                }
                Err(e) => {
                    tracing::warn!(?path, %e, "failed to parse profile file");
                }
            }
        }
        profiles
    }

    fn save_profile_to_disk(&self, profile: &ProfileDef) -> std::io::Result<()> {
        let dir = self.profile_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.profile_path(&profile.name);
        let json = serde_json::to_string_pretty(profile)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }

    fn delete_profile_from_disk(&self, name: &str) -> std::io::Result<()> {
        let path = self.profile_path(name);
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Block until a session started by [`boot`](MirageDaemonBoot::boot) is
    /// no longer in the pending map — i.e. either its containers are up
    /// (removed from the map) or the boot has entered the `Failed` state.
    ///
    /// Returns the final phase observed: [`SessionPhase::Running`] when the
    /// pending entry has been consumed, or [`SessionPhase::Failed`] on
    /// failure. Times out after the provided duration, returning
    /// [`SessionPhase::Pulling`]/[`SessionPhase::Starting`] in that case.
    pub async fn wait_for_boot(
        &self,
        session_name: &str,
        timeout: std::time::Duration,
    ) -> SessionPhase {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            {
                let state = self.state.read().await;
                match state.pending_sessions.get(session_name) {
                    None => return SessionPhase::Running,
                    Some(p) if p.phase == SessionPhase::Failed => {
                        return SessionPhase::Failed;
                    }
                    Some(p) => {
                        if std::time::Instant::now() >= deadline {
                            return p.phase;
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// Clear all transient daemon state: profiles on disk, known sessions
    /// in memory, and any containers owned by this daemon.
    ///
    /// Intended for use by end-to-end tests that need to reset between runs.
    pub async fn reset_for_testing(&self) {
        // Remove all profile JSON files.
        if let Ok(entries) = std::fs::read_dir(self.profile_dir()) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        // Clear in-memory state.
        {
            let mut state = self.state.write().await;
            state.emulator_sessions.clear();
            state.pending_sessions.clear();
            state.booted_sessions.clear();
            state.execs.clear();
        }
        // Tear down any containers the runtime knows about.
        if let Some(runtime) = &self.container_runtime {
            if let Ok(containers) = runtime.list_containers(&BTreeMap::new(), None).await {
                for c in containers {
                    let _ = runtime.stop_container(&c.handle, 1, None).await;
                    let _ = runtime.remove_container(&c.handle, true, None).await;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
//  Constructor helpers
// ---------------------------------------------------------------------------

impl MirageDaemon {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(State {
                simulators: builtin_simulators(),
                ..State::default()
            })),
            container_runtime: None,
            next_exec_id: AtomicU64::new(1),
            config_root: paths::config_dir(),
            runtime_root: paths::runtime_dir(),
        }
    }

    pub fn with_container_runtime(runtime: Arc<dyn ContainerRuntime>) -> Self {
        Self {
            state: Arc::new(RwLock::new(State {
                simulators: builtin_simulators(),
                ..State::default()
            })),
            container_runtime: Some(runtime),
            next_exec_id: AtomicU64::new(1),
            config_root: paths::config_dir(),
            runtime_root: paths::runtime_dir(),
        }
    }

    /// Override the configuration root directory.
    ///
    /// Useful for tests that need an isolated filesystem.
    pub fn set_config_root(&mut self, root: PathBuf) {
        self.config_root = root;
    }

    /// Generate a unique emulator session id for crash-recovery labelling.
    fn new_emulator_session_id(session_name: &str) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("emu-{session_name}-{ts}")
    }

    /// Query Docker for running containers that belong to a Mirage session.
    async fn list_session_containers(
        &self,
    ) -> MirageDaemonResult<Vec<mirage_schema::container::ListedContainer>> {
        let Some(runtime) = &self.container_runtime else {
            return Ok(vec![]);
        };
        let mut labels = BTreeMap::new();
        labels.insert(LABEL_MANAGED.to_string(), "true".to_string());
        runtime.list_containers(&labels, None).await.map_err(|e| {
            mirage_schema::daemon::MirageDaemonError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))
        })
    }

    /// Scan the per-session state directory tree on disk and return the
    /// session names found there. These correspond to sessions that at some
    /// point booted (or attempted to boot) on this host; if a name is not
    /// also represented by a running container or an in-flight pending
    /// boot, the session is considered [`SessionPhase::Stale`].
    fn list_session_state_dirs(&self) -> Vec<String> {
        let root = self.sessions_root();
        let Ok(entries) = std::fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut names = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
        names
    }

    /// Directory under which each session has its own state subdirectory.
    fn sessions_root(&self) -> PathBuf {
        self.runtime_root.join("session")
    }

    /// Per-session state directory.
    fn session_dir(&self, session: &str) -> PathBuf {
        self.sessions_root().join(session)
    }

    /// Build a path-format exec id: `session/<session>/exec/<n>`.
    fn make_exec_id(session: &str, n: u64) -> String {
        format!("session/{session}/exec/{n}")
    }

    fn simulator_summary(_state: &State, info: &SimulatorInfo, active: u32) -> SimulatorSummary {
        SimulatorSummary {
            name: Some(info.name.clone()),
            version: Some(info.version.clone()),
            description: info.description.clone(),
            supported_gpus: info.supported_gpus.clone(),
            supports_custom_gpus: info.supports_custom_gpus,
            supported_modes: info.supported_modes.clone(),
            active_session_count: active,
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
            args: parts[1..]
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
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
            execs: execs
                .iter()
                .map(|value| Self::parse_csv_exec(value))
                .collect(),
            cleanup,
        }
    }

    /// Build the standard set of Docker labels for a session container.
    fn session_labels(
        session_name: &str,
        profile_name: &str,
        simulator: &str,
        image: &str,
        emulator_session: &str,
        node_index: u32,
    ) -> BTreeMap<String, String> {
        let mut labels = BTreeMap::new();
        labels.insert(LABEL_MANAGED.to_string(), "true".to_string());
        labels.insert(LABEL_SESSION.to_string(), session_name.to_string());
        labels.insert(LABEL_PROFILE.to_string(), profile_name.to_string());
        labels.insert(LABEL_SIMULATOR.to_string(), simulator.to_string());
        labels.insert(LABEL_IMAGE.to_string(), image.to_string());
        labels.insert(
            LABEL_EMULATOR_SESSION.to_string(),
            emulator_session.to_string(),
        );
        labels.insert(LABEL_NODE_INDEX.to_string(), node_index.to_string());
        labels
    }
}

// ---------------------------------------------------------------------------
//  Trait implementations — health, time, attach
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonHealth for MirageDaemon {
    async fn health(&self, request: HealthRequest) -> MirageDaemonResult<HealthReply> {
        if let Some(session_id) = request.session {
            // Check Docker for the session container.
            let containers = self.list_session_containers().await?;
            let found = containers.iter().any(|c| {
                c.labels
                    .get(LABEL_SESSION)
                    .map_or(false, |s| *s == session_id)
            });
            if !found {
                return Err(mirage_schema::daemon::MirageDaemonError::Remote(format!(
                    "session '{session_id}' does not exist",
                )));
            }
        }
        Ok(HealthReply {
            healthy: true,
            status: HealthStatus::Healthy,
        })
    }
}

#[async_trait]
impl MirageDaemonTime for MirageDaemon {
    async fn time(&self, request: TimeRequest) -> MirageDaemonResult<TimeReply> {
        if let Some(session_id) = request.session {
            let containers = self.list_session_containers().await?;
            let found = containers.iter().any(|c| {
                c.labels
                    .get(LABEL_SESSION)
                    .map_or(false, |s| *s == session_id)
            });
            if !found {
                return Err(mirage_schema::daemon::MirageDaemonError::Remote(format!(
                    "session '{session_id}' does not exist",
                )));
            }
        }
        Ok(TimeReply {
            time: Time::default(),
        })
    }
}

#[async_trait]
impl MirageDaemonAttach for MirageDaemon {
    async fn attach(
        &self,
        request: AttachRequest,
        mut input: tokio::sync::mpsc::Receiver<AttachInput>,
        output: tokio::sync::mpsc::Sender<AttachOutput>,
    ) -> MirageDaemonResult<AttachReply> {
        let exec_state = {
            let state = self.state.read().await;
            state.execs.get(&request.exec_id).cloned()
        };
        let Some(exec_state) = exec_state else {
            let _ = output
                .send(AttachOutput {
                    is_stdout: false,
                    output: format!("attach: unknown exec id '{}'\n", request.exec_id).into_bytes(),
                })
                .await;
            return Ok(AttachReply { exit_code: -1 });
        };

        // Forward client stdin to the running process.
        let stdin_tx = {
            let es = exec_state.lock().await;
            es.stdin_tx.clone()
        };
        let input_task = tokio::spawn(async move {
            while let Some(frame) = input.recv().await {
                let Some(ref tx) = stdin_tx else { break };
                if tx.send(frame.stream).await.is_err() {
                    break;
                }
            }
        });

        // Stream buffered output + live output to the caller.
        let mut notify_rx = {
            let es = exec_state.lock().await;
            es.notify.subscribe()
        };
        let mut offset: usize = 0;
        loop {
            // Send any buffered output we haven't sent yet.
            let (chunks, done, exit_code) = {
                let es = exec_state.lock().await;
                // Collect (data, is_stdout) pairs for bytes at positions >= offset.
                let mut pos = 0usize;
                let mut chunks: Vec<(Vec<u8>, bool)> = Vec::new();
                for &(len, is_stdout) in &es.buffer_flags {
                    let end = pos + len;
                    if end > offset {
                        let start_in_chunk = if pos < offset { offset - pos } else { 0 };
                        let slice = es.buffer[pos + start_in_chunk..end].to_vec();
                        chunks.push((slice, is_stdout));
                    }
                    pos = end;
                }
                offset = es.buffer.len();
                (chunks, es.exit_code.is_some(), es.exit_code)
            };
            for (data, is_stdout) in chunks {
                if output
                    .send(AttachOutput {
                        is_stdout,
                        output: data,
                    })
                    .await
                    .is_err()
                {
                    input_task.abort();
                    return Ok(AttachReply { exit_code: -1 });
                }
            }
            if done {
                input_task.abort();
                return Ok(AttachReply {
                    exit_code: exit_code.unwrap_or(-1),
                });
            }
            // Wait for more output or process exit.
            if notify_rx.changed().await.is_err() {
                break;
            }
        }
        input_task.abort();
        let exit_code = exec_state.lock().await.exit_code.unwrap_or(-1);
        Ok(AttachReply { exit_code })
    }
}

// ---------------------------------------------------------------------------
//  Simulator registration
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonRegistration for MirageDaemon {
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

// ---------------------------------------------------------------------------
//  Overview
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonOverview for MirageDaemon {
    async fn get_overview(
        &self,
        _request: GetOverviewRequest,
    ) -> MirageDaemonResult<GetOverviewReply> {
        let state = self.state.read().await;
        let profiles = self.load_profiles_from_disk();
        let containers = self.list_session_containers().await?;
        // Count unique sessions from containers plus any pending sessions
        // that do not yet have a backing container.
        let mut session_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &containers {
            if let Some(name) = c.labels.get(LABEL_SESSION) {
                session_names.insert(name.clone());
            }
        }
        for name in state.pending_sessions.keys() {
            session_names.insert(name.clone());
        }
        for name in self.list_session_state_dirs() {
            session_names.insert(name);
        }
        Ok(GetOverviewReply {
            simulator_count: state.simulators.len() as u32,
            profile_count: profiles.len() as u32,
            session_count: session_names.len() as u32,
        })
    }
}

// ---------------------------------------------------------------------------
//  Simulators
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonListSimulators for MirageDaemon {
    async fn list_simulators(
        &self,
        _request: ListSimulatorsRequest,
    ) -> MirageDaemonResult<ListSimulatorsReply> {
        let state = self.state.read().await;
        let containers = self.list_session_containers().await.unwrap_or_default();
        let simulators = state
            .simulators
            .values()
            .map(|info| {
                let active = containers
                    .iter()
                    .filter(|c| {
                        c.labels
                            .get(LABEL_SIMULATOR)
                            .map_or(false, |s| *s == info.name)
                    })
                    .count() as u32;
                Self::simulator_summary(&state, info, active)
            })
            .collect();
        Ok(ListSimulatorsReply { simulators })
    }
}

#[async_trait]
impl MirageDaemonShowSimulator for MirageDaemon {
    async fn show_simulator(
        &self,
        request: ShowSimulatorRequest,
    ) -> MirageDaemonResult<ShowSimulatorReply> {
        let state = self.state.read().await;
        let containers = self.list_session_containers().await.unwrap_or_default();
        let simulator = state.simulators.get(&request.name).map(|info| {
            let active = containers
                .iter()
                .filter(|c| {
                    c.labels
                        .get(LABEL_SIMULATOR)
                        .map_or(false, |s| *s == info.name)
                })
                .count() as u32;
            Self::simulator_summary(&state, info, active)
        });
        Ok(ShowSimulatorReply { simulator })
    }
}

// ---------------------------------------------------------------------------
//  Profiles — stored to disk
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonListProfiles for MirageDaemon {
    async fn list_profiles(
        &self,
        request: ListProfilesRequest,
    ) -> MirageDaemonResult<ListProfilesReply> {
        let profiles = self.load_profiles_from_disk();
        let profiles = profiles
            .into_values()
            .filter(|profile| {
                request
                    .simulator
                    .as_ref()
                    .is_none_or(|filter| profile.simulator == *filter)
            })
            .collect();
        Ok(ListProfilesReply { profiles })
    }
}

#[async_trait]
impl MirageDaemonCreateProfile for MirageDaemon {
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

        // Check if profile already exists on disk.
        let existing = self.load_profiles_from_disk();
        if existing.contains_key(&profile.name) {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some(format!("profile '{}' already exists", profile.name)),
            });
        }

        let state = self.state.read().await;
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

        if let Err(e) = self.save_profile_to_disk(&profile) {
            return Ok(CreateProfileReply {
                ok: false,
                error: Some(format!("failed to persist profile: {e}")),
            });
        }

        Ok(CreateProfileReply {
            ok: true,
            error: None,
        })
    }
}

#[async_trait]
impl MirageDaemonDeleteProfile for MirageDaemon {
    async fn delete_profile(
        &self,
        request: DeleteProfileRequest,
    ) -> MirageDaemonResult<DeleteProfileReply> {
        let existing = self.load_profiles_from_disk();
        if !existing.contains_key(&request.name) {
            return Ok(DeleteProfileReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", request.name)),
            });
        }

        // Check if any running session references this profile.
        let containers = self.list_session_containers().await?;
        let in_use = containers.iter().any(|c| {
            c.labels
                .get(LABEL_PROFILE)
                .map_or(false, |p| *p == request.name)
        });
        if in_use {
            return Ok(DeleteProfileReply {
                ok: false,
                error: Some(format!(
                    "profile '{}' is still referenced by an active session",
                    request.name
                )),
            });
        }

        if let Err(e) = self.delete_profile_from_disk(&request.name) {
            return Ok(DeleteProfileReply {
                ok: false,
                error: Some(format!("failed to delete profile: {e}")),
            });
        }

        Ok(DeleteProfileReply {
            ok: true,
            error: None,
        })
    }
}

// ---------------------------------------------------------------------------
//  Sessions — discovered from Docker
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonListSessions for MirageDaemon {
    async fn list_sessions(
        &self,
        request: ListSessionsRequest,
    ) -> MirageDaemonResult<ListSessionsReply> {
        let containers = self.list_session_containers().await?;
        // Group by session name, pick the first container for summary info.
        let mut seen: BTreeMap<String, SessionSummary> = BTreeMap::new();
        let booted = {
            let state = self.state.read().await;
            state.booted_sessions.clone()
        };
        for c in &containers {
            let Some(session_name) = c.labels.get(LABEL_SESSION) else {
                continue;
            };
            if let Some(filter) = &request.profile {
                if c.labels.get(LABEL_PROFILE).map_or(true, |p| p != filter) {
                    continue;
                }
            }
            // Containers belonging to a session not booted by this daemon
            // instance are stale — the new daemon has no in-memory state for
            // them and cannot serve exec/attach reliably.
            let phase = if booted.contains(session_name) {
                SessionPhase::Running
            } else {
                SessionPhase::Stale
            };
            seen.entry(session_name.clone())
                .or_insert_with(|| SessionSummary {
                    name: Some(session_name.clone()),
                    profile: c.labels.get(LABEL_PROFILE).cloned(),
                    simulator: c.labels.get(LABEL_SIMULATOR).cloned(),
                    image: c.labels.get(LABEL_IMAGE).cloned(),
                    health_status: if phase == SessionPhase::Running {
                        HealthStatus::Healthy
                    } else {
                        HealthStatus::Unknown
                    },
                    phase,
                    progress_message: if phase == SessionPhase::Stale {
                        Some("session containers running but not registered with this daemon instance; shut down and re-boot to resume".to_string())
                    } else {
                        None
                    },
                });
        }
        // Merge pending sessions (still booting or boot-failed) that have
        // no backing container yet.
        {
            let state = self.state.read().await;
            for (name, pending) in &state.pending_sessions {
                if let Some(filter) = &request.profile {
                    if pending.profile != *filter {
                        continue;
                    }
                }
                seen.entry(name.clone()).or_insert_with(|| SessionSummary {
                    name: Some(name.clone()),
                    profile: Some(pending.profile.clone()),
                    simulator: Some(pending.simulator.clone()),
                    image: Some(pending.image.clone()),
                    health_status: if pending.phase == SessionPhase::Failed {
                        HealthStatus::Unhealthy
                    } else {
                        HealthStatus::Unknown
                    },
                    phase: pending.phase,
                    progress_message: pending.progress_message.clone(),
                });
            }
        }
        // Surface stale sessions: state directories left over on disk for
        // which no container is running and no boot is in flight. These
        // typically come from a prior daemon crash or a killed docker
        // process; the caller can clean them up via `shutdown`.
        if request.profile.is_none() {
            for name in self.list_session_state_dirs() {
                seen.entry(name.clone()).or_insert_with(|| SessionSummary {
                    name: Some(name),
                    profile: None,
                    simulator: None,
                    image: None,
                    health_status: HealthStatus::Unknown,
                    phase: SessionPhase::Stale,
                    progress_message: Some(
                        "state directory present but no running container".to_string(),
                    ),
                });
            }
        }
        Ok(ListSessionsReply {
            sessions: seen.into_values().collect(),
        })
    }
}

#[async_trait]
impl MirageDaemonStatus for MirageDaemon {
    async fn status(&self, request: StatusRequest) -> MirageDaemonResult<StatusReply> {
        // Check pending sessions first — they have no backing container yet.
        {
            let state = self.state.read().await;
            if let Some(pending) = state.pending_sessions.get(&request.name) {
                let profile = self.load_profiles_from_disk().remove(&pending.profile);
                return Ok(StatusReply {
                    name: Some(request.name),
                    profile,
                    simulator: Some(pending.simulator.clone()),
                    image: Some(pending.image.clone()),
                    health: if pending.phase == SessionPhase::Failed {
                        HealthStatus::Unhealthy
                    } else {
                        HealthStatus::Unknown
                    },
                    uptime: None,
                    error_message: pending.error_message.clone(),
                    ticks: 0,
                    ipc: 0.0,
                    simulation_speed: 0.0,
                    active_contexts: 0,
                    phase: pending.phase,
                    progress_message: pending.progress_message.clone(),
                });
            }
        }

        let containers = self.list_session_containers().await?;
        let session_containers: Vec<_> = containers
            .iter()
            .filter(|c| {
                c.labels
                    .get(LABEL_SESSION)
                    .map_or(false, |s| *s == request.name)
            })
            .collect();

        if session_containers.is_empty() {
            // Fall back to checking for a stale state directory on disk.
            if self.session_dir(&request.name).is_dir() {
                return Ok(StatusReply {
                    name: Some(request.name),
                    profile: None,
                    simulator: None,
                    image: None,
                    health: HealthStatus::Unknown,
                    uptime: None,
                    error_message: None,
                    ticks: 0,
                    ipc: 0.0,
                    simulation_speed: 0.0,
                    active_contexts: 0,
                    phase: SessionPhase::Stale,
                    progress_message: Some(
                        "state directory present but no running container".to_string(),
                    ),
                });
            }
            return Err(mirage_schema::daemon::MirageDaemonError::Remote(format!(
                "session '{}' does not exist",
                request.name
            )));
        }

        let first = &session_containers[0];
        let profile_name = first.labels.get(LABEL_PROFILE).cloned();
        let profile = profile_name.and_then(|n| self.load_profiles_from_disk().remove(&n));

        // If the containers are running but weren't booted by this daemon
        // instance, report the session as Stale.
        let is_booted = {
            let state = self.state.read().await;
            state.booted_sessions.contains(&request.name)
        };
        if !is_booted {
            return Ok(StatusReply {
                name: Some(request.name),
                profile,
                simulator: first.labels.get(LABEL_SIMULATOR).cloned(),
                image: first.labels.get(LABEL_IMAGE).cloned(),
                health: HealthStatus::Unknown,
                uptime: None,
                error_message: None,
                ticks: 0,
                ipc: 0.0,
                simulation_speed: 0.0,
                active_contexts: 0,
                phase: SessionPhase::Stale,
                progress_message: Some(
                    "session containers running but not registered with this daemon instance; shut down and re-boot to resume".to_string(),
                ),
            });
        }

        Ok(StatusReply {
            name: Some(request.name),
            profile,
            simulator: first.labels.get(LABEL_SIMULATOR).cloned(),
            image: first.labels.get(LABEL_IMAGE).cloned(),
            health: HealthStatus::Healthy,
            uptime: None,
            error_message: None,
            ticks: 0,
            ipc: 0.0,
            simulation_speed: 0.0,
            active_contexts: 0,
            phase: SessionPhase::Running,
            progress_message: None,
        })
    }
}

// ---------------------------------------------------------------------------
//  Boot
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonBoot for MirageDaemon {
    async fn boot(&self, request: BootRequest) -> MirageDaemonResult<BootReply> {
        let session_name = request.name;
        let profile_name = request.profile;
        let image = request.image;
        let extra_volumes = request.volumes;

        if session_name.trim().is_empty() {
            return Ok(BootReply {
                ok: false,
                error: Some("session name must not be empty".to_string()),
                container_id: None,
                container_ids: vec![],
            });
        }

        // Validate extra volume syntax up-front so the caller gets a
        // synchronous error rather than a failed async boot.
        for vol in &extra_volumes {
            let parts: Vec<&str> = vol.splitn(3, ':').collect();
            if parts.len() < 2 {
                return Ok(BootReply {
                    ok: false,
                    error: Some(format!(
                        "invalid volume spec '{vol}': expected host:container[:ro]"
                    )),
                    container_id: None,
                    container_ids: vec![],
                });
            }
        }

        // Reject duplicates — check both running containers and any pending
        // (not-yet-started or failed) session bookkeeping.
        let containers = self.list_session_containers().await?;
        let already_running = containers.iter().any(|c| {
            c.labels
                .get(LABEL_SESSION)
                .map_or(false, |s| *s == session_name)
        });
        if already_running {
            return Ok(BootReply {
                ok: false,
                error: Some(format!("session '{session_name}' already exists")),
                container_id: None,
                container_ids: vec![],
            });
        }
        {
            let state = self.state.read().await;
            if state.pending_sessions.contains_key(&session_name) {
                return Ok(BootReply {
                    ok: false,
                    error: Some(format!("session '{session_name}' already exists")),
                    container_id: None,
                    container_ids: vec![],
                });
            }
        }

        let profiles = self.load_profiles_from_disk();
        let Some(profile) = profiles.get(&profile_name).cloned() else {
            return Ok(BootReply {
                ok: false,
                error: Some(format!("profile '{profile_name}' does not exist")),
                container_id: None,
                container_ids: vec![],
            });
        };

        {
            let state = self.state.read().await;
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
        }

        let Some(runtime) = self.container_runtime.clone() else {
            return Ok(BootReply {
                ok: false,
                error: Some("no container runtime configured".to_string()),
                container_id: None,
                container_ids: vec![],
            });
        };

        // Mark the session as pending so list_sessions/status surface it
        // while the background boot task runs.
        {
            let mut state = self.state.write().await;
            state.pending_sessions.insert(
                session_name.clone(),
                PendingSession {
                    profile: profile_name.clone(),
                    simulator: profile.simulator.clone(),
                    image: image.clone(),
                    phase: SessionPhase::Pulling,
                    progress_message: Some(format!("pulling image {image}")),
                    error_message: None,
                },
            );
        }

        // Create the session runtime directory up-front so exec can write
        // into it without racing the boot task.
        let _ = std::fs::create_dir_all(self.session_dir(&session_name));

        // Spawn the long-running boot work (image pull, container start).
        let state = self.state.clone();
        let boot_session = session_name.clone();
        let boot_profile_name = profile_name.clone();
        let boot_image = image.clone();
        tokio::spawn(async move {
            let result = boot_session_task(
                state.clone(),
                runtime,
                boot_session.clone(),
                boot_profile_name,
                profile,
                boot_image,
                extra_volumes,
            )
            .await;
            match result {
                Ok(emulator_session_id) => {
                    let mut st = state.write().await;
                    st.emulator_sessions
                        .insert(boot_session.clone(), emulator_session_id);
                    // On success, drop the pending entry; the session is now
                    // discoverable via the container runtime.
                    st.pending_sessions.remove(&boot_session);
                    st.booted_sessions.insert(boot_session.clone());
                }
                Err(message) => {
                    let mut st = state.write().await;
                    if let Some(p) = st.pending_sessions.get_mut(&boot_session) {
                        p.phase = SessionPhase::Failed;
                        p.progress_message = None;
                        p.error_message = Some(message);
                    }
                }
            }
        });

        // Return immediately; the dashboard polls list_sessions/status to
        // observe the boot progressing.
        Ok(BootReply {
            ok: true,
            error: None,
            container_id: None,
            container_ids: vec![],
        })
    }
}

/// Perform the actual boot work: image pull, container start, optional
/// network creation. Runs as a background task spawned by `boot()`.
async fn boot_session_task(
    state: Arc<RwLock<State>>,
    runtime: Arc<dyn ContainerRuntime>,
    session_name: String,
    profile_name: String,
    profile: ProfileDef,
    image: String,
    extra_volumes: Vec<String>,
) -> Result<String, String> {
    let emulator_session_id = MirageDaemon::new_emulator_session_id(&session_name);

    // --- Emulator + interceptor setup ---
    let interceptor_so = find_interceptor_so();
    let mut base_mounts = Vec::new();
    let mut base_env = vec![SetEnv {
        key: "MIRAGE_SESSION".to_string(),
        value: session_name.clone(),
    }];
    let devices = Vec::new();

    if let Some(ref so_path) = interceptor_so {
        if mirage_real::RealEmulator::hardware_available() {
            if let Ok(Some(real)) = mirage_real::RealEmulator::detect() {
                use mirage_schema::topology::ProvideTopology;
                let topology = real.get_topology().ok();

                let socket_path = unique_emulator_socket(&session_name);
                let server = mirage_remote::EmulatorServer::new(socket_path.clone(), real);
                let listener = server
                    .bind()
                    .map_err(|e| format!("failed to bind emulator socket: {e}"))?;
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

                if let Some(ref topo) = topology {
                    if let Ok(topo_dir) = create_synthetic_topology(&session_name, topo) {
                        base_mounts.push(BindMount {
                            host_path: topo_dir.join("sys/class/kfd").to_string_lossy().to_string(),
                            container_path: "/sys/class/kfd".to_string(),
                            readonly: true,
                        });
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
                    value: container_so,
                });
                base_env.push(SetEnv {
                    key: "MIRAGE_INTERCEPTOR_SOCKET".to_string(),
                    value: container_sock,
                });
            }
        }
    }

    // --- Image pull with progress streaming ---
    let (progress_tx, mut progress_rx) =
        mirage_schema::container::container_runtime_progress_channel();
    {
        // Forward the latest status message into the pending session record
        // so the dashboard can surface pull progress.
        let state = state.clone();
        let session_name = session_name.clone();
        tokio::spawn(async move {
            while let Some(event) = progress_rx.recv().await {
                use mirage_schema::container::ContainerRuntimeEvent;
                let message = match event {
                    ContainerRuntimeEvent::Status { message, .. } => Some(message),
                    ContainerRuntimeEvent::Stdout { chunk, .. }
                    | ContainerRuntimeEvent::Stderr { chunk, .. } => String::from_utf8(chunk)
                        .ok()
                        .and_then(|s| s.lines().last().map(|line| line.trim().to_string())),
                };
                if let Some(message) = message
                    && !message.is_empty()
                {
                    let mut st = state.write().await;
                    if let Some(p) = st.pending_sessions.get_mut(&session_name) {
                        p.progress_message = Some(message);
                    }
                }
            }
        });
    }

    if let Err(e) = runtime.pull_image(&image, Some(progress_tx)).await {
        // Pull errors are non-fatal for locally-cached images; surface as a
        // warning by keeping progress, but continue the boot.
        tracing::warn!(%e, "image pull returned error; continuing");
    }

    // Transition to "starting" phase.
    {
        let mut st = state.write().await;
        if let Some(p) = st.pending_sessions.get_mut(&session_name) {
            p.phase = SessionPhase::Starting;
            p.progress_message = Some("starting containers".to_string());
        }
    }

    // Append user-supplied extra volumes (syntax was validated by boot()).
    for vol in &extra_volumes {
        let parts: Vec<&str> = vol.splitn(3, ':').collect();
        let readonly = parts.get(2).map_or(false, |opt| *opt == "ro");
        base_mounts.push(BindMount {
            host_path: parts[0].to_string(),
            container_path: parts[1].to_string(),
            readonly,
        });
    }

    let num_nodes = profile.num_nodes.max(1);
    let multi_node = num_nodes > 1;
    let head_container_name = if multi_node {
        format!("mirage-{session_name}-node0")
    } else {
        format!("mirage-{session_name}")
    };

    let network_name = if multi_node {
        let name = format!("mirage-{session_name}");
        runtime
            .create_network(&name, None)
            .await
            .map_err(|e| format!("failed to create network: {e}"))?;
        Some(name)
    } else {
        None
    };

    let mut container_handles: Vec<ContainerHandle> = Vec::with_capacity(num_nodes as usize);

    for node_index in 0..num_nodes {
        let container_name = if multi_node {
            format!("mirage-{session_name}-node{node_index}")
        } else {
            format!("mirage-{session_name}")
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

        let labels = MirageDaemon::session_labels(
            &session_name,
            &profile_name,
            &profile.simulator,
            &image,
            &emulator_session_id,
            node_index,
        );

        let container_def = ContainerDef {
            image: image.clone(),
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
            labels,
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
                return Err(format!("failed to start container: {err}"));
            }
        };

        container_handles.push(started.inspection.handle);
    }

    Ok(emulator_session_id)
}

// ---------------------------------------------------------------------------
//  Exec — spawns immediately, buffers output in memory
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonExec for MirageDaemon {
    async fn exec(&self, request: ExecRequest) -> MirageDaemonResult<ExecReply> {
        // Discover the session's containers from Docker and select the one
        // matching the requested node index (defaults to the head node).
        let containers = self.list_session_containers().await?;
        let target_node = request.node_index;
        let target = containers
            .iter()
            .filter(|c| {
                c.labels
                    .get(LABEL_SESSION)
                    .map_or(false, |s| *s == request.session)
            })
            .find(|c| {
                c.labels
                    .get(LABEL_NODE_INDEX)
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(0)
                    == target_node
            });

        let Some(head) = target else {
            return Err(mirage_schema::daemon::MirageDaemonError::Remote(format!(
                "session '{}' has no node {}",
                request.session, target_node
            )));
        };

        // Build the container exec args.
        let mut exec_args = Self::exec_from_command(request.command)?;

        // Inject interceptor env vars if present in the container's labels.
        if let Some(so_path) = head.labels.get("mirage.interceptor_path") {
            exec_args.env.push(SetEnv {
                key: "LD_PRELOAD".to_string(),
                value: so_path.clone(),
            });
        }
        if let Some(sock_path) = head.labels.get("mirage.emulator_socket") {
            exec_args.env.push(SetEnv {
                key: "MIRAGE_INTERCEPTOR_SOCKET".to_string(),
                value: sock_path.clone(),
            });
        }

        // Build exec_id path.
        let n = self.next_exec_id.fetch_add(1, Ordering::Relaxed);
        let exec_id = Self::make_exec_id(&request.session, n);

        // Create the in-memory exec state.
        let (notify_tx, _) = watch::channel(());
        let (stdin_tx, stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
        let exec_state = Arc::new(tokio::sync::Mutex::new(ExecState {
            buffer: Vec::new(),
            buffer_flags: Vec::new(),
            notify: notify_tx,
            exit_code: None,
            stdin_tx: Some(stdin_tx),
        }));
        {
            let mut state = self.state.write().await;
            state.execs.insert(exec_id.clone(), exec_state.clone());
        }

        let container_id = head.handle.id.clone();
        let (command, exec_env) = {
            let ExecArgs {
                command, args, env, ..
            } = exec_args;
            let mut v = vec![command];
            v.extend(args);
            (v, env)
        };

        // Spawn the docker exec process and pump output into the buffer.
        let eid = exec_id.clone();
        let state_clone = self.state.clone();
        tokio::spawn(async move {
            use std::fs::File;
            use std::process::Stdio;
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            use tokio::process::Command;

            // Allocate a host-side PTY so docker exec sees a real TTY on its
            // stdin/stdout.  This lets `-it` work and gives the container
            // process proper echo and prompt behaviour.
            let pty = match nix::pty::openpty(
                Some(&nix::pty::Winsize {
                    ws_row: 24,
                    ws_col: 80,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                }),
                None,
            ) {
                Ok(p) => p,
                Err(e) => {
                    let msg = format!("exec: openpty failed: {e}\n");
                    let mut es = exec_state.lock().await;
                    append_to_exec_buffer(&mut es, msg.as_bytes(), false);
                    es.stdin_tx = None;
                    es.exit_code = Some(-1);
                    let _ = es.notify.send(());
                    return;
                }
            };

            // Convert OwnedFds to std::fs::File (safe, RAII-managed).
            let master_file = File::from(pty.master);
            let slave_file = File::from(pty.slave);

            // Clone master so reader and writer have independent file objects
            // (a single tokio::fs::File can only run one spawn_blocking at a
            // time, so sharing via io::split would deadlock).
            let master_write_file = match master_file.try_clone() {
                Ok(f) => f,
                Err(e) => {
                    let msg = format!("exec: dup master failed: {e}\n");
                    let mut es = exec_state.lock().await;
                    append_to_exec_buffer(&mut es, msg.as_bytes(), false);
                    es.stdin_tx = None;
                    es.exit_code = Some(-1);
                    let _ = es.notify.send(());
                    return;
                }
            };

            // Clone slave for stdout and stderr: docker exec needs a TTY on
            // all three streams so the shell sees a proper controlling terminal.
            let (slave_stdout, slave_stderr) = match slave_file
                .try_clone()
                .and_then(|o| slave_file.try_clone().map(|e| (o, e)))
            {
                Ok(pair) => pair,
                Err(e) => {
                    let msg = format!("exec: dup slave failed: {e}\n");
                    let mut es = exec_state.lock().await;
                    append_to_exec_buffer(&mut es, msg.as_bytes(), false);
                    es.stdin_tx = None;
                    es.exit_code = Some(-1);
                    let _ = es.notify.send(());
                    return;
                }
            };

            // Build the docker exec command.
            let mut cmd = Command::new("docker");
            cmd.arg("exec").arg("-it").arg(&container_id);
            for env_arg in &exec_env {
                cmd.arg("-e");
                cmd.arg(format!("{}={}", env_arg.key, env_arg.value));
            }
            for part in &command {
                cmd.arg(part);
            }

            // Give docker the slave PTY as its stdin/stdout/stderr so it sees
            // a real TTY.  Stdio::from(File) is safe and has the OS set up the
            // fds in the child; the slave Files are consumed (and closed in the
            // parent) when Command::spawn() returns.
            cmd.stdin(Stdio::from(slave_file));
            cmd.stdout(Stdio::from(slave_stdout));
            cmd.stderr(Stdio::from(slave_stderr));

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("exec: failed to spawn docker exec: {e}\n");
                    let mut es = exec_state.lock().await;
                    append_to_exec_buffer(&mut es, msg.as_bytes(), false);
                    es.stdin_tx = None;
                    es.exit_code = Some(-1);
                    let _ = es.notify.send(());
                    return;
                }
            };

            // Wrap master halves for async I/O.
            let mut master_read = tokio::fs::File::from_std(master_file);
            let mut master_write = tokio::fs::File::from_std(master_write_file);

            // Pump stdin_rx → PTY master (→ slave stdin → docker → container).
            let stdin_task = tokio::spawn(async move {
                let mut rx = stdin_rx;
                while let Some(bytes) = rx.recv().await {
                    if master_write.write_all(&bytes).await.is_err() {
                        break;
                    }
                    let _ = master_write.flush().await;
                }
            });

            // Pump PTY master output → exec buffer (→ attach WebSocket → browser).
            let es_out = exec_state.clone();
            let stdout_task = tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    match master_read.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut es = es_out.lock().await;
                            append_to_exec_buffer(&mut es, &buf[..n], true);
                            let _ = es.notify.send(());
                        }
                    }
                }
            });

            let status = child.wait().await;
            stdin_task.abort();
            let _ = stdout_task.await;

            let code = match status {
                Ok(s) => s.code().unwrap_or(-1),
                Err(_) => -1,
            };
            {
                let mut es = exec_state.lock().await;
                es.stdin_tx = None;
                es.exit_code = Some(code);
                let _ = es.notify.send(());
            }
            // Remove the exec state from the global map after a short delay
            // so late-arriving attach calls can still read the final exit code.
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            let mut st = state_clone.write().await;
            st.execs.remove(&eid);
        });

        Ok(ExecReply { exec_id })
    }
}

/// Append bytes to an exec's rolling output buffer, capping at
/// [`EXEC_BUFFER_BYTES`]. Merges consecutive chunks from the same stream.
fn append_to_exec_buffer(es: &mut ExecState, data: &[u8], is_stdout: bool) {
    // Trim from the front if over budget.
    let incoming = data.len();
    let current = es.buffer.len();
    if current + incoming > EXEC_BUFFER_BYTES {
        let drop = (current + incoming) - EXEC_BUFFER_BYTES;
        es.buffer.drain(..drop);
        // Fix up buffer_flags to reflect the drain.
        let mut removed = 0usize;
        let mut keep_from = 0usize;
        for (i, &(len, _)) in es.buffer_flags.iter().enumerate() {
            if removed + len <= drop {
                removed += len;
                keep_from = i + 1;
            } else {
                // Partial chunk — shrink its length.
                es.buffer_flags[i].0 -= drop - removed;
                break;
            }
        }
        es.buffer_flags.drain(..keep_from);
    }
    es.buffer.extend_from_slice(data);
    // Merge with the last flag entry if it's the same stream.
    if let Some(last) = es.buffer_flags.last_mut() {
        if last.1 == is_stdout {
            last.0 += incoming;
            return;
        }
    }
    es.buffer_flags.push((incoming, is_stdout));
}

// ---------------------------------------------------------------------------
//  Shutdown
// ---------------------------------------------------------------------------

#[async_trait]
impl MirageDaemonShutdown for MirageDaemon {
    async fn shutdown(&self, request: ShutdownRequest) -> MirageDaemonResult<ShutdownReply> {
        // Search for any container still labeled with this session name,
        // regardless of whether it is running, exited, or missing the
        // `managed=true` label. This is broader than `list_session_containers`
        // so stale cleanup can reap orphans left behind by a crashed daemon.
        let session_containers: Vec<_> = if let Some(runtime) = &self.container_runtime {
            let mut filter = BTreeMap::new();
            filter.insert(LABEL_SESSION.to_string(), request.name.clone());
            runtime.list_containers(&filter, None).await.map_err(|e| {
                mirage_schema::daemon::MirageDaemonError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?
        } else {
            Vec::new()
        };

        // A session may be pending (still booting or boot-failed) and have
        // no containers yet — drop it from the pending map so the caller can
        // clean up a stuck or failed boot.
        let was_pending = {
            let mut state = self.state.write().await;
            state.pending_sessions.remove(&request.name).is_some()
        };

        // A session may also be stale: no containers, not in the pending
        // map, but a leftover state directory is sitting on disk. Removing
        // the directory (and any orphan network) is the cleanup for that
        // case.
        let session_dir = self.session_dir(&request.name);
        let was_stale = session_dir.is_dir();

        if session_containers.is_empty() && !was_pending && !was_stale {
            return Ok(ShutdownReply {
                ok: false,
                error: Some(format!("session '{}' does not exist", request.name)),
            });
        }

        if let Some(runtime) = &self.container_runtime {
            for c in &session_containers {
                let _ = runtime.stop_container(&c.handle, 10, None).await;
                let _ = runtime.remove_container(&c.handle, true, None).await;
            }
            // Always attempt to remove the session network. Multi-node
            // sessions create `mirage-<name>`; the call is idempotent for
            // single-node or already-cleaned-up sessions.
            let net_name = format!("mirage-{}", request.name);
            let _ = runtime.remove_network(&net_name, None).await;
        }

        // Remove emulator session mapping.
        {
            let mut state = self.state.write().await;
            state.emulator_sessions.remove(&request.name);
            state.booted_sessions.remove(&request.name);
        }

        // Clean up session runtime directory (exec FIFOs, metadata, logs).
        let _ = std::fs::remove_dir_all(&session_dir);

        Ok(ShutdownReply {
            ok: true,
            error: None,
        })
    }
}

// ---------------------------------------------------------------------------
//  Workloads — stored to disk alongside profiles
// ---------------------------------------------------------------------------

impl MirageDaemon {
    fn workload_dir(&self) -> PathBuf {
        self.config_root.join("workload")
    }

    fn workload_path(&self, name: &str) -> PathBuf {
        self.workload_dir().join(format!("{name}.json"))
    }

    fn load_workloads_from_disk(&self) -> BTreeMap<String, WorkloadDef> {
        let dir = self.workload_dir();
        let mut workloads = BTreeMap::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => return workloads,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = match std::fs::read_to_string(&path) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(?path, %e, "failed to read workload file");
                    continue;
                }
            };
            match serde_json::from_str::<WorkloadDef>(&data) {
                Ok(workload) => {
                    workloads.insert(workload.name.clone(), workload);
                }
                Err(e) => {
                    tracing::warn!(?path, %e, "failed to parse workload file");
                }
            }
        }
        workloads
    }

    fn save_workload_to_disk(&self, workload: &WorkloadDef) -> std::io::Result<()> {
        let dir = self.workload_dir();
        std::fs::create_dir_all(&dir)?;
        let path = self.workload_path(&workload.name);
        let json = serde_json::to_string_pretty(workload)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }

    fn delete_workload_from_disk(&self, name: &str) -> std::io::Result<()> {
        let path = self.workload_path(name);
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl MirageDaemonCreateWorkload for MirageDaemon {
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

        let existing = self.load_workloads_from_disk();
        if existing.contains_key(&workload.name) {
            return Ok(CreateWorkloadReply {
                ok: false,
                error: Some(format!("workload '{}' already exists", workload.name)),
            });
        }

        // Validate the referenced profile exists on disk.
        let profiles = self.load_profiles_from_disk();
        if !profiles.contains_key(&workload.profile) {
            return Ok(CreateWorkloadReply {
                ok: false,
                error: Some(format!("profile '{}' does not exist", workload.profile)),
            });
        }

        if let Err(e) = self.save_workload_to_disk(&workload) {
            return Ok(CreateWorkloadReply {
                ok: false,
                error: Some(format!("failed to persist workload: {e}")),
            });
        }

        Ok(CreateWorkloadReply {
            ok: true,
            error: None,
        })
    }
}

#[async_trait]
impl MirageDaemonListWorkloads for MirageDaemon {
    async fn list_workloads(
        &self,
        _request: ListWorkloadsRequest,
    ) -> MirageDaemonResult<ListWorkloadsReply> {
        let workloads = self.load_workloads_from_disk();
        let workloads = workloads
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
impl MirageDaemonShowWorkload for MirageDaemon {
    async fn show_workload(
        &self,
        request: ShowWorkloadRequest,
    ) -> MirageDaemonResult<ShowWorkloadReply> {
        let workloads = self.load_workloads_from_disk();
        let workload = workloads.get(&request.name).cloned();
        Ok(ShowWorkloadReply { workload })
    }
}

#[async_trait]
impl MirageDaemonDeleteWorkload for MirageDaemon {
    async fn delete_workload(
        &self,
        request: DeleteWorkloadRequest,
    ) -> MirageDaemonResult<DeleteWorkloadReply> {
        let existing = self.load_workloads_from_disk();
        if !existing.contains_key(&request.name) {
            return Ok(DeleteWorkloadReply {
                ok: false,
                error: Some(format!("workload '{}' does not exist", request.name)),
            });
        }

        if let Err(e) = self.delete_workload_from_disk(&request.name) {
            return Ok(DeleteWorkloadReply {
                ok: false,
                error: Some(format!("failed to delete workload: {e}")),
            });
        }

        Ok(DeleteWorkloadReply {
            ok: true,
            error: None,
        })
    }
}

// ---------------------------------------------------------------------------
//  Free-standing helpers
// ---------------------------------------------------------------------------

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
/// bind-mounted into a container.
fn create_synthetic_topology(
    session_name: &str,
    topology: &mirage_schema::topology::Topology,
) -> std::io::Result<PathBuf> {
    let root = std::env::temp_dir()
        .join("mirage")
        .join(format!("topo-{session_name}"));

    let topo_base = root.join("sys/class/kfd/kfd/topology");

    let mut render_minors: Vec<u32> = Vec::new();
    for (rel_path, data) in &topology.files {
        let dst = topo_base.join(rel_path);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dst, data)?;

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

    let dri = root.join("dev/dri");
    std::fs::create_dir_all(&dri)?;
    for minor in &render_minors {
        std::fs::write(dri.join(format!("renderD{minor}")), b"")?;
    }

    let dev = root.join("dev");
    std::fs::create_dir_all(&dev)?;
    std::fs::write(dev.join("kfd"), b"")?;

    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_schema::common::SessionDef;

    use std::sync::atomic::AtomicU64;

    /// Monotonic counter to give each test daemon a unique temp directory.
    static TEST_ID: AtomicU64 = AtomicU64::new(1);

    fn test_config_root() -> PathBuf {
        let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir()
            .join("mirage-test")
            .join(format!("{pid}-{id}"));
        // Ensure a clean slate even if this directory existed from a prior run.
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Per-daemon runtime root so state directories don't leak between
    /// tests and trigger spurious stale-session detections.
    fn test_runtime_root() -> PathBuf {
        let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir()
            .join("mirage-test-runtime")
            .join(format!("{pid}-{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    impl MirageDaemon {
        fn new_test() -> Self {
            Self {
                state: Arc::new(RwLock::new(State {
                    simulators: builtin_simulators(),
                    ..State::default()
                })),
                container_runtime: None,
                next_exec_id: AtomicU64::new(1),
                config_root: test_config_root(),
                runtime_root: test_runtime_root(),
            }
        }

        fn with_container_runtime_test(runtime: Arc<dyn ContainerRuntime>) -> Self {
            Self {
                state: Arc::new(RwLock::new(State {
                    simulators: builtin_simulators(),
                    ..State::default()
                })),
                container_runtime: Some(runtime),
                next_exec_id: AtomicU64::new(1),
                config_root: test_config_root(),
                runtime_root: test_runtime_root(),
            }
        }
    }

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

    fn boot_request(session: SessionDef) -> BootRequest {
        BootRequest {
            name: session.name,
            profile: session.profile,
            image: session.image,
            volumes: vec![],
        }
    }

    /// Test helper: call boot, wait for the background boot task to
    /// finish, and assert that the session reached `Running`.
    async fn boot_and_wait(daemon: &MirageDaemon, req: BootRequest) -> BootReply {
        let name = req.name.clone();
        let reply = daemon.boot(req).await.unwrap();
        assert!(reply.ok, "boot should succeed: {:?}", reply.error);
        let phase = daemon
            .wait_for_boot(&name, std::time::Duration::from_secs(5))
            .await;
        assert_eq!(phase, SessionPhase::Running, "boot did not reach Running");
        reply
    }

    /// Look up the head-node container handle for a session via the
    /// container runtime.
    async fn handle_for_session(
        mock: &mirage_container::MockContainerRuntime,
        session: &str,
    ) -> mirage_container::ContainerHandle {
        let mut labels = BTreeMap::new();
        labels.insert(LABEL_SESSION.to_string(), session.to_string());
        let containers = mock
            .list_containers(&labels, None)
            .await
            .expect("list_containers should succeed");
        containers
            .first()
            .expect("session should have at least one container")
            .handle
            .clone()
    }

    fn exec_request(session_name: impl Into<String>, exec: ExecArgs) -> ExecRequest {
        let mut command = vec![exec.command];
        command.extend(exec.args);
        ExecRequest {
            session: session_name.into(),
            node_index: 0,
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
        let daemon = MirageDaemon::new_test();

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
        assert!(
            reply.ok,
            "profile creation should succeed: {:?}",
            reply.error
        );
    }

    #[tokio::test]
    async fn boots_and_lists_sessions() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;

        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "session-a".to_string(),
                profile: "mi300x".to_string(),
                image: "ghcr.io/example/image:latest".to_string(),
            }),
        )
        .await;

        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 1);
        assert_eq!(sessions.sessions[0].name.as_deref(), Some("session-a"));
    }

    async fn daemon_with_mock_runtime()
    -> (MirageDaemon, Arc<mirage_container::MockContainerRuntime>) {
        let mock = Arc::new(mirage_container::MockContainerRuntime::default());
        let daemon = MirageDaemon::with_container_runtime_test(mock.clone());

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

        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "vllm-test".to_string(),
                profile: "mi300x".to_string(),
                image: "ghcr.io/rocm/vllm:latest".to_string(),
            }),
        )
        .await;

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
        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "exec-test".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
            }),
        )
        .await;

        // Queue a mock exec result.
        let handle = handle_for_session(&mock, "exec-test").await;
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

        assert!(!reply.exec_id.is_empty());
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
        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "shutdown-test".to_string(),
                profile: "mi300x".to_string(),
                image: "img:latest".to_string(),
            }),
        )
        .await;

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
    async fn stale_session_on_disk_is_listed_as_stale() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;
        // Simulate a crashed daemon: the state directory exists on disk
        // but no container is running and no boot is in flight.
        let stale_dir = daemon.session_dir("abandoned");
        std::fs::create_dir_all(&stale_dir).unwrap();

        let reply = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        let stale = reply
            .sessions
            .iter()
            .find(|s| s.name.as_deref() == Some("abandoned"))
            .expect("abandoned session should be listed");
        assert_eq!(stale.phase, SessionPhase::Stale);
    }

    #[tokio::test]
    async fn stale_session_status_reports_stale_phase() {
        let (daemon, _mock) = daemon_with_mock_runtime().await;
        let stale_dir = daemon.session_dir("ghost");
        std::fs::create_dir_all(&stale_dir).unwrap();

        let reply = daemon
            .status(StatusRequest {
                name: "ghost".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(reply.phase, SessionPhase::Stale);
    }

    #[tokio::test]
    async fn shutdown_removes_stale_session_state_directory() {
        let (daemon, mock) = daemon_with_mock_runtime().await;
        let stale_dir = daemon.session_dir("leftover");
        std::fs::create_dir_all(&stale_dir).unwrap();
        // Also plant a file inside and a leftover network, simulating a
        // multi-node session whose daemon died mid-run.
        std::fs::write(stale_dir.join("exec-meta.json"), b"{}").unwrap();
        mock.create_network("mirage-leftover", None).await.unwrap();
        assert!(stale_dir.is_dir());
        assert!(
            mock.networks().await.iter().any(|n| n == "mirage-leftover"),
            "network should exist before cleanup"
        );

        let reply = daemon
            .shutdown(ShutdownRequest {
                name: "leftover".to_string(),
            })
            .await
            .unwrap();
        assert!(
            reply.ok,
            "shutdown should clean up stale session: {:?}",
            reply.error
        );
        assert!(!stale_dir.exists(), "state directory should be removed");
        assert!(
            !mock.networks().await.iter().any(|n| n == "mirage-leftover"),
            "orphan network should be removed"
        );

        // A second shutdown on the same (now gone) session should fail.
        let reply2 = daemon
            .shutdown(ShutdownRequest {
                name: "leftover".to_string(),
            })
            .await
            .unwrap();
        assert!(!reply2.ok);
    }

    #[tokio::test]
    async fn boot_exec_shutdown_e2e() {
        let (daemon, mock) = daemon_with_mock_runtime().await;

        // 1. Boot a vLLM session.
        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "vllm-e2e".to_string(),
                profile: "mi300x".to_string(),
                image: "ghcr.io/rocm/vllm:latest".to_string(),
            }),
        )
        .await;

        // 2. Exec a health check.
        let handle = handle_for_session(&mock, "vllm-e2e").await;
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
        assert!(!exec_reply.exec_id.is_empty());

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

    async fn daemon_with_multinode_profile()
    -> (MirageDaemon, Arc<mirage_container::MockContainerRuntime>) {
        let mock = Arc::new(mirage_container::MockContainerRuntime::default());
        let daemon = MirageDaemon::with_container_runtime_test(mock.clone());

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

        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "multi-test".to_string(),
                profile: "mi300x-2node".to_string(),
                image: "ghcr.io/rocm/vllm:latest".to_string(),
            }),
        )
        .await;

        // Verify two start_container calls were made.
        let starts = mock.start_requests().await;
        assert_eq!(starts.len(), 2, "should have 2 containers");
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

        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "multi-shutdown".to_string(),
                profile: "mi300x-2node".to_string(),
                image: "img:latest".to_string(),
            }),
        )
        .await;

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

    async fn daemon_with_profile() -> MirageDaemon {
        let daemon = MirageDaemon::new_test();
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
        let daemon = MirageDaemon::new_test();

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

        let r2 = daemon
            .create_workload(create_workload_request(workload))
            .await
            .unwrap();
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
        let daemon = MirageDaemon::new_test();

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

    const MNIST_IMAGE: &str =
        "docker.io/rocm/pytorch:rocm6.4_ubuntu24.04_py3.12_pytorch_release_2.6.0";

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

    async fn mnist_daemon_with_mock() -> (MirageDaemon, Arc<mirage_container::MockContainerRuntime>)
    {
        let mock = Arc::new(mirage_container::MockContainerRuntime::default());
        let daemon = MirageDaemon::with_container_runtime_test(mock.clone());

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
        daemon: &MirageDaemon,
        mock: &Arc<mirage_container::MockContainerRuntime>,
    ) -> (String, mirage_container::ContainerHandle) {
        boot_and_wait(
            daemon,
            boot_request(SessionDef {
                name: "mnist-test".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }),
        )
        .await;

        let handle = handle_for_session(mock, "mnist-test").await;
        (handle.id.clone(), handle)
    }

    #[tokio::test]
    async fn mnist_profile_creation_with_single_gpu() {
        let daemon = MirageDaemon::new_test();

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
        let daemon = MirageDaemon::new_test();

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

        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "mnist-boot".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }),
        )
        .await;

        let sessions = daemon
            .list_sessions(ListSessionsRequest::default())
            .await
            .unwrap();
        assert_eq!(sessions.sessions.len(), 1);
        assert_eq!(sessions.sessions[0].name.as_deref(), Some("mnist-boot"));
        assert_eq!(sessions.sessions[0].image.as_deref(), Some(MNIST_IMAGE));

        let starts = mock.start_requests().await;
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].container.image, MNIST_IMAGE);
    }

    #[tokio::test]
    async fn mnist_boot_rejects_duplicate_session_name() {
        let (daemon, _mock) = mnist_daemon_with_mock().await;

        let boot1 = daemon
            .boot(boot_request(SessionDef {
                name: "mnist-dup".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot1.ok);

        let boot2 = daemon
            .boot(boot_request(SessionDef {
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
            .exec(exec_request(
                "mnist-test",
                mnist_exec(
                    "python",
                    &["-c", "import sys; print(f'Python {sys.version}')"],
                ),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
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
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["-c", "import torch, json; print(json.dumps({'pytorch_version': torch.__version__}))"]),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
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
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["-c", "import torchvision; print('OK')"]),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
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
                stdout: format!(
                    "Training...\n--- RESULT ---\n{result_json}\nMNIST training test passed.\n"
                )
                .into_bytes(),
                stderr: vec![],
            },
        )
        .await;

        let reply = daemon
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["-c", "# MNIST training code"]),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
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
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["-c", "# training fails"]),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
    }

    #[tokio::test]
    async fn mnist_full_e2e_boot_train_shutdown() {
        let (daemon, mock) = mnist_daemon_with_mock().await;

        // 1. Boot session.
        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "mnist-e2e".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }),
        )
        .await;
        let handle = handle_for_session(&mock, "mnist-e2e").await;

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
            .exec(exec_request(
                "mnist-e2e",
                mnist_exec("python", &["--version"]),
            ))
            .await
            .unwrap();
        assert!(!r.exec_id.is_empty());

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
            .exec(exec_request(
                "mnist-e2e",
                mnist_exec("python", &["-c", "import torch; print('ok')"]),
            ))
            .await
            .unwrap();
        assert!(!r.exec_id.is_empty());

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
            .exec(exec_request(
                "mnist-e2e",
                mnist_exec("python", &["/workspace/mnist_train.py", "--epochs", "2"]),
            ))
            .await
            .unwrap();
        assert!(!r.exec_id.is_empty());

        // 5. Shutdown.
        let shutdown = daemon
            .shutdown(ShutdownRequest {
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
            .boot(boot_request(SessionDef {
                name: "mnist-detail".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok);

        let detail = daemon
            .status(StatusRequest {
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
        let daemon = MirageDaemon::new_test();
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
        assert!(
            reply.ok,
            "workload create should succeed: {:?}",
            reply.error
        );

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
        let daemon = MirageDaemon::new_test();
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
                    mnist_exec(
                        "python",
                        &["-c", "import torch; assert torch.cuda.is_available()"],
                    ),
                    mnist_exec("python", &["-c", "import torchvision; print('OK')"]),
                    mnist_exec("python", &["/workspace/mnist_train.py", "--epochs", "2"]),
                ],
                cleanup: mirage_schema::common::CleanupPolicy::OnSuccess,
            }))
            .await
            .unwrap();
        assert!(reply.ok);

        let workload = daemon
            .show_workload(ShowWorkloadRequest {
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
        let daemon = MirageDaemon::new_test();
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
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["-c", "import torchvision"]),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
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
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["train.py", "--batch-size", "65536"]),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
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
            (
                "python train.py",
                b"Training complete. Accuracy: 97.5%\n".to_vec(),
            ),
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

        for (i, (_desc, _expected_output)) in steps.iter().enumerate() {
            let reply = daemon
                .exec(exec_request(
                    "mnist-test",
                    mnist_exec("python", &["-c", &format!("step {i}")]),
                ))
                .await
                .unwrap();
            assert!(!reply.exec_id.is_empty(), "step {i} should return exec_id");
        }

        // The non-interactive execs are dispatched via tokio::spawn, so we
        // need to give the background tasks a chance to run.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

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
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["train.py"]),
            ))
            .await
            .unwrap();
        assert!(!reply.exec_id.is_empty());

        // Shutdown should still succeed.
        let shutdown = daemon
            .shutdown(ShutdownRequest {
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
        let daemon = MirageDaemon::new_test();

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
        let daemon = MirageDaemon::new_test();

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
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["train.py"]),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
    }

    #[tokio::test]
    async fn mnist_container_image_passed_correctly() {
        let (daemon, mock) = mnist_daemon_with_mock().await;

        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "mnist-image-check".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }),
        )
        .await;

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

        boot_and_wait(
            &daemon,
            boot_request(SessionDef {
                name: "mnist-overview".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }),
        )
        .await;

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
            .exec(exec_request(
                "mnist-test",
                mnist_exec("python", &["mnist_train.py"]),
            ))
            .await
            .unwrap();

        assert!(!reply.exec_id.is_empty());
    }

    #[tokio::test]
    async fn mnist_exec_on_nonexistent_session_fails() {
        let (daemon, _mock) = mnist_daemon_with_mock().await;

        let result = daemon
            .exec(exec_request(
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
            .boot(boot_request(SessionDef {
                name: "mnist-dbl-shut".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot.ok);

        let r1 = daemon
            .shutdown(ShutdownRequest {
                name: "mnist-dbl-shut".to_string(),
            })
            .await
            .unwrap();
        assert!(r1.ok);

        let r2 = daemon
            .shutdown(ShutdownRequest {
                name: "mnist-dbl-shut".to_string(),
            })
            .await
            .unwrap();
        assert!(!r2.ok, "second shutdown should fail");
    }

    #[tokio::test]
    async fn mnist_workload_accepts_any_valid_image() {
        let daemon = MirageDaemon::new_test();
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
            .show_workload(ShowWorkloadRequest {
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
            .boot(boot_request(SessionDef {
                name: "mnist-a".to_string(),
                profile: "mi300x-mnist".to_string(),
                image: MNIST_IMAGE.to_string(),
            }))
            .await
            .unwrap();
        assert!(boot_a.ok);

        let boot_b = daemon
            .boot(boot_request(SessionDef {
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
            .shutdown(ShutdownRequest {
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
        let daemon = MirageDaemon::new_test();

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
            rocjitsu.supported_gpus.iter().any(|g| g.name == "MI300X"),
            "MI300X should be supported"
        );
        assert!(
            rocjitsu.supported_gpus.iter().any(|g| g.name == "MI350X"),
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
