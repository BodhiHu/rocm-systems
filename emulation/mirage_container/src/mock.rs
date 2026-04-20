use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use mirage_schema::container::{
    ContainerHandle, ContainerInspection, ContainerLogs, ContainerRuntime, ContainerRuntimeError,
    ContainerRuntimeEvent, ContainerRuntimeOperation, ContainerRuntimeProgressSender,
    ContainerState, ExecRequest, ExecResult, ListedContainer, ResolvedPortMapping, Result,
    StartContainerRequest, StartedContainer,
};

fn emit_status(
    progress: Option<&ContainerRuntimeProgressSender>,
    operation: ContainerRuntimeOperation,
    message: impl Into<String>,
) {
    if let Some(progress) = progress {
        let _ = progress.send(ContainerRuntimeEvent::Status {
            operation,
            message: message.into(),
        });
    }
}

fn emit_stdout(
    progress: Option<&ContainerRuntimeProgressSender>,
    operation: ContainerRuntimeOperation,
    chunk: &[u8],
) {
    if chunk.is_empty() {
        return;
    }
    if let Some(progress) = progress {
        let _ = progress.send(ContainerRuntimeEvent::Stdout {
            operation,
            chunk: chunk.to_vec(),
        });
    }
}

fn emit_stderr(
    progress: Option<&ContainerRuntimeProgressSender>,
    operation: ContainerRuntimeOperation,
    chunk: &[u8],
) {
    if chunk.is_empty() {
        return;
    }
    if let Some(progress) = progress {
        let _ = progress.send(ContainerRuntimeEvent::Stderr {
            operation,
            chunk: chunk.to_vec(),
        });
    }
}

#[derive(Debug)]
pub struct MockContainerRuntime {
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    pulled_images: Vec<String>,
    start_requests: Vec<StartContainerRequest>,
    exec_requests: Vec<ExecRequest>,
    networks: Vec<String>,
    containers: BTreeMap<String, MockContainer>,
    next_container_id: u64,
    next_ephemeral_port: u16,
    next_pull_error: Option<String>,
    next_start_error: Option<String>,
    next_exec_errors: BTreeMap<String, String>,
    next_log_errors: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct MockContainer {
    inspection: ContainerInspection,
    logs: ContainerLogs,
    next_exec_results: VecDeque<ExecResult>,
    labels: BTreeMap<String, String>,
}

impl Default for MockContainerRuntime {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                pulled_images: Vec::new(),
                start_requests: Vec::new(),
                exec_requests: Vec::new(),
                networks: Vec::new(),
                containers: BTreeMap::new(),
                next_container_id: 0,
                next_ephemeral_port: 40_000,
                next_pull_error: None,
                next_start_error: None,
                next_exec_errors: BTreeMap::new(),
                next_log_errors: BTreeMap::new(),
            }),
        }
    }
}

impl MockContainerRuntime {
    pub async fn pulled_images(&self) -> Vec<String> {
        self.state.lock().unwrap().pulled_images.clone()
    }

    pub async fn start_requests(&self) -> Vec<StartContainerRequest> {
        self.state.lock().unwrap().start_requests.clone()
    }

    pub async fn exec_requests(&self) -> Vec<ExecRequest> {
        self.state.lock().unwrap().exec_requests.clone()
    }

    pub async fn networks(&self) -> Vec<String> {
        self.state.lock().unwrap().networks.clone()
    }

    pub async fn fail_next_pull(&self, message: impl Into<String>) {
        self.state.lock().unwrap().next_pull_error = Some(message.into());
    }

    pub async fn fail_next_start(&self, message: impl Into<String>) {
        self.state.lock().unwrap().next_start_error = Some(message.into());
    }

    pub async fn fail_next_exec(&self, handle: &ContainerHandle, message: impl Into<String>) {
        self.state
            .lock()
            .unwrap()
            .next_exec_errors
            .insert(handle.id.clone(), message.into());
    }

    pub async fn fail_next_logs(&self, handle: &ContainerHandle, message: impl Into<String>) {
        self.state
            .lock()
            .unwrap()
            .next_log_errors
            .insert(handle.id.clone(), message.into());
    }

    pub async fn set_logs(&self, handle: &ContainerHandle, logs: ContainerLogs) {
        if let Some(container) = self.state.lock().unwrap().containers.get_mut(&handle.id) {
            container.logs = logs;
        }
    }

    pub async fn queue_exec_result(&self, handle: &ContainerHandle, result: ExecResult) {
        if let Some(container) = self.state.lock().unwrap().containers.get_mut(&handle.id) {
            container.next_exec_results.push_back(result);
        }
    }

    fn command_failure(action: &str, message: String) -> ContainerRuntimeError {
        ContainerRuntimeError::CommandFailed {
            command: format!("mock {action}"),
            status: Some(1),
            stdout: String::new(),
            stderr: message,
        }
    }
}

#[async_trait]
impl ContainerRuntime for MockContainerRuntime {
    async fn pull_image(
        &self,
        image: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::PullImage,
            format!("pulling image {image}"),
        );

        let mut state = self.state.lock().unwrap();
        if let Some(message) = state.next_pull_error.take() {
            return Err(Self::command_failure("pull", message));
        }
        state.pulled_images.push(image.to_string());

        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::PullImage,
            format!("finished pulling image {image}"),
        );
        Ok(())
    }

    async fn start_container(
        &self,
        request: StartContainerRequest,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<StartedContainer> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::StartContainer,
            format!("starting container {}", request.name),
        );

        let mut state = self.state.lock().unwrap();
        if let Some(message) = state.next_start_error.take() {
            return Err(Self::command_failure("run", message));
        }

        state.start_requests.push(request.clone());
        state.next_container_id += 1;

        let handle = ContainerHandle {
            id: format!("mock-{}", state.next_container_id),
            name: request.name.clone(),
        };

        let mut ports = Vec::with_capacity(request.container.ports.len());
        for port in &request.container.ports {
            let host_port = if port.host_port == 0 {
                let allocated = state.next_ephemeral_port;
                state.next_ephemeral_port += 1;
                Some(allocated)
            } else {
                Some(port.host_port)
            };

            ports.push(ResolvedPortMapping {
                container_port: port.container_port,
                host_port,
                protocol: port.protocol,
                label: port.label.clone(),
            });
        }

        let inspection = ContainerInspection {
            handle: handle.clone(),
            image: request.container.image.clone(),
            state: ContainerState::Running,
            exit_code: None,
            ports,
        };
        let labels = request.container.labels.clone();
        state.containers.insert(
            handle.id.clone(),
            MockContainer {
                inspection: inspection.clone(),
                logs: ContainerLogs::default(),
                next_exec_results: VecDeque::new(),
                labels,
            },
        );

        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::StartContainer,
            format!("started container {}", handle.id),
        );
        Ok(StartedContainer { inspection })
    }

    async fn inspect_container(
        &self,
        handle: &ContainerHandle,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ContainerInspection> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::InspectContainer,
            format!("inspecting container {}", handle.id),
        );

        self.state
            .lock()
            .unwrap()
            .containers
            .get(&handle.id)
            .map(|container| container.inspection.clone())
            .ok_or_else(|| ContainerRuntimeError::NotFound(handle.id.clone()))
    }

    async fn read_logs(
        &self,
        handle: &ContainerHandle,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ContainerLogs> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::ReadLogs,
            format!("reading logs for {}", handle.id),
        );

        let mut state = self.state.lock().unwrap();
        if let Some(message) = state.next_log_errors.remove(&handle.id) {
            return Err(Self::command_failure("logs", message));
        }

        let logs = state
            .containers
            .get(&handle.id)
            .map(|container| container.logs.clone())
            .ok_or_else(|| ContainerRuntimeError::NotFound(handle.id.clone()))?;
        emit_stdout(
            progress.as_ref(),
            ContainerRuntimeOperation::ReadLogs,
            &logs.stdout,
        );
        emit_stderr(
            progress.as_ref(),
            ContainerRuntimeOperation::ReadLogs,
            &logs.stderr,
        );
        Ok(logs)
    }

    async fn exec(
        &self,
        request: ExecRequest,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ExecResult> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::Exec,
            format!(
                "executing {} in {}",
                request.exec.command, request.container.id
            ),
        );

        let mut state = self.state.lock().unwrap();
        state.exec_requests.push(request.clone());
        if let Some(message) = state.next_exec_errors.remove(&request.container.id) {
            return Err(Self::command_failure("exec", message));
        }

        let container = state
            .containers
            .get_mut(&request.container.id)
            .ok_or_else(|| ContainerRuntimeError::NotFound(request.container.id.clone()))?;
        let result = container
            .next_exec_results
            .pop_front()
            .unwrap_or(ExecResult {
                exit_code: 0,
                stdout: Vec::new(),
                stderr: Vec::new(),
            });

        emit_stdout(
            progress.as_ref(),
            ContainerRuntimeOperation::Exec,
            &result.stdout,
        );
        emit_stderr(
            progress.as_ref(),
            ContainerRuntimeOperation::Exec,
            &result.stderr,
        );
        Ok(result)
    }

    async fn stop_container(
        &self,
        handle: &ContainerHandle,
        _timeout_secs: u32,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::StopContainer,
            format!("stopping container {}", handle.id),
        );

        let mut state = self.state.lock().unwrap();
        let container = state
            .containers
            .get_mut(&handle.id)
            .ok_or_else(|| ContainerRuntimeError::NotFound(handle.id.clone()))?;
        container.inspection.state = ContainerState::Exited;
        container.inspection.exit_code = Some(0);
        Ok(())
    }

    async fn remove_container(
        &self,
        handle: &ContainerHandle,
        _force: bool,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::RemoveContainer,
            format!("removing container {}", handle.id),
        );

        let removed = self.state.lock().unwrap().containers.remove(&handle.id);
        if removed.is_some() {
            Ok(())
        } else {
            Err(ContainerRuntimeError::NotFound(handle.id.clone()))
        }
    }

    async fn create_network(
        &self,
        name: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::CreateNetwork,
            format!("creating network {name}"),
        );
        self.state.lock().unwrap().networks.push(name.to_string());
        Ok(())
    }

    async fn remove_network(
        &self,
        name: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        emit_status(
            progress.as_ref(),
            ContainerRuntimeOperation::RemoveNetwork,
            format!("removing network {name}"),
        );
        let mut state = self.state.lock().unwrap();
        state.networks.retain(|n| n != name);
        Ok(())
    }

    async fn list_containers(
        &self,
        labels: &BTreeMap<String, String>,
        _progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<Vec<ListedContainer>> {
        let state = self.state.lock().unwrap();
        let mut result = Vec::new();
        for container in state.containers.values() {
            let matches = labels
                .iter()
                .all(|(k, v)| container.labels.get(k).map_or(false, |cv| cv == v));
            if matches {
                result.push(ListedContainer {
                    handle: container.inspection.handle.clone(),
                    image: container.inspection.image.clone(),
                    state: container.inspection.state,
                    labels: container.labels.clone(),
                });
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use mirage_schema::common::ExecArgs;
    use mirage_schema::container::{
        ContainerDef, ContainerRuntimeEvent, ContainerRuntimeOperation,
        ContainerRuntimeProgressReceiver, PortMapping, Protocol,
        container_runtime_progress_channel,
    };

    use super::*;

    fn container_request() -> StartContainerRequest {
        StartContainerRequest {
            name: "session-a-node0".to_string(),
            container: ContainerDef {
                image: "ghcr.io/example/runtime:latest".to_string(),
                mounts: Vec::new(),
                injected_files: Vec::new(),
                entrypoint: ExecArgs {
                    command: "/bin/app".to_string(),
                    args: vec!["--serve".to_string()],
                    env: Vec::new(),
                },
                working_dir: None,
                ports: vec![
                    PortMapping {
                        container_port: 8080,
                        host_port: 0,
                        protocol: Protocol::Tcp,
                        label: Some("http".to_string()),
                    },
                    PortMapping {
                        container_port: 9000,
                        host_port: 9000,
                        protocol: Protocol::Udp,
                        label: Some("metrics".to_string()),
                    },
                ],
                privileged: false,
                devices: vec![],
                resource_limits_json: None,
                network: None,
            },
        }
    }

    fn drain_events(receiver: &mut ContainerRuntimeProgressReceiver) -> Vec<ContainerRuntimeEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn mock_runtime_exercises_full_lifecycle() {
        let runtime = MockContainerRuntime::default();
        runtime
            .pull_image("ghcr.io/example/runtime:latest", None)
            .await
            .unwrap();

        let started = runtime
            .start_container(container_request(), None)
            .await
            .unwrap();
        assert_eq!(started.inspection.handle.id, "mock-1");
        assert_eq!(started.inspection.state, ContainerState::Running);
        assert_eq!(started.inspection.ports[0].host_port, Some(40_000));
        assert_eq!(started.inspection.ports[1].host_port, Some(9000));

        runtime
            .set_logs(
                &started.inspection.handle,
                ContainerLogs {
                    stdout: b"hello".to_vec(),
                    stderr: b"warn".to_vec(),
                },
            )
            .await;
        runtime
            .queue_exec_result(
                &started.inspection.handle,
                ExecResult {
                    exit_code: 12,
                    stdout: b"done".to_vec(),
                    stderr: b"oops".to_vec(),
                },
            )
            .await;

        let inspection = runtime
            .inspect_container(&started.inspection.handle, None)
            .await
            .unwrap();
        assert_eq!(inspection, started.inspection);

        let logs = runtime
            .read_logs(&started.inspection.handle, None)
            .await
            .unwrap();
        assert_eq!(logs.stdout, b"hello");
        assert_eq!(logs.stderr, b"warn");

        let exec = runtime
            .exec(
                ExecRequest {
                    container: started.inspection.handle.clone(),
                    exec: ExecArgs {
                        command: "/bin/check".to_string(),
                        args: vec!["--verbose".to_string()],
                        env: Vec::new(),
                    },
                    working_dir: Some("/work".to_string()),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(exec.exit_code, 12);
        assert_eq!(exec.stdout, b"done");
        assert_eq!(exec.stderr, b"oops");

        runtime
            .stop_container(&started.inspection.handle, 15, None)
            .await
            .unwrap();
        let inspection = runtime
            .inspect_container(&started.inspection.handle, None)
            .await
            .unwrap();
        assert_eq!(inspection.state, ContainerState::Exited);
        assert_eq!(inspection.exit_code, Some(0));

        runtime
            .remove_container(&started.inspection.handle, true, None)
            .await
            .unwrap();
        let error = runtime
            .inspect_container(&started.inspection.handle, None)
            .await
            .unwrap_err();
        assert!(matches!(error, ContainerRuntimeError::NotFound(_)));

        let pulls = runtime.pulled_images().await;
        assert_eq!(pulls, vec!["ghcr.io/example/runtime:latest".to_string()]);
        assert_eq!(runtime.start_requests().await.len(), 1);
        assert_eq!(runtime.exec_requests().await.len(), 1);
    }

    #[tokio::test]
    async fn mock_runtime_supports_failure_injection() {
        let runtime = MockContainerRuntime::default();
        runtime.fail_next_pull("registry down").await;
        let error = runtime
            .pull_image("ghcr.io/example/runtime:latest", None)
            .await
            .unwrap_err();
        assert!(matches!(error, ContainerRuntimeError::CommandFailed { .. }));

        runtime.fail_next_start("cannot launch").await;
        let error = runtime
            .start_container(container_request(), None)
            .await
            .unwrap_err();
        assert!(matches!(error, ContainerRuntimeError::CommandFailed { .. }));

        let started = runtime
            .start_container(container_request(), None)
            .await
            .unwrap();
        runtime
            .fail_next_logs(&started.inspection.handle, "logs unavailable")
            .await;
        let error = runtime
            .read_logs(&started.inspection.handle, None)
            .await
            .unwrap_err();
        assert!(matches!(error, ContainerRuntimeError::CommandFailed { .. }));

        runtime
            .fail_next_exec(&started.inspection.handle, "exec unavailable")
            .await;
        let error = runtime
            .exec(
                ExecRequest {
                    container: started.inspection.handle.clone(),
                    exec: ExecArgs {
                        command: "/bin/echo".to_string(),
                        args: vec!["hi".to_string()],
                        env: Vec::new(),
                    },
                    working_dir: None,
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ContainerRuntimeError::CommandFailed { .. }));
    }

    #[tokio::test]
    async fn mock_runtime_emits_progress_events() {
        let runtime = MockContainerRuntime::default();
        let (progress, mut receiver) = container_runtime_progress_channel();
        runtime
            .pull_image("ghcr.io/example/runtime:latest", Some(progress.clone()))
            .await
            .unwrap();
        let started = runtime
            .start_container(container_request(), Some(progress.clone()))
            .await
            .unwrap();
        runtime
            .set_logs(
                &started.inspection.handle,
                ContainerLogs {
                    stdout: b"hello".to_vec(),
                    stderr: b"warn".to_vec(),
                },
            )
            .await;

        runtime
            .read_logs(&started.inspection.handle, Some(progress.clone()))
            .await
            .unwrap();
        runtime
            .exec(
                ExecRequest {
                    container: started.inspection.handle.clone(),
                    exec: ExecArgs {
                        command: "/bin/echo".to_string(),
                        args: vec!["hi".to_string()],
                        env: Vec::new(),
                    },
                    working_dir: None,
                },
                Some(progress),
            )
            .await
            .unwrap();

        let events = drain_events(&mut receiver);
        assert!(events.iter().any(|event| matches!(
            event,
            ContainerRuntimeEvent::Status {
                operation: ContainerRuntimeOperation::PullImage,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ContainerRuntimeEvent::Status {
                operation: ContainerRuntimeOperation::StartContainer,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ContainerRuntimeEvent::Stdout {
                operation: ContainerRuntimeOperation::ReadLogs,
                chunk,
            } if chunk == b"hello"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ContainerRuntimeEvent::Stderr {
                operation: ContainerRuntimeOperation::ReadLogs,
                chunk,
            } if chunk == b"warn"
        )));
    }
}
