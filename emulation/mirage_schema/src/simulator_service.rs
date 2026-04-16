//! Simulator RPC request / reply message types.
//!
//! This module corresponds to the `simulator_service.fbs` FlatBuffer schema.
//! It defines the request and reply types for the **daemon → simulator** RPC
//! service.
//!
//! The data structures *carried* inside these messages live in
//! [`common`](crate::common), [`container`](crate::container), and
//! [`simulator`](crate::simulator).
//!
//! # RPC overview
//!
//! | RPC                | Request                        | Reply                         |
//! |--------------------|--------------------------------|-------------------------------|
//! | `GetSupportedGpus` | [`GetSupportedGpusRequest`]    | [`GetSupportedGpusReply`]     |
//! | `SetCustomGpu`     | [`SetCustomGpuRequest`]        | [`SetCustomGpuReply`]         |
//! | `CreateSession`    | [`CreateSessionRequest`]       | [`CreateSessionReply`]        |
//! | `DeleteSession`    | [`DeleteSessionRequest`]       | [`DeleteSessionReply`]        |
//! | `GetSessionHealth` | [`GetSessionHealthRequest`]    | [`GetSessionHealthReply`]     |
//! | `GetSessionPerf`   | [`GetSessionPerfRequest`]      | [`GetSessionPerfReply`]       |
//! | `GetRunDef`        | [`GetRunDefRequest`]           | [`GetRunDefReply`]            |

use serde::{Deserialize, Serialize};

use crate::common::{ExecDef, GpuDef, ProfileDef, RunDef, SessionDef};
use crate::container::ContainerDef;
use crate::simulator::{CustomGpuDef, SessionHealth, SessionPerf};

// ===========================================================================
//  GetSupportedGpus
// ===========================================================================

/// Request the list of GPU models this simulator supports.
///
/// This is a convenience RPC for refreshing GPU availability without
/// re-querying the full [`SimulatorInfo`](crate::simulator::SimulatorInfo).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSupportedGpusRequest {}

/// Response to [`GetSupportedGpusRequest`].
///
/// Contains the complete list of [`GpuDef`]s the simulator can emulate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSupportedGpusReply {
    /// All GPU models this simulator currently supports.
    pub gpus: Vec<GpuDef>,
}

// ===========================================================================
//  SetCustomGpu
// ===========================================================================

/// Register a user-designed custom GPU topology.
///
/// Only sent to simulators that advertised
/// [`supports_custom_gpus = true`](crate::simulator::SimulatorInfo::supports_custom_gpus).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetCustomGpuRequest {
    /// The custom GPU definition to register.
    pub gpu: CustomGpuDef,
}

/// Response to [`SetCustomGpuRequest`].
///
/// Returns the newly created [`GpuDef`] on success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetCustomGpuReply {
    /// The GPU definition created from the custom topology.
    pub gpu: GpuDef,
}

// ===========================================================================
//  CreateSession
// ===========================================================================

/// Create a new simulation session and return the container spec.
///
/// The simulator should:
///
/// 1. Generate any configuration files needed inside the container
///    (e.g. JSON simulation configs, FlatBuffers schemas).
/// 2. Determine which host libraries need to be bind-mounted.
/// 3. Compute the environment variables required for interposition
///    (e.g. `LD_PRELOAD`).
/// 4. Return a [`ContainerDef`] that the daemon can hand directly to an
///    OCI runtime.
///
/// The simulator may allocate internal bookkeeping state keyed by
/// [`session.name`](SessionDef::name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    /// The session to create.
    pub session: SessionDef,
    /// The profile that governs simulator, GPU, mode, and cluster shape.
    pub profile: ProfileDef,
}

/// Response to [`CreateSessionRequest`].
///
/// Returns the full container specification for the OCI runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSessionReply {
    /// The OCI container definition the daemon should launch.
    pub container: ContainerDef,
}

// ===========================================================================
//  DeleteSession
// ===========================================================================

/// Tear down a simulation session and release resources.
///
/// The simulator should clean up any internal state, temp files, or
/// resources associated with this session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteSessionRequest {
    /// Identifier of the session to tear down.
    pub session_id: String,
}

/// Response to [`DeleteSessionRequest`].
///
/// Empty on success.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteSessionReply {}

// ===========================================================================
//  GetSessionHealth
// ===========================================================================

/// Query the health of a running session.
///
/// The daemon polls this periodically.  Simulators should return
/// [`Healthy`](crate::common::HealthStatus::Healthy) when the simulation
/// engine inside the container is responsive and processing events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionHealthRequest {
    /// Identifier of the session to query.
    pub session_id: String,
}

/// Response to [`GetSessionHealthRequest`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetSessionHealthReply {
    /// Health status report for the requested session.
    pub health: SessionHealth,
}

// ===========================================================================
//  GetSessionPerf
// ===========================================================================

/// Query performance counters for a running session.
///
/// Returns simulation-time vs wall-time metrics, tick counts, IPC, and
/// any simulator-specific counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionPerfRequest {
    /// Identifier of the session to query.
    pub session_id: String,
}

/// Response to [`GetSessionPerfRequest`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetSessionPerfReply {
    /// Performance counters for the requested session.
    pub perf: SessionPerf,
}

// ===========================================================================
//  GetRunDef
// ===========================================================================

/// Transform a run request into a simulator-aware [`ExecDef`].
///
/// Called by the daemon before starting every run inside a session.
/// The simulator can:
///
/// - Inject or override environment variables (e.g. `ROCJITSU_CONFIG`,
///   `HSA_OVERRIDE_GFX_VERSION`).
/// - Wrap or rewrite the command (e.g. prefix with a simulator launcher).
/// - Append extra arguments.
///
/// The returned [`ExecDef`] is what the daemon actually executes inside
/// the container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRunDefRequest {
    /// The original run definition from the client.
    pub run: RunDef,
}

/// Response to [`GetRunDefRequest`].
///
/// Returns the (possibly modified) [`ExecDef`] with simulator-specific
/// env vars and command transformations applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetRunDefReply {
    /// The final execution definition the daemon should run.
    pub exec: ExecDef,
}
