//! Daemon socket RPC types — client/dashboard ↔ daemon communication.
//!
//! This module corresponds to the `socket.fbs` FlatBuffer schema
//! (`namespace mirage.socket`).  It defines:
//!
//! - **Basic RPCs** — time and health queries.
//! - **Attach** — stream I/O for a running process.
//! - **Simulator registration** — called by a simulator on connect.
//! - **Dashboard RPCs** — overview, simulator listing, profile and session
//!   CRUD, and session detail queries.
//!
//! # Dashboard RPC overview
//!
//! | RPC                        | Request                            | Reply                              |
//! |----------------------------|------------------------------------|-------------------------------------|
//! | `GetOverview`              | [`GetOverviewRequest`]             | [`GetOverviewReply`]                |
//! | `ListSimulators`           | [`ListSimulatorsRequest`]          | [`ListSimulatorsReply`]             |
//! | `GetSimulator`             | [`GetSimulatorRequest`]            | [`GetSimulatorReply`]               |
//! | `ListProfiles`             | [`ListProfilesRequest`]            | [`ListProfilesReply`]               |
//! | `CreateProfile`            | [`CreateProfileRequest`]           | [`CreateProfileReply`]              |
//! | `DeleteProfile`            | [`DeleteProfileRequest`]           | [`DeleteProfileReply`]              |
//! | `ListSessions`             | [`ListSessionsRequest`]            | [`ListSessionsReply`]               |
//! | `CreateSession` (dash)     | [`DashboardCreateSessionRequest`]  | [`DashboardCreateSessionReply`]     |
//! | `DeleteSession` (dash)     | [`DashboardDeleteSessionRequest`]  | [`DashboardDeleteSessionReply`]     |
//! | `GetSessionDetail`         | [`GetSessionDetailRequest`]        | [`GetSessionDetailReply`]           |

use serde::{Deserialize, Serialize};

use crate::common::{
    ExecArgs, GpuDef, HealthStatus, ProfileDef, RunExit, SessionDef, SimulatorMode, StreamData,
    Time,
};
use crate::simulator::SimulatorInfo;

// ===========================================================================
//  Basic RPCs — time & health
// ===========================================================================

/// Request the current simulated time for a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRequest {
    /// Session to query (optional — omit for daemon-global time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Response to [`TimeRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeReply {
    /// Current simulated time.
    pub time: Time,
}

/// Request the health of a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthRequest {
    /// Session to query (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Response to [`HealthRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReply {
    /// Whether the queried component is healthy (simplified boolean).
    #[serde(default)]
    pub healthy: bool,

    /// Structured health status.
    #[serde(default)]
    pub status: HealthStatus,
}

// ===========================================================================
//  Attach — stream I/O for a running process
// ===========================================================================

/// Attach to a running process and optionally send data to its `stdin`.
///
/// The server will read from [`stream`](Self::stream) and write to the
/// exec's `stdin`. It responds with a stream of [`AttachReply`] messages
/// carrying `stdout` / `stderr` chunks and eventually a [`RunExit`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachRequest {
    /// Identifier of the exec to attach to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_id: Option<String>,

    /// Data to send to the exec's `stdin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<StreamData>,
}

/// A message from the daemon carrying process output or an exit event.
///
/// Exactly one of [`data`](Self::data) or [`exit`](Self::exit) is set per
/// message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachReply {
    /// A chunk of stream output (stdout or stderr).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<StreamData>,

    /// Set when the process exits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<RunExit>,
}

// ===========================================================================
//  Simulator registration (simulator → daemon)
// ===========================================================================

/// Sent by a simulator process when it first connects to the daemon socket.
///
/// The simulator passes its full identity and capabilities via
/// [`SimulatorInfo`].  The daemon uses this to populate the dashboard's
/// simulator list and to match
/// [`ProfileDef::simulator`](ProfileDef::simulator) names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterSimRequest {
    /// Full simulator metadata.
    pub info: SimulatorInfo,
}

/// Daemon acknowledges the registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterSimReply {
    /// Whether the registration was accepted.
    #[serde(default)]
    pub ok: bool,

    /// If `ok` is `false`, a human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ===========================================================================
//  Dashboard RPCs — overview
// ===========================================================================

/// Request a high-level summary of the daemon state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetOverviewRequest {}

/// Response to [`GetOverviewRequest`].
///
/// Provides quick counts for the dashboard's overview panel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetOverviewReply {
    /// Total number of registered simulators.
    #[serde(default)]
    pub simulator_count: u32,

    /// Total number of configured profiles.
    #[serde(default)]
    pub profile_count: u32,

    /// Total number of active sessions.
    #[serde(default)]
    pub session_count: u32,
}

// ===========================================================================
//  Dashboard RPCs — simulators
// ===========================================================================

/// Request the list of registered simulators.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSimulatorsRequest {}

/// Aggregated simulator info for the dashboard.
///
/// This is a richer view than [`SimulatorInfo`] because it includes
/// runtime state such as [`active_session_count`](Self::active_session_count).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatorSummary {
    /// Machine-readable simulator name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// SemVer version string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// GPU models this simulator supports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_gpus: Vec<GpuDef>,

    /// Whether custom GPU topologies are supported.
    #[serde(default)]
    pub supports_custom_gpus: bool,

    /// Supported simulation modes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_modes: Vec<SimulatorMode>,

    /// Number of active sessions using this simulator.
    #[serde(default)]
    pub active_session_count: u32,
}

/// Response to [`ListSimulatorsRequest`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSimulatorsReply {
    /// All registered simulators.
    #[serde(default)]
    pub simulators: Vec<SimulatorSummary>,
}

/// Request details for a specific simulator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSimulatorRequest {
    /// Machine-readable simulator name.
    pub name: String,
}

/// Response to [`GetSimulatorRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSimulatorReply {
    /// The requested simulator summary, or `None` if not found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulator: Option<SimulatorSummary>,
}

// ===========================================================================
//  Dashboard RPCs — profiles
// ===========================================================================

/// Request the list of configured profiles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProfilesRequest {
    /// Optional: only return profiles for this simulator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulator_filter: Option<String>,
}

/// Response to [`ListProfilesRequest`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProfilesReply {
    /// Matching profiles.
    #[serde(default)]
    pub profiles: Vec<ProfileDef>,
}

/// Request to create a new profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProfileRequest {
    /// The profile definition to create.
    pub profile: ProfileDef,
}

/// Response to [`CreateProfileRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProfileReply {
    /// Whether the profile was created successfully.
    #[serde(default)]
    pub ok: bool,

    /// If `ok` is `false`, a human-readable error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Request to delete a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteProfileRequest {
    /// Name of the profile to delete.
    pub name: String,
}

/// Response to [`DeleteProfileRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteProfileReply {
    /// Whether the profile was deleted successfully.
    #[serde(default)]
    pub ok: bool,

    /// If `ok` is `false`, a human-readable error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ===========================================================================
//  Dashboard RPCs — sessions
// ===========================================================================

/// Request the list of active sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSessionsRequest {
    /// Optional: only return sessions using this profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_filter: Option<String>,
}

/// Compact session info for list views.
///
/// Unlike the full [`SessionDef`], this includes runtime state such as
/// the resolved simulator name and current health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Session name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Profile name this session uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,

    /// Simulator handling this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulator: Option<String>,

    /// Container image in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    /// Current health status.
    #[serde(default)]
    pub health_status: HealthStatus,
}

/// Response to [`ListSessionsRequest`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSessionsReply {
    /// Matching sessions.
    #[serde(default)]
    pub sessions: Vec<SessionSummary>,
}

/// Dashboard request to create a new session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardCreateSessionRequest {
    /// The session to create.
    pub session: SessionDef,
}

/// Response to [`DashboardCreateSessionRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardCreateSessionReply {
    /// Whether the session was created successfully.
    #[serde(default)]
    pub ok: bool,

    /// If `ok` is `false`, a human-readable error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Dashboard request to delete a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardDeleteSessionRequest {
    /// Name of the session to delete.
    pub name: String,
}

/// Response to [`DashboardDeleteSessionRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardDeleteSessionReply {
    /// Whether the session was deleted successfully.
    #[serde(default)]
    pub ok: bool,

    /// If `ok` is `false`, a human-readable error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ===========================================================================
//  Dashboard RPCs — session detail
// ===========================================================================

/// Request full details for a specific session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionDetailRequest {
    /// Name of the session.
    pub name: String,
}

// ===========================================================================
//  Boot / exec / shutdown RPCs
// ===========================================================================

/// Boot a new session: create the session record **and** start the
/// container via the configured container runtime.
///
/// This is the primary entry point for the `mirage-ctl boot` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootSessionRequest {
    /// The session to create and boot.
    pub session: SessionDef,
}

/// Response to [`BootSessionRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootSessionReply {
    /// Whether the session was booted successfully.
    #[serde(default)]
    pub ok: bool,

    /// If `ok` is `false`, a human-readable error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Container ID assigned by the runtime (if booted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
}

/// Execute a command inside a booted session's container.
///
/// This is the primary entry point for the `mirage-ctl exec` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecInSessionRequest {
    /// Name of the session to exec in.
    pub session_name: String,

    /// The command and arguments to run.
    pub exec: ExecArgs,
}

/// Response to [`ExecInSessionRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecInSessionReply {
    /// Process exit code.
    #[serde(default)]
    pub exit_code: i32,

    /// Captured stdout bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stdout: Vec<u8>,

    /// Captured stderr bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stderr: Vec<u8>,
}

/// Shut down a booted session: stop and remove the container, then
/// delete the session record.
///
/// This is the primary entry point for the `mirage-ctl shutdown` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownSessionRequest {
    /// Name of the session to shut down.
    pub name: String,
}

/// Response to [`ShutdownSessionRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownSessionReply {
    /// Whether the session was shut down successfully.
    #[serde(default)]
    pub ok: bool,

    /// If `ok` is `false`, a human-readable error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ===========================================================================
//  Dashboard RPCs — session detail
// ===========================================================================

/// Full session state including health and performance counters.
///
/// Returned for the dashboard's session detail view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetSessionDetailReply {
    /// Session name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// The profile governing this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProfileDef>,

    /// Simulator handling this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulator: Option<String>,

    /// Container image in use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    /// Current health status.
    #[serde(default)]
    pub health: HealthStatus,

    /// Uptime since session creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime: Option<Time>,

    /// Error message if unhealthy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    /// Total simulation ticks.
    #[serde(default)]
    pub ticks: u64,

    /// Instructions per cycle.
    #[serde(default)]
    pub ipc: f64,

    /// Ratio of simulated time to wall time.
    #[serde(default)]
    pub simulation_speed: f64,

    /// Number of active simulated contexts.
    #[serde(default)]
    pub active_contexts: u32,
}
