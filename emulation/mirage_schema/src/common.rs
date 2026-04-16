//! Common types shared across the Mirage schema.
//!
//! This module corresponds to the `common.fbs` FlatBuffer schema
//! (`namespace mirage.fb`).  It contains fundamental building blocks —
//! enums, identifiers, execution primitives, and time representation —
//! that every other schema module depends on.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
//  GPU family & definition
// ---------------------------------------------------------------------------

/// The family or vendor of a GPU implementation.
///
/// Used to tag [`GpuDef`]s so the daemon knows which simulator owns them.
///
/// # Variants
///
/// | Variant    | Description                                |
/// |------------|--------------------------------------------|
/// | `Unknown`  | Family has not been specified.              |
/// | `AmdCdna`  | AMD CDNA architecture – data-centre GPUs.  |
/// | `AmdRdna`  | AMD RDNA architecture – consumer GPUs.     |
/// | `RiscV`    | RISC-V based experimental accelerator.     |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuFamily {
    /// Family has not been specified.
    #[default]
    Unknown,
    /// AMD CDNA architecture (data-center compute GPUs).
    AmdCdna,
    /// AMD RDNA architecture (consumer / workstation GPUs).
    AmdRdna,
    /// RISC-V based experimental accelerator.
    RiscV,
}

/// Describes a specific GPU model that a simulator can emulate.
///
/// A `GpuDef` is returned by a simulator to advertise the GPU models it
/// supports.  The [`name`](GpuDef::name) is a human-readable identifier
/// (e.g. `"MI300X"`) while [`arch`](GpuDef::arch) carries the ISA target
/// string used by compilers (e.g. `"gfx942"`).
///
/// # Examples
///
/// ```
/// # use mirage_schema::common::{GpuDef, GpuFamily};
/// let gpu = GpuDef {
///     name: "MI300X".into(),
///     arch: "gfx942".into(),
///     family: GpuFamily::AmdCdna,
///     description: Some("AMD Instinct MI300X".into()),
/// };
/// assert_eq!(gpu.family, GpuFamily::AmdCdna);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuDef {
    /// Human-readable name of the GPU, e.g. `"MI300X"`, `"MI325X"`.
    pub name: String,

    /// ISA / architecture target string used by the compiler toolchain,
    /// e.g. `"gfx942"`, `"gfx950"`, `"rv64i"`.
    pub arch: String,

    /// The GPU family / vendor this definition belongs to.
    #[serde(default)]
    pub family: GpuFamily,

    /// Optional free-form description for display in the dashboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
//  Simulator mode
// ---------------------------------------------------------------------------

/// The fidelity level at which a simulator models the hardware.
///
/// Higher-fidelity modes are more accurate but slower.
///
/// # Variants
///
/// | Variant         | Description                                         |
/// |-----------------|-----------------------------------------------------|
/// | `Functional`    | High-level instruction execution, no timing detail. |
/// | `Clocked`       | Discrete time-step simulation (per-instruction).    |
/// | `CycleAccurate` | Full per-cycle modelling – most detailed & slowest. |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulatorMode {
    /// Functional mode models the hardware at a high level — the simulator
    /// executes instructions without detailed timing.
    #[default]
    Functional,
    /// Clocked mode advances in discrete time steps (e.g. per instruction
    /// or per event).
    Clocked,
    /// Cycle-accurate mode models every cycle of the hardware.  This is the
    /// most detailed and slowest mode.
    CycleAccurate,
}

// ---------------------------------------------------------------------------
//  Profile
// ---------------------------------------------------------------------------

/// A named configuration that binds together a simulator, GPU model,
/// simulation mode, and cluster shape.
///
/// The daemon resolves profile names to these definitions when creating
/// sessions.
///
/// # Fields
///
/// * `num_gpus` – number of GPUs **per node** (default `1`).
/// * `num_nodes` – total number of nodes in the cluster (default `1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDef {
    /// Unique name of this profile.
    pub name: String,

    /// Which simulator to use for this profile (matches
    /// [`SimulatorInfo::name`](crate::simulator::SimulatorInfo::name)).
    pub simulator: String,

    /// The simulation fidelity mode.
    #[serde(default)]
    pub mode: SimulatorMode,

    /// GPU definition name for this profile (matches [`GpuDef::name`]).
    pub gpu: String,

    /// Number of GPUs per node.
    #[serde(default = "one_non_zero_u32")]
    pub num_gpus: NonZeroU32,

    /// Number of nodes in the cluster.
    #[serde(default = "one_non_zero_u32")]
    pub num_nodes: NonZeroU32,
}

/// Helper for `#[serde(default)]` — returns `1u32`.
fn one_non_zero_u32() -> NonZeroU32 {
    NonZeroU32::new(1).unwrap()
}

// ---------------------------------------------------------------------------
//  Environment variable pair
// ---------------------------------------------------------------------------

/// A key-value pair representing a single environment variable.
///
/// Used in [`ExecDef::env`] and [`ContainerDef::env`](crate::container::ContainerDef::env)
/// to pass environment variables to a process or container.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SetEnv {
    /// Environment variable name, e.g. `"LD_PRELOAD"`.
    pub key: String,
    /// Environment variable value.
    pub value: String,
}

// ---------------------------------------------------------------------------
//  Execution primitives
// ---------------------------------------------------------------------------

/// How a run ended.
///
/// Sent from the daemon to clients when a previously launched process
/// terminates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunExit {
    /// Identifier of the run that exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,

    /// Process exit code (`0` = success).
    #[serde(default)]
    pub exit_code: i32,
}

/// A request to run a command.
///
/// Describes the program, its arguments, and any extra environment
/// variables.  Used both directly by clients and inside [`RunDef`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecDef {
    /// The program to run (absolute path or `$PATH`-resolved name).
    pub command: String,

    /// Arguments to the command, e.g. `["-c", "echo hello world"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    /// Extra environment variables to set for this run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<SetEnv>,
}

// ---------------------------------------------------------------------------
//  Streams
// ---------------------------------------------------------------------------

/// Identifies a standard I/O stream.
///
/// Used in [`StreamData`] to tag which stream a chunk of bytes belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stream {
    /// Standard input.
    #[default]
    Stdin,
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// A chunk of data from a standard I/O stream.
///
/// Used to multiplex `stdout` / `stderr` (and optionally `stdin`) over a
/// single channel between the daemon and its clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamData {
    /// Which stream this data belongs to.
    #[serde(default)]
    pub stream: Stream,

    /// Raw bytes of the stream output.
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
//  Run & session definitions
// ---------------------------------------------------------------------------

/// A request to execute a command inside an existing session.
///
/// The daemon resolves the session, looks up its simulator, and calls
/// `get_exec_run_def` to let the simulator inject any extra environment
/// variables or wrapper commands before the run is actually started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDef {
    /// The session to use for this execution.
    pub session: String,

    /// What to run on the head node.
    pub exec: ExecDef,

    /// Optional command to run on worker nodes.  If `None`, workers won't
    /// run any command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_exec: Option<ExecDef>,
}

/// Defines a simulation session.
///
/// A session binds a profile (simulator + GPU + mode) to a container image
/// and gives it a unique name that other APIs reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDef {
    /// A unique name for this session.
    pub name: String,

    /// The profile to use for this session (matches [`ProfileDef::name`]).
    pub profile: String,

    /// The container image to use for this session.
    /// Should be fully qualified, e.g. `"ghcr.io/username/image:tag"`.
    #[serde(default)]
    pub image: String,
}

// ---------------------------------------------------------------------------
//  Cluster
// ---------------------------------------------------------------------------

/// Describes a multi-node simulation cluster.
///
/// Contains the head node address, optional worker addresses, and the
/// commands to run on each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterDef {
    /// Cluster name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Network address of the head node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_address: Option<String>,

    /// Command to execute on the head node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_exec: Option<ExecDef>,

    /// Network addresses of the worker nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workers_address: Vec<String>,

    /// Command to execute on each worker node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_exec: Option<ExecDef>,
}

// ---------------------------------------------------------------------------
//  Health
// ---------------------------------------------------------------------------

/// Health status of a session or component.
///
/// The daemon polls simulator health periodically and surfaces this in the
/// dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Status has not been determined yet.
    #[default]
    Unknown,
    /// The component is operating normally.
    Healthy,
    /// The component has encountered an error.
    Unhealthy,
}

// ---------------------------------------------------------------------------
//  Time
// ---------------------------------------------------------------------------

/// A picosecond-accurate timestamp.
///
/// The total time is `seconds + picoseconds × 10⁻¹²` seconds.
///
/// # Examples
///
/// ```
/// # use mirage_schema::common::Time;
/// // Represent exactly 1.5 seconds:
/// let t = Time {
///     seconds: 1,
///     picoseconds: 500_000_000_000,
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Time {
    /// Whole seconds component.
    #[serde(default)]
    pub seconds: u64,

    /// Additional picoseconds to add to [`seconds`](Self::seconds).
    ///
    /// Must be in the range `0..1_000_000_000_000` (< 1 second).
    #[serde(default)]
    pub picoseconds: u64,
}
