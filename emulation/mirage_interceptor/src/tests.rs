//! Unit tests for the interceptor's Rust-level dispatch layer.
//!
//! These exercise everything that does *not* require LD_PRELOAD: fd
//! classification, the registry, ioctl-number decoding, and a full
//! round-trip through a real [`mirage_remote::EmulatorServer`].

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mirage_remote::EmulatorServer;
use mirage_schema::amdgpu::{
    AmdkfdIocGetVersionResponse, AnyKfdIoctlRequest, AnyKfdIoctlResponse, ForwardDrmIoctl,
    ForwardKfdIoctl, IoctlCtx,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};

use super::*;

struct VersionEmulator {
    major: u32,
    minor: u32,
}

impl ForwardKfdIoctl for VersionEmulator {
    fn forward_kfd_ioctl(
        &self,
        _ctx: IoctlCtx,
        request: AnyKfdIoctlRequest,
    ) -> AmdgpuResult<AnyKfdIoctlResponse> {
        match request {
            AnyKfdIoctlRequest::AmdkfdIocGetVersion(_) => Ok(
                AnyKfdIoctlResponse::AmdkfdIocGetVersion(AmdkfdIocGetVersionResponse {
                    major_version: self.major,
                    minor_version: self.minor,
                }),
            ),
            _ => Err(AmdgpuError::NoSys),
        }
    }
}

impl ForwardDrmIoctl for VersionEmulator {
    fn forward_drm_ioctl(
        &self,
        _ctx: IoctlCtx,
        _request: mirage_schema::amdgpu::AnyDrmIoctlRequest,
    ) -> AmdgpuResult<mirage_schema::amdgpu::AnyDrmIoctlResponse> {
        Err(AmdgpuError::NoSys)
    }
}

fn unique_socket() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("mirage-intercept-{nanos}.sock"))
}

#[test]
fn classify_kfd() {
    assert_eq!(
        DeviceKind::classify(Path::new("/dev/kfd")),
        Some(DeviceKind::Kfd)
    );
}

#[test]
fn classify_render_node() {
    assert_eq!(
        DeviceKind::classify(Path::new("/dev/dri/renderD128")),
        Some(DeviceKind::DrmRender)
    );
}

#[test]
fn classify_untracked() {
    assert_eq!(DeviceKind::classify(Path::new("/etc/passwd")), None);
}

#[test]
fn registry_round_trip() {
    register_fd(12345, DeviceKind::Kfd);
    assert_eq!(lookup_fd(12345), Some(DeviceKind::Kfd));
    forget_fd(12345);
    assert_eq!(lookup_fd(12345), None);
}

/// Drive the Rust-level dispatch path (without LD_PRELOAD) to confirm a
/// full C-buffer round-trip works.
#[test]
fn ioctl_get_version_dispatch_via_remote() {
    // Start a server with a fixed version.
    let path = unique_socket();
    let server = EmulatorServer::from_arc(
        path.clone(),
        Arc::new(VersionEmulator {
            major: 7,
            minor: 42,
        }),
    );
    let listener = server.bind().unwrap();
    thread::spawn(move || {
        let _ = server.serve_on(listener);
    });
    thread::sleep(Duration::from_millis(5));

    let remote = mirage_remote::RemoteEmulator::new(&path);

    // Build the raw kfd_ioctl_get_version_args struct ourselves.
    #[repr(C)]
    struct Args {
        major: u32,
        minor: u32,
    }
    let mut args = Args { major: 0, minor: 0 };

    // Compute the actual ioctl cmd: _IOWR('K', 0x01, Args)
    let size = core::mem::size_of::<Args>() as u32;
    let cmd: u32 = (3 /*READ|WRITE*/ << 30) | ((b'K' as u32) << 8) | 0x01 | (size << 16);

    let rc = dispatch_ioctl_with(
        &remote,
        DeviceKind::Kfd,
        cmd,
        &mut args as *mut _ as *mut c_void,
    );
    assert_eq!(rc, 0, "dispatch should succeed (errno={})", unsafe {
        *libc::__errno_location()
    });
    assert_eq!(args.major, 7);
    assert_eq!(args.minor, 42);
}

#[test]
fn unknown_ioctl_returns_enosys() {
    // Use an explicit (disconnected) remote — with no matching nr the
    // dispatch table bails before ever touching the socket.
    let remote = mirage_remote::RemoteEmulator::new("/tmp/mirage-nonexistent-does-not-exist");
    let mut buf = [0u8; 8];
    let cmd: u32 = (1u32 << 30) | ((b'K' as u32) << 8) | 0xFF | (8u32 << 16);
    let rc = dispatch_ioctl_with(
        &remote,
        DeviceKind::Kfd,
        cmd,
        buf.as_mut_ptr() as *mut c_void,
    );
    assert_eq!(rc, -1);
    let errno = unsafe { *libc::__errno_location() };
    assert_eq!(errno, libc::ENOSYS);
}

#[test]
fn wrong_subsystem_returns_enotty() {
    let remote = mirage_remote::RemoteEmulator::new("/tmp/mirage-nonexistent-does-not-exist");
    let mut buf = [0u8; 8];
    // KFD device classified but the cmd is for DRM.
    let cmd: u32 = (1u32 << 30) | ((b'd' as u32) << 8) | 0x01 | (8u32 << 16);
    let rc = dispatch_ioctl_with(
        &remote,
        DeviceKind::Kfd,
        cmd,
        buf.as_mut_ptr() as *mut c_void,
    );
    assert_eq!(rc, -1);
    assert_eq!(unsafe { *libc::__errno_location() }, libc::ENOTTY);
}
