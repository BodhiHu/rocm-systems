#![forbid(unsafe_code)]
pub mod docker_cli;
pub mod mock;

pub use docker_cli::{CommandOutput, CommandRunner, DockerCli, TokioCommandRunner};
pub use mirage_schema::container::{
    ContainerHandle, ContainerInspection, ContainerLogs, ContainerRuntime, ContainerRuntimeError,
    ContainerRuntimeEvent, ContainerRuntimeOperation, ContainerRuntimeProgressReceiver,
    ContainerRuntimeProgressSender, ContainerState, ExecRequest, ExecResult, ListedContainer,
    ResolvedPortMapping, Result, StartContainerRequest, StartedContainer,
    container_runtime_progress_channel,
};
pub use mock::MockContainerRuntime;
