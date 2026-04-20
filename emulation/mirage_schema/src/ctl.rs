//! Documented source of truth for the Mirage control protocol.
//!
//! The `ctl` module describes the daemon RPC surface once and uses
//! [`crate::ctl_dsl!`] to generate:
//!
//! - request and reply types,
//! - endpoint traits for daemon implementations,
//! - a transport-facing request/input/output/reply registry,
//! - and the public CLI parser used by `mirage_ctl`.
//!
//! Keeping the protocol description here avoids repeating the same endpoint
//! table in `socket.rs`, `daemon.rs`, and the CLI binary.

use std::fmt;
use std::io;

use crate::common::{
    CleanupPolicy, GpuDef, HealthStatus, ProfileDef, SimulatorMode, Time, WorkloadDef,
};
use crate::simulator::SimulatorInfo;

/// Result type used by daemon implementations and ctl transports.
pub type MirageDaemonResult<T> = Result<T, MirageDaemonError>;

/// Error raised while transporting or serving daemon control requests.
#[derive(Debug)]
pub enum MirageDaemonError {
    /// The underlying Unix-socket or filesystem operation failed.
    Io(io::Error),
    /// A JSON frame could not be encoded or decoded.
    Serialization(serde_json::Error),
    /// The peer violated the ctl framing or message contract.
    Protocol(String),
    /// The daemon rejected the request with a human-readable message.
    Remote(String),
}

impl MirageDaemonError {
    /// Constructs a protocol-level ctl error.
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol(message.into())
    }
}

impl fmt::Display for MirageDaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Serialization(error) => write!(f, "serialization error: {error}"),
            Self::Protocol(message) => write!(f, "protocol error: {message}"),
            Self::Remote(message) => write!(f, "remote error: {message}"),
        }
    }
}

impl std::error::Error for MirageDaemonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::Protocol(_) | Self::Remote(_) => None,
        }
    }
}

impl From<io::Error> for MirageDaemonError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for MirageDaemonError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value)
    }
}

/// Dashboard-facing simulator information enriched with daemon runtime state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SimulatorSummary {
    /// Machine-readable simulator name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Simulator version string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Human-readable simulator description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// GPU models the simulator can emulate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_gpus: Vec<GpuDef>,
    /// Whether the simulator accepts custom GPU definitions.
    #[serde(default)]
    pub supports_custom_gpus: bool,
    /// Supported simulation modes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_modes: Vec<SimulatorMode>,
    /// Number of currently active sessions using this simulator.
    #[serde(default)]
    pub active_session_count: u32,
}

/// Compact session information used by list views.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionSummary {
    /// Session name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Profile assigned to the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Simulator currently serving the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulator: Option<String>,
    /// Container image in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Current health state for the session.
    #[serde(default)]
    pub health_status: HealthStatus,
}

/// Compact workload information used by list views.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkloadSummary {
    /// Workload name.
    pub name: String,
    /// Profile used to boot the temporary session.
    pub profile: String,
    /// Container image used by the workload.
    pub image: String,
    /// Whether the workload has a startup command.
    #[serde(default)]
    pub has_startup: bool,
    /// Number of exec steps in the workload.
    #[serde(default)]
    pub exec_count: u32,
    /// Cleanup policy for the temporary session.
    #[serde(default)]
    pub cleanup: CleanupPolicy,
}

crate::ctl_dsl! {
#[ctl(
    cli_name = "mirage-ctl",
    error = crate::ctl::MirageDaemonError,
    protocol_error = crate::ctl::MirageDaemonError::protocol
)]
pub mod daemon {
    /// Query the daemon or a single session for its current health state.
    health({
        /// Optional session name to scope the health lookup to.
        session_id: String = None,
    }) -> {
        /// Whether the queried target is considered healthy.
        healthy: bool,
        /// Structured health status for the target.
        status: HealthStatus,
    };

    /// Query the daemon or a single session for its current simulated time.
    time({
        /// Optional session name to scope the time lookup to.
        session_id: String = None,
    }) -> {
        /// Current simulated time for the requested scope.
        time: Time,
    };

    /// Attach to a running exec and stream bytes to its standard input.
    ///
    /// The request identifies the exec, stdin bytes are sent as [`AttachInput`]
    /// messages, streamed stdout and stderr are returned as [`AttachOutput`]
    /// messages, and the final exit status arrives in [`AttachReply`].
    attach({
        /// Identifier of the exec to attach to.
        exec_id: String,
    }) ... {
        /// Raw bytes to forward to the exec's standard input.
        stream: Vec<u8>,
    } -> {
        /// Whether the emitted chunk came from stdout (`true`) or stderr (`false`).
        is_stdout: bool,
        /// Raw bytes emitted by the exec.
        output: Vec<u8>,
    } => {
        /// Final process exit code reported after the exec terminates.
        exit_code: i32,
    };

    /// Register a simulator process with the daemon.
    ///
    /// This endpoint is internal to simulator startup and is intentionally not
    /// exposed on the end-user CLI.
    #[ctl(skip_cli)]
    register_sim({
        /// Full simulator identity and capability description.
        info: SimulatorInfo,
    }) -> {
        /// Whether the simulator registration succeeded.
        ok: bool,
        /// Human-readable registration error when `ok` is `false`.
        error: String = None,
    };

    /// Retrieve top-level daemon counts for dashboards and health probes.
    get_overview({}) -> {
        /// Number of registered simulators.
        simulator_count: u32,
        /// Number of stored profiles.
        profile_count: u32,
        /// Number of active sessions.
        session_count: u32,
    };

    /// List all simulators currently registered with the daemon.
    list_simulators({}) -> {
        /// Registered simulators and their runtime summaries.
        simulators: Vec<SimulatorSummary>,
    };

    /// Fetch the summary for one simulator by name.
    show_simulator({
        /// Machine-readable simulator name.
        name: String,
    }) -> {
        /// Matching simulator summary, if one is registered.
        simulator: SimulatorSummary = None,
    };

    /// List stored profiles, optionally filtered to a simulator.
    list_profiles({
        /// Optional simulator name used to filter the profile list.
        simulator: String = None,
    }) -> {
        /// Matching profiles.
        profiles: Vec<ProfileDef>,
    };

    /// Create a new reusable profile definition.
    create_profile({
        /// Unique name for the profile.
        name: String,
        /// Simulator that should serve sessions created from this profile.
        simulator: String,
        /// GPU model name supported by the selected simulator.
        gpu: String,
        /// Simulation mode to use for this profile.
        #[arg(long, value_enum)]
        mode: SimulatorMode,
        /// Number of GPUs per node.
        #[arg(long = "gpus-per-node")]
        gpus_per_node: u32,
        /// Number of nodes in the session topology.
        nodes: u32,
    }) -> {
        /// Whether the profile was created successfully.
        ok: bool,
        /// Human-readable validation or creation error when `ok` is `false`.
        error: String = None,
    };

    /// Delete a stored profile by name.
    delete_profile({
        /// Profile name to delete.
        name: String,
    }) -> {
        /// Whether the profile was deleted successfully.
        ok: bool,
        /// Human-readable deletion error when `ok` is `false`.
        error: String = None,
    };

    /// List active sessions, optionally filtered to a single profile.
    list_sessions({
        /// Optional profile name used to filter the session list.
        profile: String = None,
    }) -> {
        /// Matching session summaries.
        sessions: Vec<SessionSummary>,
    };

    /// Fetch the full dashboard detail view for one session.
    status({
        /// Session name to inspect.
        name: String,
    }) -> {
        /// Session name.
        name: String = None,
        /// Full profile associated with the session.
        profile: ProfileDef = None,
        /// Simulator currently serving the session.
        simulator: String = None,
        /// Container image associated with the session.
        image: String = None,
        /// Current health state.
        health: HealthStatus,
        /// Session uptime, when known.
        uptime: Time = None,
        /// Failure message, when the session is unhealthy.
        error_message: String = None,
        /// Total simulated ticks completed so far.
        ticks: u64,
        /// Instructions per cycle.
        ipc: f64,
        /// Ratio of simulated time to wall-clock time.
        simulation_speed: f64,
        /// Number of active simulated contexts.
        active_contexts: u32,
    };

    /// Create a session record and boot its backing containers.
    boot({
        /// Unique session name.
        name: String,
        /// Profile used for the session.
        profile: String,
        /// Container image to boot.
        image: String,
        /// Extra bind-mount volumes (`host:container[:ro]`).
        #[arg(long = "volume", short = 'v')]
        volumes: Vec<String>,
    }) -> {
        /// Whether the session boot completed successfully.
        ok: bool,
        /// Human-readable boot error when `ok` is `false`.
        error: String = None,
        /// Head-node container ID when the boot succeeds.
        container_id: String = None,
        /// All container IDs for the booted session.
        container_ids: Vec<String>,
    };

    /// Run one command inside an already booted session.
    exec({
        /// Session that should execute the command.
        session_name: String,
        /// Program and arguments to run inside the session container.
        ///
        /// Pass the command after `--`, for example:
        /// `mirage-ctl exec-in-session --session-name demo -- python -c 'print(1)'`.
        #[arg(last = true, required = true)]
        command: Vec<String>,
    }) -> {
        /// Process exit code.
        exit_code: i32,
        /// Captured stdout bytes.
        stdout: Vec<u8>,
        /// Captured stderr bytes.
        stderr: Vec<u8>,
    };

    /// Stop a booted session and clean up its backing resources.
    shutdown({
        /// Session name to shut down.
        name: String,
    }) -> {
        /// Whether the session shut down successfully.
        ok: bool,
        /// Human-readable shutdown error when `ok` is `false`.
        error: String = None,
    };

    /// Persist a workload definition for later execution.
    create_workload({
        /// Unique workload name.
        name: String,
        /// Profile used to boot the workload's temporary session.
        profile: String,
        /// Container image used for the workload's temporary session.
        image: String,
        /// Optional startup command encoded as `command,arg1,arg2,...`.
        startup: String = None,
        /// Repeated exec steps encoded as `command,arg1,arg2,...`.
        #[arg(long = "exec", required = true)]
        execs: Vec<String>,
        /// Cleanup policy applied after the workload finishes.
        #[arg(long, value_enum, default_value_t = CleanupPolicy::Always)]
        cleanup: CleanupPolicy,
    }) -> {
        /// Whether the workload definition was created successfully.
        ok: bool,
        /// Human-readable creation error when `ok` is `false`.
        error: String = None,
    };

    /// List stored workload definitions.
    list_workloads({}) -> {
        /// Stored workload summaries.
        workloads: Vec<WorkloadSummary>,
    };

    /// Fetch one stored workload definition by name.
    show_workload({
        /// Workload name to fetch.
        name: String,
    }) -> {
        /// Matching workload definition, if one exists.
        workload: WorkloadDef = None,
    };

    /// Delete a stored workload definition by name.
    delete_workload({
        /// Workload name to delete.
        name: String,
    }) -> {
        /// Whether the workload definition was deleted successfully.
        ok: bool,
        /// Human-readable deletion error when `ok` is `false`.
        error: String = None,
    };
}
}
