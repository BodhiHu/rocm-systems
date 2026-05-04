//! Simulator plugin data types.
//!
//! This module corresponds to the `simulator.fbs` FlatBuffer schema
//! (`namespace mirage.simulator`).  It contains the shared data structures
//! used by the simulator plugin system:
//!
//! - [`SimulatorInfo`] — identity and capabilities advertised on connect.
//! - [`CustomGpuDef`] — user-designed GPU topology for supported simulators.
//! - [`SessionHealth`] — health status of a running simulation session.
//! - [`SessionPerf`] — performance counters for a running session.
//!
//! The RPC request/reply message types that *carry* these structures live
//! in [`simulator_service`](crate::simulator_service).

use serde::{Deserialize, Serialize};

use crate::common::{GpuDef, HealthStatus, SimulatorMode, Time};

// ---------------------------------------------------------------------------
//  Simulator identity
// ---------------------------------------------------------------------------

/// Metadata that a simulator returns to identify itself to the daemon.
///
/// Sent in response to a `RegisterSim` request.  The daemon uses this to
/// populate the dashboard's simulator list and to match
/// [`ProfileDef::simulator`](crate::common::ProfileDef::simulator) names.
///
/// # Examples
///
/// ```
/// # use mirage_schema::simulator::SimulatorInfo;
/// # use mirage_schema::common::{GpuDef, GpuFamily};
/// let info = SimulatorInfo {
///     name: "rocjitsu".into(),
///     version: "0.4.1".into(),
///     description: Some("AMD CDNA functional simulator".into()),
///     supported_gpus: vec![GpuDef {
///         name: "MI300X".into(),
///         arch: "gfx942".into(),
///         family: GpuFamily::AmdCdna,
///         description: None,
///     }],
///     supports_custom_gpus: false,
///     supported_modes: vec![],
/// };
/// assert_eq!(info.name, "rocjitsu");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatorInfo {
    /// A unique machine-readable identifier for this simulator,
    /// e.g. `"rocjitsu"`, `"gem5-gpu"`, `"timing-model-x"`.
    ///
    /// Must be stable across versions — profiles reference this name.
    pub name: String,

    /// SemVer version string, e.g. `"0.4.1"`.
    pub version: String,

    /// Human-readable one-liner shown in the dashboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The list of GPU models this simulator can emulate out of the box.
    ///
    /// Each entry should have a unique [`GpuDef::name`].
    pub supported_gpus: Vec<GpuDef>,

    /// Whether this simulator allows the user to design completely custom
    /// GPU topologies beyond the pre-defined [`supported_gpus`](Self::supported_gpus)
    /// list.
    ///
    /// When `true`, the dashboard will show a "Design Custom GPU" button.
    #[serde(default)]
    pub supports_custom_gpus: bool,

    /// The simulation modes this simulator supports.
    ///
    /// If empty, **all** modes defined in [`SimulatorMode`] are assumed
    /// supported.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_modes: Vec<SimulatorMode>,
}

// ---------------------------------------------------------------------------
//  Custom GPU design
// ---------------------------------------------------------------------------

/// A user-designed custom GPU topology.
///
/// Simulators that set [`SimulatorInfo::supports_custom_gpus`] to `true`
/// must handle `SetCustomGpu` requests carrying this type.
///
/// The [`topology_json`](Self::topology_json) field carries a
/// simulator-specific topology description (e.g. rocjitsu's hierarchical
/// component tree) as a UTF-8 JSON string.
///
/// The simulator validates the JSON against its own schema, and if valid,
/// responds with a [`GpuDef`] that the daemon can use in profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomGpuDef {
    /// A human-readable name the user chose for this custom GPU,
    /// e.g. `"my-cdna4-256cu"`.
    pub name: String,

    /// Simulator-specific topology description encoded as a JSON string.
    ///
    /// For rocjitsu this is the `topology` subtree of a simulation config.
    /// Other simulators define their own topology format.
    pub topology_json: String,
}

// ---------------------------------------------------------------------------
//  Session health
// ---------------------------------------------------------------------------

/// Health status of a running simulation session.
///
/// Returned in a `GetSessionHealthReply`.  The daemon polls this
/// periodically and surfaces it in the dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionHealth {
    /// The session this health report is for.
    pub session_id: String,

    /// Overall health status.
    ///
    /// Simulators should return [`Healthy`](HealthStatus::Healthy) when the
    /// simulation engine is responsive and processing events.
    #[serde(default)]
    pub status: HealthStatus,

    /// Wall-clock uptime since the session was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime: Option<Time>,

    /// If status is [`Unhealthy`](HealthStatus::Unhealthy), a
    /// human-readable explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    /// Optional structured diagnostics as a JSON string.
    ///
    /// Simulators can include whatever is useful for debugging
    /// (event queue depth, thread status, memory usage, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics_json: Option<String>,
}

// ---------------------------------------------------------------------------
//  Session performance
// ---------------------------------------------------------------------------

/// Performance counters for a running simulation session.
///
/// Returned in a `GetSessionPerfReply`.  These metrics are displayed in the
/// dashboard's session detail view and can be used for auto-scaling
/// decisions in cluster mode.
///
/// # Metrics
///
/// | Field               | Description                                        |
/// |---------------------|----------------------------------------------------|
/// | `simulated_time`    | Simulated clock time elapsed.                      |
/// | `wall_time`         | Actual wall-clock time since session start.         |
/// | `ticks`             | Total simulation ticks (engine-specific unit).      |
/// | `ipc`               | Instructions per cycle.                             |
/// | `simulation_speed`  | Ratio of simulated time to wall time (>1 = faster). |
/// | `active_contexts`   | Active simulated wavefronts / threads.              |
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionPerf {
    /// The session this perf report is for.
    pub session_id: String,

    /// Simulated time elapsed inside the simulator.
    ///
    /// For cycle-accurate simulators this reflects simulated clock ticks
    /// converted to wall-clock equivalent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulated_time: Option<Time>,

    /// Actual wall-clock time elapsed since session start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_time: Option<Time>,

    /// Total simulation ticks (engine-specific unit).
    ///
    /// For rocjitsu this is simdojo discrete-event ticks.
    #[serde(default)]
    pub ticks: u64,

    /// Instructions per cycle (IPC) — a measure of how efficiently
    /// the simulated GPU is executing workloads.
    ///
    /// `0.0` if the simulator does not track IPC.
    #[serde(default)]
    pub ipc: f64,

    /// Ratio of simulated time to wall time.
    ///
    /// Values > `1.0` mean the simulator runs faster than real-time.
    /// `0.0` if not applicable.
    #[serde(default)]
    pub simulation_speed: f64,

    /// Number of active simulated wavefronts / threads / contexts.
    #[serde(default)]
    pub active_contexts: u32,

    /// Optional additional metrics as a JSON string.
    ///
    /// Simulators can expose whatever counters they have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_json: Option<String>,
}
