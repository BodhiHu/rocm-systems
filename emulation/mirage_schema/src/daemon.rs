use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::paths;
use crate::socket::{
    AttachReply, AttachRequest, BootSessionReply, BootSessionRequest, CreateProfileReply,
    CreateProfileRequest, DashboardCreateSessionReply, DashboardCreateSessionRequest,
    DashboardDeleteSessionReply, DashboardDeleteSessionRequest, DeleteProfileReply,
    DeleteProfileRequest, ExecInSessionReply, ExecInSessionRequest, GetOverviewReply,
    GetOverviewRequest, GetSessionDetailReply, GetSessionDetailRequest, GetSimulatorReply,
    GetSimulatorRequest, HealthReply, HealthRequest, ListProfilesReply, ListProfilesRequest,
    ListSessionsReply, ListSessionsRequest, ListSimulatorsReply, ListSimulatorsRequest,
    RegisterSimReply, RegisterSimRequest, ShutdownSessionReply, ShutdownSessionRequest, TimeReply,
    TimeRequest,
};

const MAX_FRAME_LEN: usize = 644 * 1024 * 1024;

pub type MirageDaemonResult<T> = Result<T, MirageDaemonError>;

#[derive(Debug)]
pub enum MirageDaemonError {
    Io(io::Error),
    Serialization(serde_json::Error),
    Protocol(String),
    Remote(String),
}

impl MirageDaemonError {
    fn protocol(message: impl Into<String>) -> Self {
        Self::Protocol(message.into())
    }
}

impl fmt::Display for MirageDaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Serialization(error) => write!(f, "serialization error: {error}"),
            Self::Protocol(message) => write!(f, "protocol error: {message}"),
            Self::Remote(message) => write!(f, "remote error: {message}"),
        }
    }
}

impl std::error::Error for MirageDaemonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::Protocol(_) | Self::Remote(_) => None,
        }
    }
}

impl From<io::Error> for MirageDaemonError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for MirageDaemonError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value)
    }
}

macro_rules! mirage_daemon_rpcs {
    (
        $(
            $trait_name:ident {
                $(
                    $method:ident($request_ty:ty) -> $reply_ty:ty => $variant:ident;
                )*
            }
        )*
    ) => {
        $(
            #[async_trait]
            pub trait $trait_name: Send + Sync {
                $(
                    async fn $method(
                        &self,
                        request: $request_ty,
                    ) -> MirageDaemonResult<$reply_ty>;
                )*
            }
        )*

        pub trait MirageDaemon: $( $trait_name + )* Send + Sync {}

        impl<T> MirageDaemon for T where T: $( $trait_name + )* Send + Sync {}

        #[derive(Debug, Serialize, Deserialize)]
        enum MirageDaemonRequest {
            $(
                $(
                    $variant($request_ty),
                )*
            )*
        }

        #[derive(Debug, Serialize, Deserialize)]
        enum MirageDaemonResponse {
            $(
                $(
                    $variant(RpcResult<$reply_ty>),
                )*
            )*
        }

        impl MirageDaemonResponse {
            fn kind(&self) -> &'static str {
                match self {
                    $(
                        $(
                            Self::$variant(_) => stringify!($variant),
                        )*
                    )*
                }
            }
        }

        $(
            #[async_trait]
            impl $trait_name for MirageDaemonClient {
                $(
                    async fn $method(
                        &self,
                        request: $request_ty,
                    ) -> MirageDaemonResult<$reply_ty> {
                        let response = self.send(MirageDaemonRequest::$variant(request)).await?;
                        match response {
                            MirageDaemonResponse::$variant(result) => result.into_result(),
                            other => Err(Self::unexpected_response(stringify!($variant), &other)),
                        }
                    }
                )*
            }
        )*

        async fn dispatch_request(
            daemon: &dyn MirageDaemon,
            request: MirageDaemonRequest,
        ) -> MirageDaemonResponse {
            match request {
                $(
                    $(
                        MirageDaemonRequest::$variant(request) => {
                            MirageDaemonResponse::$variant(to_rpc_result(daemon.$method(request).await))
                        }
                    )*
                )*
            }
        }
    };
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
struct RpcResult<T> {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<T> RpcResult<T> {
    fn ok(value: T) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(error.into()),
        }
    }

    fn into_result(self) -> MirageDaemonResult<T> {
        if self.ok {
            self.value.ok_or_else(|| {
                MirageDaemonError::protocol("successful rpc response missing payload")
            })
        } else {
            Err(MirageDaemonError::Remote(
                self.error
                    .unwrap_or_else(|| "remote request failed".to_string()),
            ))
        }
    }
}

mirage_daemon_rpcs! {
    MirageDaemonHealth {
        health(HealthRequest) -> HealthReply => Health;
    }
    MirageDaemonTime {
        time(TimeRequest) -> TimeReply => Time;
    }
    MirageDaemonAttach {
        attach(AttachRequest) -> Vec<AttachReply> => Attach;
    }
    MirageDaemonRegistration {
        register_sim(RegisterSimRequest) -> RegisterSimReply => RegisterSim;
    }
    MirageDaemonOverview {
        get_overview(GetOverviewRequest) -> GetOverviewReply => GetOverview;
    }
    MirageDaemonSimulators {
        list_simulators(ListSimulatorsRequest) -> ListSimulatorsReply => ListSimulators;
        get_simulator(GetSimulatorRequest) -> GetSimulatorReply => GetSimulator;
    }
    MirageDaemonProfiles {
        list_profiles(ListProfilesRequest) -> ListProfilesReply => ListProfiles;
        create_profile(CreateProfileRequest) -> CreateProfileReply => CreateProfile;
        delete_profile(DeleteProfileRequest) -> DeleteProfileReply => DeleteProfile;
    }
    MirageDaemonSessions {
        list_sessions(ListSessionsRequest) -> ListSessionsReply => ListSessions;
        create_session(DashboardCreateSessionRequest) -> DashboardCreateSessionReply => CreateSession;
        delete_session(DashboardDeleteSessionRequest) -> DashboardDeleteSessionReply => DeleteSession;
        get_session_detail(GetSessionDetailRequest) -> GetSessionDetailReply => GetSessionDetail;
    }
    MirageDaemonBoot {
        boot_session(BootSessionRequest) -> BootSessionReply => BootSession;
    }
    MirageDaemonExec {
        exec_in_session(ExecInSessionRequest) -> ExecInSessionReply => ExecInSession;
    }
    MirageDaemonShutdown {
        shutdown_session(ShutdownSessionRequest) -> ShutdownSessionReply => ShutdownSession;
    }
}

#[derive(Debug, Clone)]
pub struct MirageDaemonClient {
    socket_path: PathBuf,
}

impl MirageDaemonClient {
    pub fn new<P>(socket_path: P) -> Self
    where
        P: Into<PathBuf>,
    {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn local() -> Self {
        Self::new(paths::socket_path())
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    async fn send(&self, request: MirageDaemonRequest) -> MirageDaemonResult<MirageDaemonResponse> {
        let mut stream = UnixStream::connect(&self.socket_path).await?;
        write_frame(&mut stream, &request).await?;
        read_frame(&mut stream).await
    }

    fn unexpected_response(
        expected: &'static str,
        actual: &MirageDaemonResponse,
    ) -> MirageDaemonError {
        MirageDaemonError::protocol(format!(
            "expected {expected} response but received {}",
            actual.kind()
        ))
    }
}

#[derive(Clone)]
pub struct MirageDaemonServer {
    daemon: Arc<dyn MirageDaemon>,
    socket_path: PathBuf,
}

impl fmt::Debug for MirageDaemonServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MirageDaemonServer")
            .field("socket_path", &self.socket_path)
            .finish_non_exhaustive()
    }
}

impl MirageDaemonServer {
    pub fn new<P, D>(socket_path: P, daemon: D) -> Self
    where
        P: Into<PathBuf>,
        D: MirageDaemon + 'static,
    {
        Self::from_arc(socket_path, Arc::new(daemon))
    }

    pub fn local<D>(daemon: D) -> Self
    where
        D: MirageDaemon + 'static,
    {
        Self::new(paths::socket_path(), daemon)
    }

    pub fn from_arc<P>(socket_path: P, daemon: Arc<dyn MirageDaemon>) -> Self
    where
        P: Into<PathBuf>,
    {
        Self {
            daemon,
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub async fn serve(&self) -> MirageDaemonResult<()> {
        if let Some(parent) = self.socket_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        match fs::remove_file(&self.socket_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        loop {
            let (stream, _) = listener.accept().await?;
            let daemon = Arc::clone(&self.daemon);
            tokio::spawn(async move {
                if let Err(error) = Self::handle_client(daemon, stream).await {
                    eprintln!("mirage daemon connection failed: {error}");
                }
            });
        }
    }

    async fn handle_client(
        daemon: Arc<dyn MirageDaemon>,
        mut stream: UnixStream,
    ) -> MirageDaemonResult<()> {
        let request: MirageDaemonRequest = read_frame(&mut stream).await?;
        let response = dispatch_request(&*daemon, request).await;
        write_frame(&mut stream, &response).await
    }
}

fn to_rpc_result<T>(result: MirageDaemonResult<T>) -> RpcResult<T> {
    match result {
        Ok(value) => RpcResult::ok(value),
        Err(error) => RpcResult::err(error.to_string()),
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
    T: DeserializeOwned,
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

    use crate::common::{GpuDef, GpuFamily, HealthStatus, SimulatorMode};
    use crate::simulator::SimulatorInfo;
    use crate::socket::SimulatorSummary;

    #[derive(Debug)]
    struct FixedDaemon;

    #[async_trait]
    impl MirageDaemonHealth for FixedDaemon {
        async fn health(&self, _request: HealthRequest) -> MirageDaemonResult<HealthReply> {
            Ok(HealthReply {
                healthy: true,
                status: HealthStatus::Healthy,
            })
        }
    }

    #[async_trait]
    impl MirageDaemonTime for FixedDaemon {
        async fn time(&self, _request: TimeRequest) -> MirageDaemonResult<TimeReply> {
            Ok(TimeReply {
                time: Default::default(),
            })
        }
    }

    #[async_trait]
    impl MirageDaemonAttach for FixedDaemon {
        async fn attach(&self, _request: AttachRequest) -> MirageDaemonResult<Vec<AttachReply>> {
            Ok(vec![])
        }
    }

    #[async_trait]
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

    #[async_trait]
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

    #[async_trait]
    impl MirageDaemonSimulators for FixedDaemon {
        async fn list_simulators(
            &self,
            _request: ListSimulatorsRequest,
        ) -> MirageDaemonResult<ListSimulatorsReply> {
            Ok(ListSimulatorsReply {
                simulators: vec![SimulatorSummary {
                    name: Some("rocjitsu".to_string()),
                    version: Some("1.0.0".to_string()),
                    description: Some("functional simulator".to_string()),
                    supported_gpus: vec![GpuDef {
                        name: "MI300X".to_string(),
                        arch: "gfx942".to_string(),
                        family: GpuFamily::AmdCdna,
                        description: None,
                    }],
                    supports_custom_gpus: false,
                    supported_modes: vec![SimulatorMode::Functional],
                    active_session_count: 0,
                }],
            })
        }

        async fn get_simulator(
            &self,
            request: GetSimulatorRequest,
        ) -> MirageDaemonResult<GetSimulatorReply> {
            let info = SimulatorInfo {
                name: "rocjitsu".to_string(),
                version: "1.0.0".to_string(),
                description: Some("functional simulator".to_string()),
                supported_gpus: vec![GpuDef {
                    name: "MI300X".to_string(),
                    arch: "gfx942".to_string(),
                    family: GpuFamily::AmdCdna,
                    description: None,
                }],
                supports_custom_gpus: false,
                supported_modes: vec![SimulatorMode::Functional],
            };
            let simulator = (request.name == info.name).then(|| SimulatorSummary {
                name: Some(info.name),
                version: Some(info.version),
                description: info.description,
                supported_gpus: info.supported_gpus,
                supports_custom_gpus: info.supports_custom_gpus,
                supported_modes: info.supported_modes,
                active_session_count: 0,
            });
            Ok(GetSimulatorReply { simulator })
        }
    }

    #[async_trait]
    impl MirageDaemonProfiles for FixedDaemon {
        async fn list_profiles(
            &self,
            _request: ListProfilesRequest,
        ) -> MirageDaemonResult<ListProfilesReply> {
            Ok(ListProfilesReply { profiles: vec![] })
        }

        async fn create_profile(
            &self,
            _request: CreateProfileRequest,
        ) -> MirageDaemonResult<CreateProfileReply> {
            Ok(CreateProfileReply {
                ok: true,
                error: None,
            })
        }

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

    #[async_trait]
    impl MirageDaemonSessions for FixedDaemon {
        async fn list_sessions(
            &self,
            _request: ListSessionsRequest,
        ) -> MirageDaemonResult<ListSessionsReply> {
            Ok(ListSessionsReply { sessions: vec![] })
        }

        async fn create_session(
            &self,
            _request: DashboardCreateSessionRequest,
        ) -> MirageDaemonResult<DashboardCreateSessionReply> {
            Ok(DashboardCreateSessionReply {
                ok: true,
                error: None,
            })
        }

        async fn delete_session(
            &self,
            _request: DashboardDeleteSessionRequest,
        ) -> MirageDaemonResult<DashboardDeleteSessionReply> {
            Ok(DashboardDeleteSessionReply {
                ok: true,
                error: None,
            })
        }

        async fn get_session_detail(
            &self,
            _request: GetSessionDetailRequest,
        ) -> MirageDaemonResult<GetSessionDetailReply> {
            Ok(GetSessionDetailReply {
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
            })
        }
    }

    #[async_trait]
    impl MirageDaemonBoot for FixedDaemon {
        async fn boot_session(
            &self,
            _request: BootSessionRequest,
        ) -> MirageDaemonResult<BootSessionReply> {
            Ok(BootSessionReply {
                ok: true,
                error: None,
                container_id: Some("mock-container-id".to_string()),
            })
        }
    }

    #[async_trait]
    impl MirageDaemonExec for FixedDaemon {
        async fn exec_in_session(
            &self,
            _request: ExecInSessionRequest,
        ) -> MirageDaemonResult<ExecInSessionReply> {
            Ok(ExecInSessionReply {
                exit_code: 0,
                stdout: b"ok\n".to_vec(),
                stderr: vec![],
            })
        }
    }

    #[async_trait]
    impl MirageDaemonShutdown for FixedDaemon {
        async fn shutdown_session(
            &self,
            _request: ShutdownSessionRequest,
        ) -> MirageDaemonResult<ShutdownSessionReply> {
            Ok(ShutdownSessionReply {
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
            .list_simulators(ListSimulatorsRequest::default())
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
        let reply = client
            .get_overview(GetOverviewRequest::default())
            .await
            .unwrap();

        assert_eq!(reply.simulator_count, 1);
        assert_eq!(reply.profile_count, 0);
        assert_eq!(reply.session_count, 0);

        server_task.await.unwrap();
        let _ = fs::remove_file(&socket_path).await;
    }
}
