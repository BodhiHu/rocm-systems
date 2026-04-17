use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use mirage_schema::container::{
    ContainerHandle, ContainerInspection, ContainerLogs, ContainerRuntime, ContainerRuntimeError,
    ContainerRuntimeEvent, ContainerRuntimeOperation, ContainerRuntimeProgressSender,
    ContainerState, ExecRequest, ExecResult, InjectedFile, Protocol, ResolvedPortMapping, Result,
    StartContainerRequest, StartedContainer,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

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

#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, program: &str, args: &[String]) -> std::io::Result<CommandOutput>;

    async fn run_streaming(
        &self,
        program: &str,
        args: &[String],
        operation: ContainerRuntimeOperation,
        progress: &ContainerRuntimeProgressSender,
    ) -> std::io::Result<CommandOutput> {
        let output = self.run(program, args).await?;
        emit_stdout(Some(progress), operation, &output.stdout);
        emit_stderr(Some(progress), operation, &output.stderr);
        Ok(output)
    }
}

#[derive(Debug, Default)]
pub struct TokioCommandRunner;

#[async_trait]
impl CommandRunner for TokioCommandRunner {
    async fn run(&self, program: &str, args: &[String]) -> std::io::Result<CommandOutput> {
        let output = Command::new(program).args(args).output().await?;
        Ok(CommandOutput {
            status: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    async fn run_streaming(
        &self,
        program: &str,
        args: &[String],
        operation: ContainerRuntimeOperation,
        progress: &ContainerRuntimeProgressSender,
    ) -> std::io::Result<CommandOutput> {
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("failed to capture child stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("failed to capture child stderr"))?;

        let stdout_progress = progress.clone();
        let stderr_progress = progress.clone();
        let stdout_task = tokio::spawn(read_stream(
            stdout,
            stdout_progress,
            operation,
            StreamKind::Stdout,
        ));
        let stderr_task = tokio::spawn(read_stream(
            stderr,
            stderr_progress,
            operation,
            StreamKind::Stderr,
        ));

        let status = child.wait().await?;
        let stdout = stdout_task
            .await
            .map_err(|error| io::Error::other(error.to_string()))??;
        let stderr = stderr_task
            .await
            .map_err(|error| io::Error::other(error.to_string()))??;

        Ok(CommandOutput {
            status: status.code(),
            stdout,
            stderr,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

async fn read_stream<R>(
    mut reader: R,
    progress: ContainerRuntimeProgressSender,
    operation: ContainerRuntimeOperation,
    kind: StreamKind,
) -> io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut collected = Vec::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }

        let chunk = &buffer[..read];
        collected.extend_from_slice(chunk);
        match kind {
            StreamKind::Stdout => emit_stdout(Some(&progress), operation, chunk),
            StreamKind::Stderr => emit_stderr(Some(&progress), operation, chunk),
        }
    }

    Ok(collected)
}

#[derive(Debug)]
pub struct DockerCli<R = TokioCommandRunner> {
    program: String,
    runner: Arc<R>,
    temp_root: PathBuf,
    unique_id: AtomicU64,
    managed: Mutex<HashMap<String, ManagedContainer>>,
}

#[derive(Debug, Clone)]
struct ManagedContainer {
    injection_dir: Option<PathBuf>,
    port_labels: HashMap<(u16, Protocol), Option<String>>,
}

impl DockerCli<TokioCommandRunner> {
    pub fn new() -> Self {
        Self::with_program_and_runner("docker", Arc::new(TokioCommandRunner))
    }
}

impl<R> DockerCli<R>
where
    R: CommandRunner + 'static,
{
    pub fn with_runner(runner: Arc<R>) -> Self {
        Self::with_program_and_runner("docker", runner)
    }

    pub fn with_program_and_runner(program: impl Into<String>, runner: Arc<R>) -> Self {
        Self::with_program_runner_and_temp_root(program, runner, std::env::temp_dir())
    }

    pub fn with_program_runner_and_temp_root(
        program: impl Into<String>,
        runner: Arc<R>,
        temp_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            program: program.into(),
            runner,
            temp_root: temp_root.into(),
            unique_id: AtomicU64::new(0),
            managed: Mutex::new(HashMap::new()),
        }
    }

    async fn run_unchecked(
        &self,
        args: &[String],
        operation: ContainerRuntimeOperation,
        progress: Option<&ContainerRuntimeProgressSender>,
    ) -> Result<CommandOutput> {
        let command = format_command(&self.program, args);
        emit_status(progress, operation, format!("running {command}"));

        let output = if let Some(progress) = progress {
            self.runner
                .run_streaming(&self.program, args, operation, progress)
                .await
        } else {
            self.runner.run(&self.program, args).await
        };

        let output = output.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ContainerRuntimeError::RuntimeUnavailable(format!(
                    "{} is not installed or not on PATH",
                    self.program
                ))
            } else {
                ContainerRuntimeError::Io(error)
            }
        })?;

        let status_text = output
            .status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        emit_status(
            progress,
            operation,
            format!("{command} exited with {status_text}"),
        );

        Ok(output)
    }

    async fn run_checked(
        &self,
        args: &[String],
        operation: ContainerRuntimeOperation,
        progress: Option<&ContainerRuntimeProgressSender>,
    ) -> Result<CommandOutput> {
        let output = self.run_unchecked(args, operation, progress).await?;
        if output.status == Some(0) {
            return Ok(output);
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.contains("No such container") || stderr.contains("No such object") {
            return Err(ContainerRuntimeError::NotFound(stderr));
        }

        Err(ContainerRuntimeError::CommandFailed {
            command: format_command(&self.program, args),
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr,
        })
    }

    async fn stage_injected_files(
        &self,
        name: &str,
        injected_files: &[InjectedFile],
    ) -> Result<Option<PathBuf>> {
        if injected_files.is_empty() {
            return Ok(None);
        }

        let directory = self.unique_temp_dir(name);
        fs::create_dir_all(&directory).await?;

        for (index, injected_file) in injected_files.iter().enumerate() {
            validate_container_path(&injected_file.container_path)?;
            let file_path = directory.join(format!("injected-{index}"));
            fs::write(file_path, &injected_file.content).await?;
        }

        Ok(Some(directory))
    }

    fn unique_temp_dir(&self, name: &str) -> PathBuf {
        let counter = self.unique_id.fetch_add(1, Ordering::Relaxed) + 1;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.temp_root
            .join(format!("mirage-container-{name}-{nanos}-{counter}"))
    }

    fn resource_limit_args(&self, resource_limits_json: &Option<String>) -> Result<Vec<String>> {
        let Some(json) = resource_limits_json else {
            return Ok(Vec::new());
        };
        let value: Value = serde_json::from_str(json)?;
        let object = value.as_object().ok_or_else(|| {
            ContainerRuntimeError::InvalidRequest(
                "resource_limits_json must be a JSON object".to_string(),
            )
        })?;

        let mut args = Vec::new();
        if let Some(cpu) = object.get("cpu") {
            args.push("--cpus".to_string());
            args.push(stringify_json_scalar(cpu, "cpu")?);
        }
        if let Some(memory) = object.get("memory") {
            args.push("--memory".to_string());
            args.push(stringify_json_scalar(memory, "memory")?);
        }
        Ok(args)
    }

    fn build_run_args(
        &self,
        request: &StartContainerRequest,
        injection_dir: Option<&PathBuf>,
    ) -> Result<Vec<String>> {
        if request.name.trim().is_empty() {
            return Err(ContainerRuntimeError::InvalidRequest(
                "container name must not be empty".to_string(),
            ));
        }
        if request.container.image.trim().is_empty() {
            return Err(ContainerRuntimeError::InvalidRequest(
                "container image must not be empty".to_string(),
            ));
        }
        if request.container.entrypoint.command.trim().is_empty() {
            return Err(ContainerRuntimeError::InvalidRequest(
                "container entrypoint command must not be empty".to_string(),
            ));
        }

        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            request.name.clone(),
            "--label".to_string(),
            "mirage.managed=true".to_string(),
        ];

        if request.container.privileged {
            args.push("--privileged".to_string());
        }

        for device in &request.container.devices {
            args.push("--device".to_string());
            args.push(device.clone());
        }

        if let Some(working_dir) = &request.container.working_dir {
            validate_container_path(working_dir)?;
            args.push("--workdir".to_string());
            args.push(working_dir.clone());
        }

        args.extend(self.resource_limit_args(&request.container.resource_limits_json)?);

        for mount in &request.container.mounts {
            validate_host_path(&mount.host_path)?;
            validate_container_path(&mount.container_path)?;
            args.push("-v".to_string());
            args.push(format!(
                "{}:{}:{}",
                mount.host_path,
                mount.container_path,
                if mount.readonly { "ro" } else { "rw" }
            ));
        }

        if let Some(directory) = injection_dir {
            for (index, injected_file) in request.container.injected_files.iter().enumerate() {
                args.push("-v".to_string());
                args.push(format!(
                    "{}:{}:{}",
                    directory.join(format!("injected-{index}")).display(),
                    injected_file.container_path,
                    if injected_file.readonly { "ro" } else { "rw" }
                ));
            }
        }

        for env in &request.container.entrypoint.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", env.key, env.value));
        }

        for port in &request.container.ports {
            args.push("-p".to_string());
            if port.host_port == 0 {
                args.push(format!(
                    "{}/{}",
                    port.container_port,
                    protocol_name(port.protocol)
                ));
            } else {
                args.push(format!(
                    "{}:{}/{}",
                    port.host_port,
                    port.container_port,
                    protocol_name(port.protocol)
                ));
            }
        }

        if let Some(network) = &request.container.network {
            args.push("--network".to_string());
            args.push(network.clone());
        }

        args.push("--entrypoint".to_string());
        args.push(request.container.entrypoint.command.clone());
        args.push(request.container.image.clone());
        args.extend(request.container.entrypoint.args.iter().cloned());
        Ok(args)
    }

    fn managed_port_labels(
        request: &StartContainerRequest,
    ) -> HashMap<(u16, Protocol), Option<String>> {
        request
            .container
            .ports
            .iter()
            .map(|port| ((port.container_port, port.protocol), port.label.clone()))
            .collect()
    }

    async fn cleanup_injection_dir(&self, container_id: &str) -> Result<()> {
        let injection_dir = self
            .managed
            .lock()
            .unwrap()
            .get(container_id)
            .and_then(|managed| managed.injection_dir.clone());
        if let Some(path) = injection_dir {
            match fs::remove_dir_all(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ContainerRuntimeError::Io(error)),
            }
        }
        self.managed.lock().unwrap().remove(container_id);
        Ok(())
    }

    fn parse_inspection(
        &self,
        handle: &ContainerHandle,
        stdout: &[u8],
    ) -> Result<ContainerInspection> {
        let entries: Vec<DockerInspectEntry> = serde_json::from_slice(stdout)?;
        let entry = entries.into_iter().next().ok_or_else(|| {
            ContainerRuntimeError::Parse("docker inspect returned an empty response".to_string())
        })?;

        let port_labels = self
            .managed
            .lock()
            .unwrap()
            .get(&handle.id)
            .map(|managed| managed.port_labels.clone())
            .unwrap_or_default();

        let mut ports = Vec::new();
        for (key, bindings) in entry.network_settings.ports {
            let (container_port, protocol) = parse_port_key(&key)?;
            let host_port = bindings
                .and_then(|bindings| bindings.into_iter().next())
                .map(|binding| binding.host_port.parse::<u16>())
                .transpose()
                .map_err(|error| ContainerRuntimeError::Parse(error.to_string()))?;
            ports.push(ResolvedPortMapping {
                container_port,
                host_port,
                protocol,
                label: port_labels
                    .get(&(container_port, protocol))
                    .cloned()
                    .flatten(),
            });
        }
        ports.sort_by(|left, right| {
            left.container_port
                .cmp(&right.container_port)
                .then_with(|| protocol_name(left.protocol).cmp(protocol_name(right.protocol)))
        });

        Ok(ContainerInspection {
            handle: ContainerHandle {
                id: entry.id,
                name: entry.name.trim_start_matches('/').to_string(),
            },
            image: entry.config.image,
            state: parse_state(&entry.state.status)?,
            exit_code: Some(entry.state.exit_code),
            ports,
        })
    }
}

#[async_trait]
impl<R> ContainerRuntime for DockerCli<R>
where
    R: CommandRunner + 'static,
{
    async fn pull_image(
        &self,
        image: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        if image.trim().is_empty() {
            return Err(ContainerRuntimeError::InvalidRequest(
                "container image must not be empty".to_string(),
            ));
        }
        self.run_checked(
            &["pull".to_string(), image.to_string()],
            ContainerRuntimeOperation::PullImage,
            progress.as_ref(),
        )
        .await?;
        Ok(())
    }

    async fn start_container(
        &self,
        request: StartContainerRequest,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<StartedContainer> {
        let injection_dir = self
            .stage_injected_files(&request.name, &request.container.injected_files)
            .await?;
        let args = match self.build_run_args(&request, injection_dir.as_ref()) {
            Ok(args) => args,
            Err(error) => {
                if let Some(path) = injection_dir {
                    let _ = fs::remove_dir_all(path).await;
                }
                return Err(error);
            }
        };

        let output = match self
            .run_checked(
                &args,
                ContainerRuntimeOperation::StartContainer,
                progress.as_ref(),
            )
            .await
        {
            Ok(output) => output,
            Err(error) => {
                if let Some(path) = injection_dir {
                    let _ = fs::remove_dir_all(path).await;
                }
                return Err(error);
            }
        };

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if container_id.is_empty() {
            if let Some(path) = injection_dir {
                let _ = fs::remove_dir_all(path).await;
            }
            return Err(ContainerRuntimeError::Parse(
                "docker run did not return a container id".to_string(),
            ));
        }

        self.managed.lock().unwrap().insert(
            container_id.clone(),
            ManagedContainer {
                injection_dir,
                port_labels: Self::managed_port_labels(&request),
            },
        );

        let inspection = self
            .inspect_container(
                &ContainerHandle {
                    id: container_id,
                    name: request.name,
                },
                progress.clone(),
            )
            .await?;

        Ok(StartedContainer { inspection })
    }

    async fn inspect_container(
        &self,
        handle: &ContainerHandle,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ContainerInspection> {
        let output = self
            .run_checked(
                &["inspect".to_string(), handle.id.clone()],
                ContainerRuntimeOperation::InspectContainer,
                progress.as_ref(),
            )
            .await?;
        self.parse_inspection(handle, &output.stdout)
    }

    async fn read_logs(
        &self,
        handle: &ContainerHandle,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ContainerLogs> {
        let output = self
            .run_checked(
                &["logs".to_string(), handle.id.clone()],
                ContainerRuntimeOperation::ReadLogs,
                progress.as_ref(),
            )
            .await?;
        Ok(ContainerLogs {
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    async fn exec(
        &self,
        request: ExecRequest,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<ExecResult> {
        if request.exec.command.trim().is_empty() {
            return Err(ContainerRuntimeError::InvalidRequest(
                "exec command must not be empty".to_string(),
            ));
        }

        let mut args = vec!["exec".to_string()];
        if let Some(working_dir) = &request.working_dir {
            validate_container_path(working_dir)?;
            args.push("--workdir".to_string());
            args.push(working_dir.clone());
        }
        for env in &request.exec.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", env.key, env.value));
        }
        args.push(request.container.id.clone());
        args.push(request.exec.command.clone());
        args.extend(request.exec.args.iter().cloned());

        let output = self
            .run_unchecked(&args, ContainerRuntimeOperation::Exec, progress.as_ref())
            .await?;
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        if stderr_text.contains("No such container") {
            return Err(ContainerRuntimeError::NotFound(
                stderr_text.trim().to_string(),
            ));
        }

        Ok(ExecResult {
            exit_code: output.status.unwrap_or_default(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    async fn stop_container(
        &self,
        handle: &ContainerHandle,
        timeout_secs: u32,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        self.run_checked(
            &[
                "stop".to_string(),
                "--time".to_string(),
                timeout_secs.to_string(),
                handle.id.clone(),
            ],
            ContainerRuntimeOperation::StopContainer,
            progress.as_ref(),
        )
        .await?;
        Ok(())
    }

    async fn remove_container(
        &self,
        handle: &ContainerHandle,
        force: bool,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        let mut args = vec!["rm".to_string()];
        if force {
            args.push("-f".to_string());
        }
        args.push(handle.id.clone());

        match self
            .run_checked(
                &args,
                ContainerRuntimeOperation::RemoveContainer,
                progress.as_ref(),
            )
            .await
        {
            Ok(_) => self.cleanup_injection_dir(&handle.id).await,
            Err(ContainerRuntimeError::NotFound(_)) => {
                self.cleanup_injection_dir(&handle.id).await?;
                Err(ContainerRuntimeError::NotFound(handle.id.clone()))
            }
            Err(error) => Err(error),
        }
    }

    async fn create_network(
        &self,
        name: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        if name.trim().is_empty() {
            return Err(ContainerRuntimeError::InvalidRequest(
                "network name must not be empty".to_string(),
            ));
        }
        self.run_checked(
            &[
                "network".to_string(),
                "create".to_string(),
                name.to_string(),
            ],
            ContainerRuntimeOperation::CreateNetwork,
            progress.as_ref(),
        )
        .await?;
        Ok(())
    }

    async fn remove_network(
        &self,
        name: &str,
        progress: Option<ContainerRuntimeProgressSender>,
    ) -> Result<()> {
        if name.trim().is_empty() {
            return Err(ContainerRuntimeError::InvalidRequest(
                "network name must not be empty".to_string(),
            ));
        }
        self.run_checked(
            &[
                "network".to_string(),
                "rm".to_string(),
                name.to_string(),
            ],
            ContainerRuntimeOperation::RemoveNetwork,
            progress.as_ref(),
        )
        .await?;
        Ok(())
    }
}

fn validate_host_path(path: &str) -> Result<()> {
    if Path::new(path).is_absolute() {
        Ok(())
    } else {
        Err(ContainerRuntimeError::InvalidRequest(format!(
            "host path '{path}' must be absolute"
        )))
    }
}

fn validate_container_path(path: &str) -> Result<()> {
    if Path::new(path).is_absolute() {
        Ok(())
    } else {
        Err(ContainerRuntimeError::InvalidRequest(format!(
            "container path '{path}' must be absolute"
        )))
    }
}

fn stringify_json_scalar(value: &Value, key: &str) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(ContainerRuntimeError::InvalidRequest(format!(
            "resource limit '{key}' must be a string or number"
        ))),
    }
}

fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
    }
}

fn format_command(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_port_key(value: &str) -> Result<(u16, Protocol)> {
    let (container_port, protocol) = value.split_once('/').ok_or_else(|| {
        ContainerRuntimeError::Parse(format!("invalid port mapping key '{value}'"))
    })?;
    let container_port = container_port
        .parse::<u16>()
        .map_err(|error| ContainerRuntimeError::Parse(error.to_string()))?;
    let protocol = match protocol {
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        other => {
            return Err(ContainerRuntimeError::Parse(format!(
                "unsupported protocol '{other}'"
            )));
        }
    };
    Ok((container_port, protocol))
}

fn parse_state(value: &str) -> Result<ContainerState> {
    match value {
        "created" => Ok(ContainerState::Created),
        "running" => Ok(ContainerState::Running),
        "exited" => Ok(ContainerState::Exited),
        "dead" => Ok(ContainerState::Dead),
        other => Err(ContainerRuntimeError::Parse(format!(
            "unsupported docker container state '{other}'"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct DockerInspectEntry {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Config")]
    config: DockerInspectConfig,
    #[serde(rename = "State")]
    state: DockerInspectState,
    #[serde(rename = "NetworkSettings")]
    network_settings: DockerNetworkSettings,
}

#[derive(Debug, Deserialize)]
struct DockerInspectConfig {
    #[serde(rename = "Image")]
    image: String,
}

#[derive(Debug, Deserialize)]
struct DockerInspectState {
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "ExitCode")]
    exit_code: i32,
}

#[derive(Debug, Deserialize)]
struct DockerNetworkSettings {
    #[serde(rename = "Ports", default)]
    ports: BTreeMap<String, Option<Vec<DockerPortBinding>>>,
}

#[derive(Debug, Deserialize)]
struct DockerPortBinding {
    #[serde(rename = "HostPort")]
    host_port: String,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use mirage_schema::common::{ExecArgs, SetEnv};
    use mirage_schema::container::{
        BindMount, ContainerDef, ContainerRuntimeEvent, ContainerRuntimeOperation, PortMapping,
        container_runtime_progress_channel,
    };

    use super::*;

    #[derive(Debug, Default)]
    struct FakeRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        responses: Mutex<VecDeque<std::io::Result<CommandOutput>>>,
    }

    impl FakeRunner {
        fn push_response(&self, response: std::io::Result<CommandOutput>) {
            self.responses.lock().unwrap().push_back(response);
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, program: &str, args: &[String]) -> std::io::Result<CommandOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(CommandOutput {
                        status: Some(0),
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                    })
                })
        }
    }

    fn temp_root(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mirage-container-tests-{test_name}"))
    }

    async fn cleanup_temp_root(path: &Path) {
        match fs::remove_dir_all(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove temp root {}: {error}", path.display()),
        }
    }

    fn drain_events(
        receiver: &mut mirage_schema::container::ContainerRuntimeProgressReceiver,
    ) -> Vec<ContainerRuntimeEvent> {
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        events
    }

    fn complex_request() -> StartContainerRequest {
        StartContainerRequest {
            name: "session-a-node0".to_string(),
            container: ContainerDef {
                image: "ghcr.io/example/runtime:latest".to_string(),
                mounts: vec![BindMount {
                    host_path: "/opt/rocjitsu/lib".to_string(),
                    container_path: "/usr/lib/rocjitsu".to_string(),
                    readonly: true,
                }],
                injected_files: vec![InjectedFile {
                    container_path: "/etc/mirage/config.json".to_string(),
                    content: br#"{"mode":"functional"}"#.to_vec(),
                    readonly: true,
                    label: Some("config".to_string()),
                }],
                entrypoint: ExecArgs {
                    command: "/bin/mirage".to_string(),
                    args: vec!["--serve".to_string(), "--foreground".to_string()],
                    env: vec![SetEnv {
                        key: "ROCJITSU_MODE".to_string(),
                        value: "functional".to_string(),
                    }],
                },
                working_dir: Some("/workspace".to_string()),
                ports: vec![
                    PortMapping {
                        container_port: 8080,
                        host_port: 0,
                        protocol: Protocol::Tcp,
                        label: Some("http".to_string()),
                    },
                    PortMapping {
                        container_port: 9000,
                        host_port: 19000,
                        protocol: Protocol::Udp,
                        label: Some("metrics".to_string()),
                    },
                ],
                privileged: true,
                devices: vec![],
                resource_limits_json: Some("{\"cpu\":\"4\",\"memory\":\"16Gi\"}".to_string()),
                network: None,
            },
        }
    }

    fn inspect_output() -> Vec<u8> {
        br#"[
            {
                "Id": "container-123",
                "Name": "/session-a-node0",
                "Config": { "Image": "ghcr.io/example/runtime:latest" },
                "State": { "Status": "running", "ExitCode": 0 },
                "NetworkSettings": {
                    "Ports": {
                        "8080/tcp": [{ "HostPort": "32768" }],
                        "9000/udp": [{ "HostPort": "19000" }]
                    }
                }
            }
        ]"#
        .to_vec()
    }

    #[tokio::test]
    async fn docker_cli_builds_expected_run_exec_and_lifecycle_commands() {
        let root = temp_root("run-lifecycle");
        cleanup_temp_root(&root).await;
        let runner = Arc::new(FakeRunner::default());
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: b"container-123\n".to_vec(),
            stderr: Vec::new(),
        }));
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: inspect_output(),
            stderr: Vec::new(),
        }));
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: b"out".to_vec(),
            stderr: b"err".to_vec(),
        }));
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }));
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }));

        let runtime = DockerCli::with_program_runner_and_temp_root("docker", runner.clone(), &root);
        let started = runtime
            .start_container(complex_request(), None)
            .await
            .unwrap();
        assert_eq!(started.inspection.handle.id, "container-123");
        assert_eq!(started.inspection.state, ContainerState::Running);
        assert_eq!(started.inspection.ports[0].container_port, 8080);
        assert_eq!(started.inspection.ports[0].host_port, Some(32768));
        assert_eq!(started.inspection.ports[0].label.as_deref(), Some("http"));
        assert_eq!(started.inspection.ports[1].host_port, Some(19000));

        let exec = runtime
            .exec(
                ExecRequest {
                    container: started.inspection.handle.clone(),
                    exec: ExecArgs {
                        command: "/bin/check".to_string(),
                        args: vec!["--status".to_string()],
                        env: vec![SetEnv {
                            key: "MODE".to_string(),
                            value: "check".to_string(),
                        }],
                    },
                    working_dir: Some("/tmp".to_string()),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(exec.exit_code, 0);
        assert_eq!(exec.stdout, b"out");
        assert_eq!(exec.stderr, b"err");

        runtime
            .stop_container(&started.inspection.handle, 9, None)
            .await
            .unwrap();
        runtime
            .remove_container(&started.inspection.handle, true, None)
            .await
            .unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0].0, "docker");
        assert_eq!(calls[0].1[0], "run");
        assert!(calls[0].1.contains(&"--privileged".to_string()));
        assert!(calls[0].1.contains(&"--cpus".to_string()));
        assert!(calls[0].1.contains(&"4".to_string()));
        assert!(calls[0].1.contains(&"--memory".to_string()));
        assert!(calls[0].1.contains(&"16Gi".to_string()));
        assert!(calls[0].1.contains(&"--entrypoint".to_string()));
        assert!(calls[0].1.contains(&"/bin/mirage".to_string()));
        assert!(
            calls[0]
                .1
                .contains(&"ghcr.io/example/runtime:latest".to_string())
        );
        assert!(calls[0].1.iter().any(|arg| arg == "/workspace"));
        assert!(
            calls[0]
                .1
                .iter()
                .any(|arg| arg.contains("/opt/rocjitsu/lib:/usr/lib/rocjitsu:ro"))
        );
        assert!(
            calls[0]
                .1
                .iter()
                .any(|arg| arg.ends_with(":/etc/mirage/config.json:ro"))
        );
        assert!(calls[0].1.iter().any(|arg| arg == "8080/tcp"));
        assert!(calls[0].1.iter().any(|arg| arg == "19000:9000/udp"));

        assert_eq!(
            calls[1].1,
            vec!["inspect".to_string(), "container-123".to_string()]
        );
        assert_eq!(calls[2].1[0], "exec");
        assert!(calls[2].1.contains(&"--workdir".to_string()));
        assert!(calls[2].1.contains(&"/tmp".to_string()));
        assert!(calls[2].1.contains(&"MODE=check".to_string()));
        assert!(calls[2].1.contains(&"/bin/check".to_string()));
        assert_eq!(
            calls[3].1,
            vec![
                "stop".to_string(),
                "--time".to_string(),
                "9".to_string(),
                "container-123".to_string()
            ]
        );
        assert_eq!(
            calls[4].1,
            vec![
                "rm".to_string(),
                "-f".to_string(),
                "container-123".to_string()
            ]
        );

        cleanup_temp_root(&root).await;
    }

    #[tokio::test]
    async fn docker_cli_reads_logs_and_cleans_injected_files_on_remove() {
        let root = temp_root("logs-cleanup");
        cleanup_temp_root(&root).await;
        let runner = Arc::new(FakeRunner::default());
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: b"container-123\n".to_vec(),
            stderr: Vec::new(),
        }));
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: inspect_output(),
            stderr: Vec::new(),
        }));
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: b"stdout".to_vec(),
            stderr: b"stderr".to_vec(),
        }));
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }));
        let runtime = DockerCli::with_program_runner_and_temp_root("docker", runner, &root);

        let started = runtime
            .start_container(complex_request(), None)
            .await
            .unwrap();
        let created_paths = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(created_paths.len(), 1);
        assert!(created_paths[0].join("injected-0").exists());

        let logs = runtime
            .read_logs(&started.inspection.handle, None)
            .await
            .unwrap();
        assert_eq!(logs.stdout, b"stdout");
        assert_eq!(logs.stderr, b"stderr");

        runtime
            .remove_container(&started.inspection.handle, true, None)
            .await
            .unwrap();
        let remaining = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert!(remaining.is_empty());

        cleanup_temp_root(&root).await;
    }

    #[tokio::test]
    async fn docker_cli_reports_invalid_requests_and_runtime_errors() {
        let root = temp_root("errors");
        cleanup_temp_root(&root).await;
        let runner = Arc::new(FakeRunner::default());
        let runtime = DockerCli::with_program_runner_and_temp_root("docker", runner.clone(), &root);

        let mut request = complex_request();
        request.container.resource_limits_json = Some("[]".to_string());
        let error = runtime.start_container(request, None).await.unwrap_err();
        assert!(matches!(error, ContainerRuntimeError::InvalidRequest(_)));

        let mut request = complex_request();
        request.container.mounts[0].host_path = "relative/path".to_string();
        let error = runtime.start_container(request, None).await.unwrap_err();
        assert!(matches!(error, ContainerRuntimeError::InvalidRequest(_)));

        runner.push_response(Ok(CommandOutput {
            status: Some(1),
            stdout: Vec::new(),
            stderr: b"No such container: container-404".to_vec(),
        }));
        let error = runtime
            .read_logs(
                &ContainerHandle {
                    id: "container-404".to_string(),
                    name: "missing".to_string(),
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ContainerRuntimeError::NotFound(_)));

        let unavailable_runner = Arc::new(FakeRunner::default());
        unavailable_runner.push_response(Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "docker not found",
        )));
        let unavailable =
            DockerCli::with_program_runner_and_temp_root("docker", unavailable_runner, &root);
        let error = unavailable
            .pull_image("ghcr.io/example/runtime:latest", None)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ContainerRuntimeError::RuntimeUnavailable(_)
        ));

        cleanup_temp_root(&root).await;
    }

    #[tokio::test]
    async fn docker_cli_parses_exited_state_and_exec_non_zero_exit_codes() {
        let root = temp_root("inspect-exit");
        cleanup_temp_root(&root).await;
        let runner = Arc::new(FakeRunner::default());
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: br#"[
                {
                    "Id": "container-7",
                    "Name": "/session-a-node0",
                    "Config": { "Image": "ghcr.io/example/runtime:latest" },
                    "State": { "Status": "exited", "ExitCode": 42 },
                    "NetworkSettings": { "Ports": {} }
                }
            ]"#
            .to_vec(),
            stderr: Vec::new(),
        }));
        runner.push_response(Ok(CommandOutput {
            status: Some(17),
            stdout: b"partial".to_vec(),
            stderr: b"failure".to_vec(),
        }));
        let runtime = DockerCli::with_program_runner_and_temp_root("docker", runner, &root);

        let inspection = runtime
            .inspect_container(
                &ContainerHandle {
                    id: "container-7".to_string(),
                    name: "session-a-node0".to_string(),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(inspection.state, ContainerState::Exited);
        assert_eq!(inspection.exit_code, Some(42));

        let exec = runtime
            .exec(
                ExecRequest {
                    container: inspection.handle,
                    exec: ExecArgs {
                        command: "/bin/check".to_string(),
                        args: Vec::new(),
                        env: Vec::new(),
                    },
                    working_dir: None,
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(exec.exit_code, 17);
        assert_eq!(exec.stdout, b"partial");
        assert_eq!(exec.stderr, b"failure");

        cleanup_temp_root(&root).await;
    }

    #[tokio::test]
    async fn docker_cli_streams_pull_progress_events() {
        let root = temp_root("pull-progress");
        cleanup_temp_root(&root).await;
        let runner = Arc::new(FakeRunner::default());
        runner.push_response(Ok(CommandOutput {
            status: Some(0),
            stdout: b"layer-1\nlayer-2\n".to_vec(),
            stderr: b"digest-check\n".to_vec(),
        }));
        let runtime = DockerCli::with_program_runner_and_temp_root("docker", runner, &root);
        let (progress, mut receiver) = container_runtime_progress_channel();

        runtime
            .pull_image("ghcr.io/example/runtime:latest", Some(progress))
            .await
            .unwrap();

        let events = drain_events(&mut receiver);
        assert!(events.iter().any(|event| matches!(
            event,
            ContainerRuntimeEvent::Status { operation: ContainerRuntimeOperation::PullImage, message }
                if message.contains("running docker pull ghcr.io/example/runtime:latest")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ContainerRuntimeEvent::Stdout { operation: ContainerRuntimeOperation::PullImage, chunk }
                if chunk == b"layer-1\nlayer-2\n"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ContainerRuntimeEvent::Stderr { operation: ContainerRuntimeOperation::PullImage, chunk }
                if chunk == b"digest-check\n"
        )));

        cleanup_temp_root(&root).await;
    }
}
