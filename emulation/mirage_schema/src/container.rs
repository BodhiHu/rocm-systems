//! OCI container definition types.
//!
//! This module corresponds to the `container.fbs` FlatBuffer schema
//! (`namespace mirage.container`).  It describes everything the daemon
//! needs to launch, configure, and connect to an OCI container for a
//! simulation session.
//!
//! The types are **runtime-agnostic** — the daemon translates them to
//! Docker, Podman, or any other OCI-compatible runtime without the
//! simulator ever knowing which runtime is in use.

use serde::{Deserialize, Serialize};

use crate::common::{ExecArgs, SetEnv};

// ---------------------------------------------------------------------------
//  Bind mounts
// ---------------------------------------------------------------------------

/// A bind-mount from the host into the container.
///
/// Used to expose simulator libraries, configuration files, device nodes,
/// or shared-memory regions to the containerised workload.
///
/// # Examples
///
/// ```
/// # use mirage_schema::container::BindMount;
/// let mount = BindMount {
///     host_path: "/opt/rocjitsu/lib".into(),
///     container_path: "/usr/lib/rocjitsu".into(),
///     readonly: true,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindMount {
    /// Absolute path on the host.
    pub host_path: String,

    /// Absolute path inside the container where [`host_path`](Self::host_path)
    /// is mounted.
    pub container_path: String,

    /// If `true` the mount is read-only inside the container.
    #[serde(default = "default_true")]
    pub readonly: bool,
}

// ---------------------------------------------------------------------------
//  Injected files
// ---------------------------------------------------------------------------

/// A file whose contents are generated at session-creation time and
/// injected into the container filesystem.
///
/// Unlike bind-mounts, injected files do **not** correspond to an existing
/// host path — the daemon writes [`content`](Self::content) to a temp file
/// and mount it.  This is how simulator-generated configs, schemas, and
/// stubs are delivered to the container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectedFile {
    /// Absolute path inside the container where the file will appear.
    pub container_path: String,

    /// Raw file content.
    ///
    /// For text files (JSON configs, FBS schemas) this is UTF-8 encoded.
    /// For binary blobs use raw bytes.
    pub content: Vec<u8>,

    /// If `true` the file is mounted read-only (default).
    #[serde(default = "default_true")]
    pub readonly: bool,

    /// Optional human-readable label for debugging / dashboard display,
    /// e.g. `"rocjitsu simulation config"`, `"simulation_config.fbs schema"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

// ---------------------------------------------------------------------------
//  Port mappings
// ---------------------------------------------------------------------------

/// Transport protocol for a [`PortMapping`].
///
/// Corresponds to the OCI `-p` flag's `/protocol` suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// Transmission Control Protocol (default).
    #[default]
    Tcp,
    /// User Datagram Protocol.
    Udp,
}

/// A port mapping from container to host.
///
/// Maps to the OCI `-p host_port:container_port/protocol` flag.
/// If [`host_port`](Self::host_port) is `0`, the runtime picks an
/// ephemeral port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMapping {
    /// Port number inside the container.
    pub container_port: u16,

    /// Port number on the host.  `0` means "let the runtime choose".
    #[serde(default)]
    pub host_port: u16,

    /// Transport protocol: TCP (default) or UDP.
    #[serde(default)]
    pub protocol: Protocol,

    /// Optional human-readable label, e.g. `"metrics"`, `"debug-server"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

// ---------------------------------------------------------------------------
//  Container definition
// ---------------------------------------------------------------------------

/// Full specification of the OCI container that should be launched for a
/// simulation session.
///
/// Returned by the simulator in a `CreateSessionReply`.  The daemon hands
/// this to the container runtime (Docker, Podman, …) without modification.
///
/// This keeps the simulator completely decoupled from the container
/// runtime implementation.
///
/// # Key fields
///
/// | Field               | Purpose                                        |
/// |---------------------|------------------------------------------------|
/// | [`image`]           | Container image reference.                     |
/// | [`mounts`]          | Host bind-mounts.                              |
/// | [`injected_files`]  | Simulator-generated files injected at runtime. |
/// | [`entrypoint`]      | What to execute.                               |
/// | [`ports`]           | Exposed port mappings.                         |
/// | [`privileged`]      | Whether `--privileged` is required.            |
///
/// [`image`]: ContainerDef::image
/// [`mounts`]: ContainerDef::mounts
/// [`injected_files`]: ContainerDef::injected_files
/// [`entrypoint`]: ContainerDef::entrypoint
/// [`ports`]: ContainerDef::ports
/// [`privileged`]: ContainerDef::privileged
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerDef {
    /// Fully-qualified container image reference,
    /// e.g. `"ghcr.io/rocm/pytorch:latest"`.
    ///
    /// Mandatory: this must be provided by the simulator.
    pub image: String,

    /// Host paths to bind-mount into the container.
    ///
    /// Used for simulator shared libraries, device stubs, etc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<BindMount>,

    /// Files generated by the simulator to inject into the container.
    ///
    /// Typical entries: simulation config JSON, FlatBuffers schema files,
    /// generated GPU topology, stub device nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub injected_files: Vec<InjectedFile>,

    /// what to execute.
    pub entrypoint: ExecArgs,

    /// Optional working directory inside the container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,

    /// Ports to expose from the container.
    ///
    /// Each entry maps a container port to an optional host port.
    /// Used for simulator debug servers, metrics endpoints, or
    /// application-level services (e.g. vLLM serving endpoint).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<PortMapping>,

    /// Whether the container needs `--privileged` or elevated caps.
    ///
    /// Simulators should avoid requiring this whenever possible.
    #[serde(default)]
    pub privileged: bool,

    /// Optional resource limits (CPU, memory) for the container,
    /// encoded as a JSON string for runtime-agnostic portability.
    ///
    /// Example: `{"cpu": "4", "memory": "16Gi"}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_limits_json: Option<String>,
}

// ---------------------------------------------------------------------------
//  Helpers
// ---------------------------------------------------------------------------

/// Returns `true` — used as `#[serde(default)]` for boolean fields whose
/// FlatBuffer default is `true`.
fn default_true() -> bool {
    true
}
