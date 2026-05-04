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

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::common::ExecArgs;

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

    /// Host device nodes to pass through (e.g. `/dev/kfd`, `/dev/dri/renderD128`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,

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

    /// Optional Docker/OCI network to attach this container to.
    ///
    /// When set, the container runtime passes `--network <name>` so the
    /// container joins a pre-created network.  Used for multi-node
    /// sessions where containers need to reach each other by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,

    /// OCI labels applied to the container (`--label key=value`).
    ///
    /// Used by the daemon to tag containers with session metadata so
    /// running sessions can be rediscovered after a daemon restart.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
//  Container runtime contract
// ---------------------------------------------------------------------------

/// Progress sender used by long-running container runtime operations.
pub type ContainerRuntimeProgressSender = UnboundedSender<ContainerRuntimeEvent>;

/// Progress receiver used by long-running container runtime operations.
pub type ContainerRuntimeProgressReceiver = UnboundedReceiver<ContainerRuntimeEvent>;

/// Creates an unbounded progress channel for container runtime events.
pub fn container_runtime_progress_channel() -> (
    ContainerRuntimeProgressSender,
    ContainerRuntimeProgressReceiver,
) {
    unbounded_channel()
}

/// Standard result type for container runtime operations.
pub type Result<T> = std::result::Result<T, ContainerRuntimeError>;

/// High-level operation performed by the container runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerRuntimeOperation {
    PullImage,
    StartContainer,
    InspectContainer,
    ReadLogs,
    Exec,
    StopContainer,
    RemoveContainer,
    CreateNetwork,
    RemoveNetwork,
}

/// Streamable progress event emitted by the container runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContainerRuntimeEvent {
    Status {
        operation: ContainerRuntimeOperation,
        message: String,
    },
    Stdout {
        operation: ContainerRuntimeOperation,
        chunk: Vec<u8>,
    },
    Stderr {
        operation: ContainerRuntimeOperation,
        chunk: Vec<u8>,
    },
}

/// Runtime abstraction for managing OCI-compatible containers used by Mirage.
#[async_trait]
pub trait ContainerRuntime: Send + Sync {
    async fn pull_image(
        &self,
        image: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()>;

    async fn start_container(
        &self,
        request: StartContainerRequest,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<StartedContainer>;

    async fn inspect_container(
        &self,
        handle: &ContainerHandle,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ContainerInspection>;

    async fn read_logs(
        &self,
        handle: &ContainerHandle,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ContainerLogs>;

    async fn exec(
        &self,
        request: ExecRequest,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ExecResult>;

    async fn stop_container(
        &self,
        handle: &ContainerHandle,
        timeout_secs: u32,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()>;

    async fn remove_container(
        &self,
        handle: &ContainerHandle,
        force: bool,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()>;

    /// Create a named network for inter-container communication.
    async fn create_network(
        &self,
        name: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()>;

    /// Remove a named network.
    async fn remove_network(
        &self,
        name: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()>;

    /// List running containers that match **all** of the given label
    /// key=value pairs.
    ///
    /// Returns a lightweight summary for each match.  This is used by the
    /// daemon to rediscover sessions from Docker state instead of keeping
    /// them in memory.
    async fn list_containers(
        &self,
        labels: &BTreeMap<String, String>,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<Vec<ListedContainer>>;
}

/// Container start request resolved by the simulator and consumed by a runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartContainerRequest {
    pub name: String,
    pub container: ContainerDef,
}

/// Runtime-specific handle to a managed container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerHandle {
    pub id: String,
    pub name: String,
}

/// Lifecycle state returned by the runtime for a managed container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    Created,
    Running,
    Exited,
    Dead,
}

/// Port mapping resolved by the runtime after the container has started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPortMapping {
    pub container_port: u16,
    pub host_port: Option<u16>,
    pub protocol: Protocol,
    pub label: Option<String>,
}

/// Resolved inspection result for a managed container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInspection {
    pub handle: ContainerHandle,
    pub image: String,
    pub state: ContainerState,
    pub exit_code: Option<i32>,
    pub ports: Vec<ResolvedPortMapping>,
}

/// Start result including the first resolved inspection payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedContainer {
    pub inspection: ContainerInspection,
}

/// Lightweight summary returned by [`ContainerRuntime::list_containers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedContainer {
    pub handle: ContainerHandle,
    pub image: String,
    pub state: ContainerState,
    pub labels: BTreeMap<String, String>,
}

/// Captured container logs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerLogs {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Request to execute a command inside a running container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    pub container: ContainerHandle,
    pub exec: ExecArgs,
    pub working_dir: Option<String>,
}

/// Result of an `exec` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Failure modes exposed by the container runtime contract.
#[derive(Debug)]
pub enum ContainerRuntimeError {
    InvalidRequest(String),
    NotFound(String),
    Parse(String),
    RuntimeUnavailable(String),
    CommandFailed {
        command: String,
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for ContainerRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) => write!(f, "invalid container request: {message}"),
            Self::NotFound(message) => write!(f, "container not found: {message}"),
            Self::Parse(message) => {
                write!(f, "failed to parse container runtime output: {message}")
            }
            Self::RuntimeUnavailable(message) => {
                write!(f, "container runtime unavailable: {message}")
            }
            Self::CommandFailed {
                command,
                status,
                stdout,
                stderr,
            } => {
                write!(f, "container command failed: {command}")?;
                if let Some(status) = status {
                    write!(f, " (exit {status})")?;
                }
                if !stderr.is_empty() {
                    write!(f, ": {stderr}")?;
                } else if !stdout.is_empty() {
                    write!(f, ": {stdout}")?;
                }
                Ok(())
            }
            Self::Io(error) => write!(f, "container runtime I/O error: {error}"),
            Self::Json(error) => write!(f, "container runtime JSON error: {error}"),
        }
    }
}

impl Error for ContainerRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ContainerRuntimeError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for ContainerRuntimeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

// ---------------------------------------------------------------------------
//  Helpers
// ---------------------------------------------------------------------------

/// Returns `true` — used as `#[serde(default)]` for boolean fields whose
/// FlatBuffer default is `true`.
fn default_true() -> bool {
    true
}
