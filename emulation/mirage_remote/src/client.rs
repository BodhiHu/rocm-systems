//! [`RemoteEmulator`] — client side of the ioctl proxy.
//!
//! Implements [`Emulator`](mirage_schema::emulator::Emulator) by forwarding
//! every ioctl to a remote [`EmulatorServer`](crate::server::EmulatorServer)
//! over a Unix-domain socket.
//!
//! # Thread safety
//!
//! `RemoteEmulator` opens a **separate socket per OS thread**, stored in a
//! `thread_local!`. This simultaneously gives us:
//!
//! * interior mutability without a `Mutex` (each thread writes to its own
//!   socket),
//! * cheap cross-thread sharing (`Send + Sync` with no locking),
//! * a natural way to keep round-trips ordered (no interleaving of
//!   frames from different threads on the same socket).

use std::cell::RefCell;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use mirage_schema::amdgpu::{
    AnyDrmIoctlRequest, AnyDrmIoctlResponse, AnyKfdIoctlRequest, AnyKfdIoctlResponse, IoctlCtx,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};

use crate::protocol::{WireRequest, WireResponse};
use crate::transport::{WireError, read_frame, write_frame};

/// A [`mirage_schema::emulator::Emulator`] that forwards every request to
/// a daemon over a Unix socket.
///
/// `RemoteEmulator` is cheap to clone — cloning does not open a socket;
/// sockets are lazily created per calling thread on first use.
#[derive(Debug, Clone)]
pub struct RemoteEmulator {
    socket_path: PathBuf,
}

impl RemoteEmulator {
    pub fn new<P: Into<PathBuf>>(socket_path: P) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Liveness probe. Useful in tests and for the interceptor to decide
    /// whether to fall back to the host kernel.
    pub fn ping(&self) -> AmdgpuResult<()> {
        match self.round_trip(WireRequest::Ping)? {
            WireResponse::Pong => Ok(()),
            _ => Err(AmdgpuError::Invalid),
        }
    }

    fn round_trip(&self, request: WireRequest) -> AmdgpuResult<WireResponse> {
        THREAD_CONNS.with(|cell| {
            let mut cache = cell.borrow_mut();
            // Try up to twice: if a cached socket has gone stale (peer
            // closed, daemon restarted), drop it and reconnect once.
            for attempt in 0..2 {
                if cache.get_for(&self.socket_path).is_none() {
                    match UnixStream::connect(&self.socket_path) {
                        Ok(stream) => cache.insert(self.socket_path.clone(), stream),
                        Err(e) => return Err(io_to_amdgpu(&e)),
                    }
                }
                let stream = cache.get_for(&self.socket_path).expect("just inserted");
                match do_round_trip(stream, &request) {
                    Ok(response) => return Ok(response),
                    Err(WireError::Io(ref e)) if attempt == 0 && is_retryable_io(e) => {
                        cache.forget(&self.socket_path);
                        continue;
                    }
                    Err(e) => return Err(wire_to_amdgpu(&e)),
                }
            }
            Err(AmdgpuError::Io)
        })
    }
}

fn do_round_trip(
    stream: &mut UnixStream,
    request: &WireRequest,
) -> Result<WireResponse, WireError> {
    write_frame(stream, request)?;
    read_frame(stream)
}

fn is_retryable_io(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::UnexpectedEof
            | io::ErrorKind::NotConnected
    )
}

fn io_to_amdgpu(e: &io::Error) -> AmdgpuError {
    e.raw_os_error()
        .map(AmdgpuError::from_errno)
        .unwrap_or(AmdgpuError::Io)
}

fn wire_to_amdgpu(e: &WireError) -> AmdgpuError {
    match e {
        WireError::Io(e) => io_to_amdgpu(e),
        _ => AmdgpuError::Io,
    }
}

// ---------------------------------------------------------------------------
// Per-thread socket cache

thread_local! {
    static THREAD_CONNS: RefCell<ConnCache> = const { RefCell::new(ConnCache::new()) };
}

/// Small per-thread map of `socket_path -> UnixStream`. In the typical
/// interceptor usage there is only ever a single entry, but we keep it
/// keyed by path so multiple `RemoteEmulator`s pointing at different
/// daemons coexist cleanly.
struct ConnCache {
    entries: Vec<(PathBuf, UnixStream)>,
}

impl ConnCache {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn get_for(&mut self, path: &Path) -> Option<&mut UnixStream> {
        self.entries
            .iter_mut()
            .find(|(p, _)| p == path)
            .map(|(_, s)| s)
    }

    fn insert(&mut self, path: PathBuf, stream: UnixStream) {
        self.forget(&path);
        self.entries.push((path, stream));
    }

    fn forget(&mut self, path: &Path) {
        self.entries.retain(|(p, _)| p != path);
    }
}

// ---------------------------------------------------------------------------
// Emulator forwarding impls — one method each thanks to the DSL macro.

impl mirage_schema::amdgpu::ForwardKfdIoctl for RemoteEmulator {
    fn forward_kfd_ioctl(
        &self,
        ctx: IoctlCtx,
        request: AnyKfdIoctlRequest,
    ) -> AmdgpuResult<AnyKfdIoctlResponse> {
        match self.round_trip(WireRequest::Kfd { ctx, request })? {
            WireResponse::Kfd(result) => result,
            _ => Err(AmdgpuError::Invalid),
        }
    }
}

impl mirage_schema::amdgpu::ForwardDrmIoctl for RemoteEmulator {
    fn forward_drm_ioctl(
        &self,
        ctx: IoctlCtx,
        request: AnyDrmIoctlRequest,
    ) -> AmdgpuResult<AnyDrmIoctlResponse> {
        match self.round_trip(WireRequest::Drm { ctx, request })? {
            WireResponse::Drm(result) => result,
            _ => Err(AmdgpuError::Invalid),
        }
    }
}

impl mirage_schema::syscalls::ForwardFsSyscalls for RemoteEmulator {
    fn forward_fs_syscall(
        &self,
        ctx: IoctlCtx,
        request: mirage_schema::syscalls::AnyFsSyscallRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::AnyFsSyscallResponse> {
        match self.round_trip(WireRequest::Fs { ctx, request })? {
            WireResponse::Fs(result) => result,
            _ => Err(AmdgpuError::Invalid),
        }
    }
}

impl mirage_schema::amdgpu::HandleAnyKfdIoctl for RemoteEmulator {}

impl mirage_schema::amdgpu::HandleAnyDrmIoctl for RemoteEmulator {}

impl mirage_schema::syscalls::HandleAnyFsSyscalls for RemoteEmulator {}
