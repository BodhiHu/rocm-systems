use std::path::PathBuf;

use async_trait::async_trait;

use crate::common::{Stream, StreamData, Time};
use crate::daemon::{
    MirageDaemonAttach, MirageDaemonClient, MirageDaemonError, MirageDaemonTime,
};
use crate::socket;

crate::ctl_dsl! {
pub mod mirage_ctl {
    /// Attach to a running process and optionally send data to its `stdin`.
    ///
    /// The server will read from [`stream`](Self::stream) and write to the
    /// exec's `stdin`. It responds with streamed stdout and stderr chunks and
    /// eventually a final exit status.
    attach({
        /// Identifier of the exec to attach to.
        exec_id: String,
    }) ... {
        /// Data to write to the exec's `stdin`.
        stream: Vec<u8>,
    } -> {
        /// Whether this chunk came from `stdout` or `stderr`.
        is_stdout: bool,
        /// Chunks of data read from the exec's `stdout` or `stderr`.
        output: Vec<u8>,
    } => {
        /// The exit code of the exec after it finishes.
        exit_code: i32,
    };

    /// Request the current simulated time for a session.
    time({
        /// Session identifier to query the time for.
        session_id: String = None,
    }) -> {
        /// The current simulated time for the session.
        session_time: Time,
    } ? SessionNotFound;
}
}

pub use mirage_ctl::*;

#[derive(Debug, Clone)]
pub struct MirageCtlDaemonAdapter {
    client: MirageDaemonClient,
}

impl MirageCtlDaemonAdapter {
    pub fn new<P>(socket_path: P) -> Self
    where
        P: Into<PathBuf>,
    {
        Self {
            client: MirageDaemonClient::new(socket_path),
        }
    }

    fn map_time_error(error: MirageDaemonError) -> TimeError {
        match error {
            MirageDaemonError::Remote(message) if message.contains("does not exist") => {
                TimeError::SessionNotFound(SessionNotFound)
            }
            other => TimeError::Other(other.to_string()),
        }
    }
}

#[async_trait]
impl ImplTime for MirageCtlDaemonAdapter {
    async fn time(&self, request: TimeRequest) -> Result<TimeReply, TimeError> {
        let reply = self
            .client
            .time(socket::TimeRequest {
                session_id: request.session_id,
            })
            .await
            .map_err(Self::map_time_error)?;

        Ok(TimeReply {
            session_time: reply.time,
        })
    }
}

#[async_trait]
impl ImplAttach for MirageCtlDaemonAdapter {
    async fn attach(
        &self,
        request: AttachRequest,
        mut input: tokio::sync::mpsc::Receiver<AttachInput>,
        output: tokio::sync::mpsc::Sender<AttachOutput>,
    ) -> Result<AttachReply, AttachError> {
        let mut stdin_bytes = Vec::new();
        while let Some(message) = input.recv().await {
            stdin_bytes.extend(message.stream);
        }

        let replies = self
            .client
            .attach(socket::AttachRequest {
                exec_id: Some(request.exec_id),
                stream: (!stdin_bytes.is_empty()).then(|| StreamData {
                    stream: Stream::Stdin,
                    data: stdin_bytes,
                }),
            })
            .await
            .map_err(|error| AttachError::Other(error.to_string()))?;

        let mut exit_code = 0;
        for reply in replies {
            if let Some(data) = reply.data {
                output
                    .send(AttachOutput {
                        is_stdout: matches!(data.stream, Stream::Stdout),
                        output: data.data,
                    })
                    .await
                    .map_err(|_| {
                        AttachError::Other(
                            "attach output receiver dropped before the stream completed"
                                .to_string(),
                        )
                    })?;
            }

            if let Some(exit) = reply.exit {
                exit_code = exit.exit_code;
            }
        }

        Ok(AttachReply { exit_code })
    }
}

impl MirageCtlCli {
    pub async fn run_daemon(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let adapter = MirageCtlDaemonAdapter::new(self.socket.clone());
        self.run_with(&adapter).await
    }
}
