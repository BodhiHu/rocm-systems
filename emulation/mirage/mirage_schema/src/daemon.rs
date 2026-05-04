//! Transport and trait re-exports for the Mirage daemon control protocol.
//!
//! The protocol surface itself is defined in [`crate::ctl`]. This module keeps
//! the Unix-socket transport and the public compatibility re-exports that the
//! daemon implementation and client binaries consume.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::ctl::daemon::{
    self, DaemonInput, DaemonOutput, DaemonReply, DaemonRequest, DaemonTransport,
};
use crate::paths;

pub use crate::ctl::daemon::{
    AttachInput, AttachOutput, AttachReply, AttachRequest, BootReply, BootRequest,
    CreateProfileReply, CreateProfileRequest, CreateWorkloadReply, CreateWorkloadRequest,
    DeleteProfileReply, DeleteProfileRequest, DeleteWorkloadReply, DeleteWorkloadRequest,
    ExecReply, ExecRequest, GetOverviewReply, GetOverviewRequest, HealthReply, HealthRequest,
    ListProfilesReply, ListProfilesRequest, ListSessionsReply, ListSessionsRequest,
    ListSimulatorsReply, ListSimulatorsRequest, ListWorkloadsReply, ListWorkloadsRequest,
    RegisterSimReply, RegisterSimRequest, ShowSimulatorReply, ShowSimulatorRequest,
    ShowWorkloadReply, ShowWorkloadRequest, ShutdownReply, ShutdownRequest, StatusReply,
    StatusRequest, TimeReply, TimeRequest,
};
pub use crate::ctl::daemon::{
    DaemonCli as MirageDaemonCli, DaemonCommand as MirageDaemonCommand, DaemonImpl as MirageDaemon,
    ImplAttach as MirageDaemonAttach, ImplBoot as MirageDaemonBoot,
    ImplCreateProfile as MirageDaemonCreateProfile,
    ImplCreateWorkload as MirageDaemonCreateWorkload,
    ImplDeleteProfile as MirageDaemonDeleteProfile,
    ImplDeleteWorkload as MirageDaemonDeleteWorkload, ImplExec as MirageDaemonExec,
    ImplGetOverview as MirageDaemonOverview, ImplHealth as MirageDaemonHealth,
    ImplListProfiles as MirageDaemonListProfiles, ImplListSessions as MirageDaemonListSessions,
    ImplListSimulators as MirageDaemonListSimulators,
    ImplListWorkloads as MirageDaemonListWorkloads, ImplRegisterSim as MirageDaemonRegistration,
    ImplShowSimulator as MirageDaemonShowSimulator, ImplShowWorkload as MirageDaemonShowWorkload,
    ImplShutdown as MirageDaemonShutdown, ImplStatus as MirageDaemonStatus,
    ImplTime as MirageDaemonTime,
};
pub use crate::ctl::{
    MirageDaemonError, MirageDaemonResult, SessionSummary, SimulatorSummary, WorkloadSummary,
};

const MAX_FRAME_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
enum TransportFrame {
    Request(DaemonRequest),
    Input(DaemonInput),
    Output(DaemonOutput),
    Reply(DaemonReply),
    Error(String),
    EndInput,
}

impl TransportFrame {
    fn kind(&self) -> &'static str {
        match self {
            Self::Request(request) => request.kind(),
            Self::Input(input) => input.kind(),
            Self::Output(output) => output.kind(),
            Self::Reply(reply) => reply.kind(),
            Self::Error(_) => "error",
            Self::EndInput => "end-input",
        }
    }
}

/// Unix-socket client for the generated daemon control protocol.
#[derive(Debug, Clone)]
pub struct MirageDaemonClient {
    socket_path: PathBuf,
}

impl MirageDaemonClient {
    /// Creates a client that talks to the daemon listening on `socket_path`.
    pub fn new<P>(socket_path: P) -> Self
    where
        P: Into<PathBuf>,
    {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// Creates a client that talks to the default local daemon socket.
    pub fn local() -> Self {
        Self::new(paths::socket_path())
    }

    /// Returns the socket path used by this client.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    async fn read_reply_frames(
        mut reader: tokio::net::unix::OwnedReadHalf,
        output: Option<mpsc::Sender<DaemonOutput>>,
    ) -> MirageDaemonResult<DaemonReply> {
        let output = output;
        loop {
            match read_frame::<_, TransportFrame>(&mut reader).await? {
                TransportFrame::Output(value) => {
                    let Some(sender) = &output else {
                        return Err(MirageDaemonError::protocol(format!(
                            "received unexpected {} output frame",
                            value.kind()
                        )));
                    };
                    sender.send(value).await.map_err(|_| {
                        MirageDaemonError::protocol(
                            "output receiver dropped before daemon output could be delivered",
                        )
                    })?;
                }
                TransportFrame::Reply(reply) => return Ok(reply),
                TransportFrame::Error(message) => return Err(MirageDaemonError::Remote(message)),
                other => {
                    return Err(MirageDaemonError::protocol(format!(
                        "expected output or reply frame but received {}",
                        other.kind()
                    )));
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl DaemonTransport for MirageDaemonClient {
    async fn transport_call(
        &self,
        request: DaemonRequest,
        input: Option<mpsc::Receiver<DaemonInput>>,
        output: Option<mpsc::Sender<DaemonOutput>>,
    ) -> MirageDaemonResult<DaemonReply> {
        let stream = UnixStream::connect(&self.socket_path).await?;
        let (reader, mut writer) = stream.into_split();

        write_frame(&mut writer, &TransportFrame::Request(request)).await?;

        let write_task = tokio::spawn(async move {
            if let Some(mut input) = input {
                while let Some(value) = input.recv().await {
                    write_frame(&mut writer, &TransportFrame::Input(value)).await?;
                }
            }
            write_frame(&mut writer, &TransportFrame::EndInput).await
        });

        let read_result = Self::read_reply_frames(reader, output).await;
        if read_result.is_err() {
            write_task.abort();
        }

        match write_task.await {
            Ok(write_result) => write_result?,
            Err(error) if error.is_cancelled() => {}
            Err(error) => {
                return Err(MirageDaemonError::protocol(format!(
                    "client writer task failed: {error}"
                )));
            }
        }

        read_result
    }
}

/// Unix-socket server for the generated daemon control protocol.
#[derive(Clone)]
pub struct MirageDaemonServer {
    daemon: Arc<dyn MirageDaemon>,
    socket_path: PathBuf,
}

impl std::fmt::Debug for MirageDaemonServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirageDaemonServer")
            .field("socket_path", &self.socket_path)
            .finish_non_exhaustive()
    }
}

impl MirageDaemonServer {
    /// Creates a server that serves `daemon` on `socket_path`.
    pub fn new<P, D>(socket_path: P, daemon: D) -> Self
    where
        P: Into<PathBuf>,
        D: MirageDaemon + 'static,
    {
        Self::from_arc(socket_path, Arc::new(daemon))
    }

    /// Creates a server bound to the default local daemon socket.
    pub fn local<D>(daemon: D) -> Self
    where
        D: MirageDaemon + 'static,
    {
        Self::new(paths::socket_path(), daemon)
    }

    /// Creates a server from an already shared daemon instance.
    pub fn from_arc<P>(socket_path: P, daemon: Arc<dyn MirageDaemon>) -> Self
    where
        P: Into<PathBuf>,
    {
        Self {
            daemon,
            socket_path: socket_path.into(),
        }
    }

    /// Returns the socket path served by this server.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Serves daemon requests until the process exits.
    pub async fn serve(&self) -> MirageDaemonResult<()> {
        if let Some(parent) = self.socket_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        match fs::remove_file(&self.socket_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        loop {
            let (stream, _) = listener.accept().await?;
            let daemon = Arc::clone(&self.daemon);
            tokio::spawn(async move {
                if let Err(error) = Self::handle_client(daemon, stream).await {
                    tracing::warn!(%error, "mirage daemon connection failed");
                }
            });
        }
    }

    async fn handle_client(
        daemon: Arc<dyn MirageDaemon>,
        stream: UnixStream,
    ) -> MirageDaemonResult<()> {
        let (mut reader, writer) = stream.into_split();
        let request = match read_frame::<_, TransportFrame>(&mut reader).await? {
            TransportFrame::Request(request) => request,
            other => {
                return Err(MirageDaemonError::protocol(format!(
                    "expected request frame but received {}",
                    other.kind()
                )));
            }
        };

        let (transport_input_tx, transport_input_rx) = mpsc::channel(16);
        let (transport_output_tx, mut transport_output_rx) = mpsc::channel(16);
        let (frame_tx, mut frame_rx) = mpsc::channel::<TransportFrame>(16);

        let writer_task = tokio::spawn(async move {
            let mut writer = writer;
            while let Some(frame) = frame_rx.recv().await {
                write_frame(&mut writer, &frame).await?;
            }
            Ok::<(), MirageDaemonError>(())
        });

        let output_sender = frame_tx.clone();
        let output_task = tokio::spawn(async move {
            while let Some(output) = transport_output_rx.recv().await {
                output_sender
                    .send(TransportFrame::Output(output))
                    .await
                    .map_err(|_| {
                        MirageDaemonError::protocol(
                            "frame writer dropped before daemon output could be forwarded",
                        )
                    })?;
            }
            Ok::<(), MirageDaemonError>(())
        });

        let input_task = tokio::spawn(async move {
            loop {
                match read_frame::<_, TransportFrame>(&mut reader).await? {
                    TransportFrame::Input(input) => {
                        transport_input_tx.send(input).await.map_err(|_| {
                            MirageDaemonError::protocol(
                                "daemon input receiver dropped before all client input was read",
                            )
                        })?
                    }
                    TransportFrame::EndInput => break,
                    other => {
                        return Err(MirageDaemonError::protocol(format!(
                            "expected input or end-input frame but received {}",
                            other.kind()
                        )));
                    }
                }
            }
            Ok::<(), MirageDaemonError>(())
        });

        let reply = daemon::dispatch_transport(
            &*daemon,
            request,
            Some(transport_input_rx),
            Some(transport_output_tx),
        )
        .await;

        output_task.await.map_err(|error| {
            MirageDaemonError::protocol(format!("server output task failed: {error}"))
        })??;

        input_task.abort();
        match input_task.await {
            Ok(result) => result?,
            Err(error) if error.is_cancelled() => {}
            Err(error) => {
                return Err(MirageDaemonError::protocol(format!(
                    "server input task failed: {error}"
                )));
            }
        }

        frame_tx
            .send(match reply {
                Ok(reply) => TransportFrame::Reply(reply),
                Err(error) => TransportFrame::Error(error.to_string()),
            })
            .await
            .map_err(|_| {
                MirageDaemonError::protocol(
                    "frame writer dropped before the final daemon reply could be sent",
                )
            })?;
        drop(frame_tx);

        writer_task.await.map_err(|error| {
            MirageDaemonError::protocol(format!("server writer task failed: {error}"))
        })??;
        Ok(())
    }
}

async fn write_frame<W, T>(writer: &mut W, value: &T) -> MirageDaemonResult<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value)?;
    let len: u32 = payload
        .len()
        .try_into()
        .map_err(|_| MirageDaemonError::protocol("rpc frame exceeds u32 length limit"))?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_frame<R, T>(reader: &mut R) -> MirageDaemonResult<T>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len_bytes = [0_u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    if len > MAX_FRAME_LEN {
        return Err(MirageDaemonError::protocol(format!(
            "rpc frame of {len} bytes exceeds maximum supported size of {MAX_FRAME_LEN} bytes"
        )));
    }
    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::common::{CleanupPolicy, HealthStatus, ProfileDef, SimulatorMode, WorkloadDef};

    #[derive(Debug)]
    struct FixedDaemon;

    #[async_trait::async_trait]
    impl MirageDaemonHealth for FixedDaemon {
        async fn health(&self, _request: HealthRequest) -> MirageDaemonResult<HealthReply> {
            Ok(HealthReply {
                healthy: true,
                status: HealthStatus::Healthy,
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonTime for FixedDaemon {
        async fn time(&self, _request: TimeRequest) -> MirageDaemonResult<TimeReply> {
            Ok(TimeReply {
                time: Default::default(),
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonAttach for FixedDaemon {
        async fn attach(
            &self,
            request: AttachRequest,
            mut input: mpsc::Receiver<AttachInput>,
            output: mpsc::Sender<AttachOutput>,
        ) -> MirageDaemonResult<AttachReply> {
            assert_eq!(request.exec_id, "exec-1");
            let mut stdin = Vec::new();
            while let Some(chunk) = input.recv().await {
                stdin.extend(chunk.stream);
            }
            assert_eq!(stdin, b"stdin-data".to_vec());

            output
                .send(AttachOutput {
                    is_stdout: true,
                    output: b"stdout".to_vec(),
                })
                .await
                .unwrap();
            output
                .send(AttachOutput {
                    is_stdout: false,
                    output: b"stderr".to_vec(),
                })
                .await
                .unwrap();

            Ok(AttachReply { exit_code: 0 })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonRegistration for FixedDaemon {
        async fn register_sim(
            &self,
            _request: RegisterSimRequest,
        ) -> MirageDaemonResult<RegisterSimReply> {
            Ok(RegisterSimReply {
                ok: true,
                error: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonOverview for FixedDaemon {
        async fn get_overview(
            &self,
            _request: GetOverviewRequest,
        ) -> MirageDaemonResult<GetOverviewReply> {
            Ok(GetOverviewReply {
                simulator_count: 1,
                profile_count: 0,
                session_count: 0,
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonListSimulators for FixedDaemon {
        async fn list_simulators(
            &self,
            _request: ListSimulatorsRequest,
        ) -> MirageDaemonResult<ListSimulatorsReply> {
            Ok(ListSimulatorsReply {
                simulators: vec![SimulatorSummary {
                    name: Some("rocjitsu".to_string()),
                    version: Some("1.0.0".to_string()),
                    description: Some("functional simulator".to_string()),
                    supported_gpus: vec![],
                    supports_custom_gpus: false,
                    supported_modes: vec![SimulatorMode::Functional],
                    active_session_count: 0,
                }],
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonShowSimulator for FixedDaemon {
        async fn show_simulator(
            &self,
            request: ShowSimulatorRequest,
        ) -> MirageDaemonResult<ShowSimulatorReply> {
            Ok(ShowSimulatorReply {
                simulator: (request.name == "rocjitsu").then(|| SimulatorSummary {
                    name: Some("rocjitsu".to_string()),
                    version: Some("1.0.0".to_string()),
                    description: Some("functional simulator".to_string()),
                    supported_gpus: vec![],
                    supports_custom_gpus: false,
                    supported_modes: vec![SimulatorMode::Functional],
                    active_session_count: 0,
                }),
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonListProfiles for FixedDaemon {
        async fn list_profiles(
            &self,
            _request: ListProfilesRequest,
        ) -> MirageDaemonResult<ListProfilesReply> {
            Ok(ListProfilesReply { profiles: vec![] })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonCreateProfile for FixedDaemon {
        async fn create_profile(
            &self,
            _request: CreateProfileRequest,
        ) -> MirageDaemonResult<CreateProfileReply> {
            Ok(CreateProfileReply {
                ok: true,
                error: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonDeleteProfile for FixedDaemon {
        async fn delete_profile(
            &self,
            _request: DeleteProfileRequest,
        ) -> MirageDaemonResult<DeleteProfileReply> {
            Ok(DeleteProfileReply {
                ok: true,
                error: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonListSessions for FixedDaemon {
        async fn list_sessions(
            &self,
            _request: ListSessionsRequest,
        ) -> MirageDaemonResult<ListSessionsReply> {
            Ok(ListSessionsReply { sessions: vec![] })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonStatus for FixedDaemon {
        async fn status(&self, _request: StatusRequest) -> MirageDaemonResult<StatusReply> {
            Ok(StatusReply {
                name: None,
                profile: None,
                simulator: None,
                image: None,
                health: HealthStatus::Unknown,
                uptime: None,
                error_message: None,
                ticks: 0,
                ipc: 0.0,
                simulation_speed: 0.0,
                active_contexts: 0,
                phase: crate::common::SessionPhase::default(),
                progress_message: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonBoot for FixedDaemon {
        async fn boot(&self, _request: BootRequest) -> MirageDaemonResult<BootReply> {
            Ok(BootReply {
                ok: true,
                error: None,
                container_id: Some("mock-container-id".to_string()),
                container_ids: vec!["mock-container-id".to_string()],
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonExec for FixedDaemon {
        async fn exec(
            &self,
            _request: crate::ctl::daemon::ExecRequest,
        ) -> MirageDaemonResult<crate::ctl::daemon::ExecReply> {
            Ok(crate::ctl::daemon::ExecReply {
                exec_id: "mock-exec-id".to_string(),
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonShutdown for FixedDaemon {
        async fn shutdown(&self, _request: ShutdownRequest) -> MirageDaemonResult<ShutdownReply> {
            Ok(ShutdownReply {
                ok: true,
                error: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonCreateWorkload for FixedDaemon {
        async fn create_workload(
            &self,
            _request: CreateWorkloadRequest,
        ) -> MirageDaemonResult<CreateWorkloadReply> {
            Ok(CreateWorkloadReply {
                ok: true,
                error: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonListWorkloads for FixedDaemon {
        async fn list_workloads(
            &self,
            _request: ListWorkloadsRequest,
        ) -> MirageDaemonResult<ListWorkloadsReply> {
            Ok(ListWorkloadsReply { workloads: vec![] })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonShowWorkload for FixedDaemon {
        async fn show_workload(
            &self,
            _request: ShowWorkloadRequest,
        ) -> MirageDaemonResult<ShowWorkloadReply> {
            Ok(ShowWorkloadReply { workload: None })
        }
    }

    #[async_trait::async_trait]
    impl MirageDaemonDeleteWorkload for FixedDaemon {
        async fn delete_workload(
            &self,
            _request: DeleteWorkloadRequest,
        ) -> MirageDaemonResult<DeleteWorkloadReply> {
            Ok(DeleteWorkloadReply {
                ok: true,
                error: None,
            })
        }
    }

    fn unique_socket_path(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mirage_daemon_{label}_{}_{}.sock",
            std::process::id(),
            nanos
        ))
    }

    #[tokio::test]
    async fn client_round_trips_list_simulators() {
        let socket_path = unique_socket_path("list_simulators");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let daemon: Arc<dyn MirageDaemon> = Arc::new(FixedDaemon);

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            MirageDaemonServer::handle_client(daemon, stream)
                .await
                .unwrap();
        });

        let client = MirageDaemonClient::new(&socket_path);
        let reply = client
            .list_simulators(ListSimulatorsRequest {})
            .await
            .unwrap();

        assert_eq!(reply.simulators.len(), 1);
        assert_eq!(reply.simulators[0].name.as_deref(), Some("rocjitsu"));

        server_task.await.unwrap();
        let _ = fs::remove_file(&socket_path).await;
    }

    #[tokio::test]
    async fn client_round_trips_get_overview() {
        let socket_path = unique_socket_path("overview");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let daemon: Arc<dyn MirageDaemon> = Arc::new(FixedDaemon);

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            MirageDaemonServer::handle_client(daemon, stream)
                .await
                .unwrap();
        });

        let client = MirageDaemonClient::new(&socket_path);
        let reply = client.get_overview(GetOverviewRequest {}).await.unwrap();

        assert_eq!(reply.simulator_count, 1);
        assert_eq!(reply.profile_count, 0);
        assert_eq!(reply.session_count, 0);

        server_task.await.unwrap();
        let _ = fs::remove_file(&socket_path).await;
    }

    #[tokio::test]
    async fn client_round_trips_streaming_attach() {
        let socket_path = unique_socket_path("attach");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let daemon: Arc<dyn MirageDaemon> = Arc::new(FixedDaemon);

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            MirageDaemonServer::handle_client(daemon, stream)
                .await
                .unwrap();
        });

        let client = MirageDaemonClient::new(&socket_path);
        let (input_tx, input_rx) = mpsc::channel(4);
        let (output_tx, mut output_rx) = mpsc::channel(4);
        input_tx
            .send(AttachInput {
                stream: b"stdin-data".to_vec(),
            })
            .await
            .unwrap();
        drop(input_tx);

        let reply = client
            .attach(
                AttachRequest {
                    exec_id: "exec-1".to_string(),
                },
                input_rx,
                output_tx,
            )
            .await
            .unwrap();

        let mut outputs = Vec::new();
        while let Some(output) = output_rx.recv().await {
            outputs.push(output);
        }

        assert_eq!(reply.exit_code, 0);
        assert_eq!(outputs.len(), 2);
        assert!(outputs[0].is_stdout);
        assert_eq!(outputs[0].output, b"stdout".to_vec());
        assert!(!outputs[1].is_stdout);
        assert_eq!(outputs[1].output, b"stderr".to_vec());

        server_task.await.unwrap();
        let _ = fs::remove_file(&socket_path).await;
    }

    #[test]
    fn generated_types_stay_serde_friendly() {
        let profile = ProfileDef {
            name: "default".to_string(),
            simulator: "rocjitsu".to_string(),
            mode: SimulatorMode::Functional,
            gpu: "MI300X".to_string(),
            num_gpus: 1,
            num_nodes: 1,
        };
        let workload = WorkloadDef {
            name: "smoke".to_string(),
            profile: "default".to_string(),
            image: "img:latest".to_string(),
            startup: None,
            execs: vec![],
            cleanup: CleanupPolicy::Always,
        };

        let simulator_json = serde_json::to_string(&SimulatorSummary {
            name: Some("rocjitsu".to_string()),
            version: Some("1.0.0".to_string()),
            description: None,
            supported_gpus: vec![],
            supports_custom_gpus: false,
            supported_modes: vec![SimulatorMode::Functional],
            active_session_count: 0,
        })
        .unwrap();
        let workload_json = serde_json::to_string(&workload).unwrap();

        assert!(simulator_json.contains("rocjitsu"));
        assert!(workload_json.contains("smoke"));
        assert_eq!(profile.name, "default");
    }
}
