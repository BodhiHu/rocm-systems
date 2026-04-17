//! Unit tests for `mirage_remote`.
//!
//! These exercise the wire protocol end-to-end with a trivial in-process
//! [`Emulator`] — the `mirage_real` and interceptor crates pull in
//! additional integration tests.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mirage_schema::amdgpu::{
    AmdkfdIocGetVersionRequest, AmdkfdIocGetVersionResponse, AnyKfdIoctlRequest,
    AnyKfdIoctlResponse, IoctlCtx,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};

use crate::protocol::{WireRequest, WireResponse};
use crate::transport::{read_frame, write_frame};
use crate::{EmulatorServer, RemoteEmulator};

/// Minimal emulator that always returns a fixed KFD version and rejects
/// everything else with `ENOSYS`.
struct FixedEmulator;

impl mirage_schema::amdgpu::ForwardKfdIoctl for FixedEmulator {
    fn forward_kfd_ioctl(
        &self,
        _ctx: IoctlCtx,
        request: AnyKfdIoctlRequest,
    ) -> AmdgpuResult<AnyKfdIoctlResponse> {
        match request {
            AnyKfdIoctlRequest::AmdkfdIocGetVersion(_) => Ok(
                AnyKfdIoctlResponse::AmdkfdIocGetVersion(AmdkfdIocGetVersionResponse {
                    major_version: 1,
                    minor_version: 13,
                }),
            ),
            _ => Err(AmdgpuError::NoSys),
        }
    }
}

impl mirage_schema::amdgpu::ForwardDrmIoctl for FixedEmulator {
    fn forward_drm_ioctl(
        &self,
        _ctx: IoctlCtx,
        _request: mirage_schema::amdgpu::AnyDrmIoctlRequest,
    ) -> AmdgpuResult<mirage_schema::amdgpu::AnyDrmIoctlResponse> {
        Err(AmdgpuError::NoSys)
    }
}

fn unique_socket(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("mirage-remote-{tag}-{nanos}.sock"))
}

fn start_fixed_server(tag: &str) -> (std::path::PathBuf, thread::JoinHandle<()>) {
    let path = unique_socket(tag);
    let server = EmulatorServer::from_arc(path.clone(), Arc::new(FixedEmulator));
    let listener = server.bind().expect("bind");
    let handle = thread::spawn(move || {
        let _ = server.serve_on(listener);
    });
    // Give the server a tick to settle (not strictly needed since bind is
    // already done, but cheap).
    thread::sleep(Duration::from_millis(5));
    (path, handle)
}

#[test]
fn wire_request_roundtrips_through_bare() {
    let req = WireRequest::Kfd {
        ctx: IoctlCtx { pid: 42, tid: 7 },
        request: AnyKfdIoctlRequest::AmdkfdIocGetVersion(AmdkfdIocGetVersionRequest {}),
    };
    let bytes = serde_bare::to_vec(&req).unwrap();
    let back: WireRequest = serde_bare::from_slice(&bytes).unwrap();
    assert_eq!(req, back);
}

#[test]
fn ping_pong_round_trip() {
    let (path, _s) = start_fixed_server("ping");
    let remote = RemoteEmulator::new(&path);
    remote.ping().expect("ping ok");
}

#[test]
fn kfd_get_version_round_trip_via_handle_trait() {
    use mirage_schema::amdgpu::HandleKfdIoctl;
    let (path, _s) = start_fixed_server("getver");
    let remote = RemoteEmulator::new(&path);
    let ctx = IoctlCtx { pid: 1, tid: 1 };
    let resp = remote
        .amdkfd_ioc_get_version(ctx, AmdkfdIocGetVersionRequest {})
        .expect("get_version ok");
    assert_eq!(resp.major_version, 1);
    assert_eq!(resp.minor_version, 13);
}

#[test]
fn unimplemented_ioctl_round_trips_as_enosys() {
    use mirage_schema::amdgpu::{AmdkfdIocSetEventRequest, HandleKfdIoctl};
    let (path, _s) = start_fixed_server("enosys");
    let remote = RemoteEmulator::new(&path);
    let ctx = IoctlCtx { pid: 1, tid: 1 };
    let err = remote
        .amdkfd_ioc_set_event(ctx, AmdkfdIocSetEventRequest { event_id: 3 })
        .unwrap_err();
    assert_eq!(err, AmdgpuError::NoSys);
}

#[test]
fn parallel_threads_each_get_their_own_socket() {
    let (path, _s) = start_fixed_server("parallel");
    let remote = Arc::new(RemoteEmulator::new(&path));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let r = Arc::clone(&remote);
        handles.push(thread::spawn(move || {
            for _ in 0..32 {
                r.ping().expect("ping ok under contention");
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}

// Dummy use of items so rustdoc links in the tree don't dangle even if a
// consumer decides not to pull them in.
#[allow(dead_code)]
fn _compile_guards() {
    let _ = read_frame::<WireRequest>;
    let _ = write_frame::<WireRequest>;
    let _: Option<WireResponse> = None;
    let _: Option<AmdkfdIocGetVersionResponse> = None;
    let _: Option<AnyKfdIoctlResponse> = None;
}
