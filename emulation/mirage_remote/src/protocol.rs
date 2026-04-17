//! Wire protocol used to tunnel AMD KFD / DRM ioctl requests between the
//! `mirage_interceptor` (or any other [`RemoteEmulator`] user) and a
//! daemon hosting an [`Emulator`] implementation.
//!
//! Frames are length-prefixed with a big-endian [`u32`] and carry a
//! [`serde_bare`]-encoded [`WireRequest`] / [`WireResponse`]. BARE is a
//! compact, canonical, schema-driven binary format that is cheap to
//! decode in the hot path of every ioctl.
//!
//! [`Emulator`]: mirage_schema::emulator::Emulator

use serde::{Deserialize, Serialize};

use mirage_schema::amdgpu::{
    AnyDrmIoctlRequest, AnyDrmIoctlResponse, AnyKfdIoctlRequest, AnyKfdIoctlResponse, IoctlCtx,
};
use mirage_schema::amdgpu_error::AmdgpuError;
use mirage_schema::syscalls::{AnyDeviceSyscallRequest, AnyDeviceSyscallResponse};
use mirage_schema::topology::Topology;

/// Wire-level result so a remote error round-trips as data rather than
/// causing a transport-level failure.
pub type WireResult<T> = Result<T, AmdgpuError>;

/// Request sent from the client to the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WireRequest {
    /// A KFD ioctl targeting `/dev/kfd`.
    Kfd {
        ctx: IoctlCtx,
        request: AnyKfdIoctlRequest,
    },
    /// A DRM-AMDGPU ioctl targeting `/dev/dri/renderD*` or
    /// `/dev/dri/card*`.
    Drm {
        ctx: IoctlCtx,
        request: AnyDrmIoctlRequest,
    },
    /// A non-ioctl device syscall (open/close/stat/mmap/…).
    Device {
        ctx: IoctlCtx,
        request: AnyDeviceSyscallRequest,
    },
    /// Fetch the entire KFD sysfs topology in one shot.
    GetTopology,
    /// Simple liveness probe — server echoes a [`WireResponse::Pong`].
    Ping,
}

/// Response returned by the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WireResponse {
    Kfd(WireResult<AnyKfdIoctlResponse>),
    Drm(WireResult<AnyDrmIoctlResponse>),
    Device(WireResult<AnyDeviceSyscallResponse>),
    Topology(WireResult<Topology>),
    Pong,
}

/// Maximum accepted frame payload in bytes. A KFD/DRM ioctl request is
/// always comfortably smaller than this.
pub const MAX_FRAME_LEN: usize = 64 * 1024 * 1024;
