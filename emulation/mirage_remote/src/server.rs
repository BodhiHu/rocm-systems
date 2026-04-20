//! [`EmulatorServer`] — daemon side of the ioctl proxy.
//!
//! Accepts Unix-domain connections from [`RemoteEmulator`](crate::RemoteEmulator)
//! clients and dispatches each request to a user-supplied [`Emulator`].
//!
//! The server is fully synchronous and spawns a standard OS thread per
//! connection. That matches the client model (one socket per thread) and
//! means the daemon can host emulators that call into C libraries
//! without having to hold a Tokio runtime.

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use mirage_schema::emulator::Emulator;

use crate::protocol::{WireRequest, WireResponse};
use crate::transport::{WireError, read_frame, write_frame};

/// Unix-socket server that exposes an [`Emulator`] to remote clients.
pub struct EmulatorServer {
    emulator: Arc<dyn Emulator>,
    socket_path: PathBuf,
}

impl EmulatorServer {
    pub fn new<P, E>(socket_path: P, emulator: E) -> Self
    where
        P: Into<PathBuf>,
        E: Emulator + 'static,
    {
        Self::from_arc(socket_path, Arc::new(emulator))
    }

    pub fn from_arc<P: Into<PathBuf>>(socket_path: P, emulator: Arc<dyn Emulator>) -> Self {
        Self {
            emulator,
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Bind the socket, removing any stale file from a previous run.
    pub fn bind(&self) -> io::Result<UnixListener> {
        if let Some(parent) = self.socket_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::remove_file(&self.socket_path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        UnixListener::bind(&self.socket_path)
    }

    /// Serve forever, spawning a thread per accepted connection.
    pub fn serve(&self) -> io::Result<()> {
        let listener = self.bind()?;
        self.serve_on(listener)
    }

    /// Serve on an already-bound listener. Useful in tests.
    pub fn serve_on(&self, listener: UnixListener) -> io::Result<()> {
        loop {
            let (stream, _) = listener.accept()?;
            let emulator = Arc::clone(&self.emulator);
            thread::spawn(move || {
                if let Err(e) = handle_client(emulator, stream) {
                    tracing::warn!(error = %e, "mirage_remote connection failed");
                }
            });
        }
    }
}

fn handle_client(emulator: Arc<dyn Emulator>, mut stream: UnixStream) -> Result<(), WireError> {
    loop {
        let request: WireRequest = match read_frame(&mut stream) {
            Ok(r) => r,
            Err(WireError::Io(ref e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe
                ) =>
            {
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let response = dispatch(&*emulator, request);
        write_frame(&mut stream, &response)?;
    }
}

fn dispatch(emulator: &dyn Emulator, request: WireRequest) -> WireResponse {
    match request {
        WireRequest::Ping => WireResponse::Pong,
        WireRequest::Kfd { ctx, request } => {
            WireResponse::Kfd(emulator.handle_any_kfd_ioctl(ctx, request))
        }
        WireRequest::Drm { ctx, request } => {
            WireResponse::Drm(emulator.handle_any_drm_ioctl(ctx, request))
        }
        WireRequest::Device { ctx, request } => {
            WireResponse::Device(emulator.handle_any_device_syscall(ctx, request))
        }
        WireRequest::GetTopology => WireResponse::Topology(emulator.get_topology()),
    }
}
