//! `LD_PRELOAD` shim that replaces a process's view of `/dev/kfd` and
//! `/dev/dri/renderD*` with a [`RemoteEmulator`] talking to a Mirage
//! daemon.
//!
//! # Architecture
//!
//! 1. `open`/`openat`/`open64` are exported. Paths matching
//!    [`KFD_PATH`] or a `renderD*` node are redirected: a `memfd` is
//!    allocated to serve as a *cookie* file descriptor so that the
//!    calling program sees an ordinary-looking integer, while the
//!    interceptor tracks the actual device class in a global
//!    registry ([`FD_REGISTRY`]).
//! 2. `close` removes the entry from the registry and forwards to
//!    libc.
//! 3. `ioctl` checks the registry: when the fd is tracked, the raw
//!    ioctl number is decoded via the Linux `_IOC_*` macros. The
//!    appropriate `mirage_schema::amdgpu` enum variant is built from
//!    the C argument buffer, shipped to the remote daemon, and the
//!    response written back. Untracked fds pass through to libc.
//!
//! # Scope
//!
//! The Rust ↔ C marshalling is large (one pair per ioctl). This file
//! ships GET_VERSION as a worked example and a macro-driven skeleton
//! for the rest — adding a new ioctl is one macro invocation.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_ulong, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once, OnceLock};

use libc::{O_CLOEXEC, size_t};

use mirage_remote::RemoteEmulator;
use mirage_schema::amdgpu::{self, HandleDrmIoctl, HandleKfdIoctl, IoctlCtx};
use mirage_schema::amdgpu_error::AmdgpuError;
use mirage_schema::syscalls::{self, DeviceClass, FakeStat, HandleDeviceSyscalls};
use mirage_uapi::{drm, ioc as uapi_ioc, kfd};

/// Path to the KFD char device.
pub const KFD_PATH: &str = "/dev/kfd";

/// Prefix of DRM render node files.
pub const DRI_RENDER_PREFIX: &str = "/dev/dri/renderD";

/// Environment variable naming the daemon socket path. If unset, the
/// interceptor behaves transparently (every call falls through to libc).
pub const MIRAGE_SOCKET_ENV: &str = "MIRAGE_INTERCEPTOR_SOCKET";

// ---------------------------------------------------------------------------
// Global state

static REMOTE: OnceLock<Option<RemoteEmulator>> = OnceLock::new();

fn remote() -> Option<&'static RemoteEmulator> {
    REMOTE
        .get_or_init(|| {
            init_tracing();
            std::env::var_os(MIRAGE_SOCKET_ENV).map(|s| RemoteEmulator::new(PathBuf::from(s)))
        })
        .as_ref()
}

/// Lazily cached KFD sysfs topology, fetched once from the daemon.
static TOPOLOGY: OnceLock<Option<mirage_schema::topology::Topology>> = OnceLock::new();

/// Sysfs topology prefix the interceptor recognises.
const SYSFS_TOPO_PREFIX: &str = "/sys/class/kfd/kfd/topology/";

/// Alternative sysfs path that hsakmt reads from.
const SYSFS_TOPO_PREFIX_ALT: &str = "/sys/devices/virtual/kfd/kfd/topology/";

fn cached_topology() -> Option<&'static mirage_schema::topology::Topology> {
    TOPOLOGY
        .get_or_init(|| {
            use mirage_schema::topology::ProvideTopology;
            remote().and_then(|r| r.get_topology().ok())
        })
        .as_ref()
}

/// Look up a sysfs topology file by its absolute path, returning the
/// file contents from the cached topology if present.
fn topology_lookup(path: &str) -> Option<&'static [u8]> {
    let rel = path
        .strip_prefix(SYSFS_TOPO_PREFIX)
        .or_else(|| path.strip_prefix(SYSFS_TOPO_PREFIX_ALT))?;
    let topo = cached_topology()?;
    topo.files.get(rel).map(|v| v.as_slice())
}

/// Returns `true` if the path is under the KFD sysfs topology tree.
fn is_topology_path(path: &str) -> bool {
    path.starts_with(SYSFS_TOPO_PREFIX) || path.starts_with(SYSFS_TOPO_PREFIX_ALT)
}

/// Create a memfd pre-filled with `data` so that subsequent reads
/// see the cached topology file contents.
fn memfd_from_topology_data(data: &[u8]) -> c_int {
    let name = CString::new("mirage-topo").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC as u32) };
    if fd < 0 {
        return -1;
    }
    if !data.is_empty() {
        let written =
            unsafe { libc::write(fd, data.as_ptr() as *const c_void, data.len()) };
        if written < 0 {
            unsafe { libc::close(fd) };
            return -1;
        }
        // Rewind to the beginning so reads start from offset 0.
        unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
    }
    fd
}

/// Device class a tracked fd refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Kfd,
    DrmRender,
}

impl DeviceKind {
    pub fn classify(path: &Path) -> Option<Self> {
        let s = path.to_str()?;
        if s == KFD_PATH {
            return Some(Self::Kfd);
        }
        if s.starts_with(DRI_RENDER_PREFIX) {
            return Some(Self::DrmRender);
        }
        None
    }

    fn device_class(self) -> DeviceClass {
        match self {
            Self::Kfd => DeviceClass::Kfd,
            Self::DrmRender => DeviceClass::DrmRender,
        }
    }

    fn default_path(self) -> &'static str {
        match self {
            Self::Kfd => KFD_PATH,
            Self::DrmRender => "/dev/dri/renderD128",
        }
    }
}

#[derive(Debug, Clone)]
struct TrackedFd {
    cookie_fd: c_int,
    remote_fd: c_int,
    host_fd: c_int,
    /// `true` when `host_fd` is a memfd placeholder (no real device present).
    /// Passthrough ioctls and device-backed mmap must not use a synthetic fd.
    synthetic_host: bool,
    kind: DeviceKind,
    path: PathBuf,
    fake_stat: FakeStat,
}

static FD_REGISTRY: OnceLock<Mutex<Vec<TrackedFd>>> = OnceLock::new();

/// Counter for synthetic handles (USERPTR allocations handled locally).
/// Starts at a high value to avoid collisions with daemon-assigned handles.
static SYNTHETIC_HANDLE_COUNTER: AtomicU64 = AtomicU64::new(0xFFFF_0000_0000_0001);

fn registry() -> &'static Mutex<Vec<TrackedFd>> {
    FD_REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Public helper used by tests and out-of-process callers.
pub fn register_fd(fd: c_int, kind: DeviceKind) {
    register_entry(TrackedFd {
        cookie_fd: fd,
        remote_fd: -1,
        host_fd: -1,
        synthetic_host: true,
        kind,
        path: PathBuf::from(kind.default_path()),
        fake_stat: FakeStat::default(),
    });
}

/// Public helper: look up the device class for a tracked fd.
pub fn lookup_fd(fd: c_int) -> Option<DeviceKind> {
    lookup_entry(fd).map(|entry| entry.kind)
}

/// Public helper: drop a tracked fd.
pub fn forget_fd(fd: c_int) {
    registry()
        .lock()
        .unwrap()
        .retain(|entry| entry.cookie_fd != fd);
}

fn register_entry(entry: TrackedFd) {
    let mut g = registry().lock().unwrap();
    g.retain(|tracked| tracked.cookie_fd != entry.cookie_fd);
    g.push(entry);
}

fn lookup_entry(fd: c_int) -> Option<TrackedFd> {
    registry()
        .lock()
        .unwrap()
        .iter()
        .find(|entry| entry.cookie_fd == fd)
        .cloned()
}

fn translate_remote_fd(fd: c_int) -> Option<c_int> {
    lookup_entry(fd).and_then(|entry| (entry.remote_fd >= 0).then_some(entry.remote_fd))
}

fn has_other_aliases(remote_fd: c_int, except_cookie_fd: c_int) -> bool {
    registry().lock().unwrap().iter().any(|entry| {
        entry.cookie_fd != except_cookie_fd && entry.remote_fd >= 0 && entry.remote_fd == remote_fd
    })
}

fn has_other_host_aliases(host_fd: c_int, except_cookie_fd: c_int) -> bool {
    registry().lock().unwrap().iter().any(|entry| {
        entry.cookie_fd != except_cookie_fd && entry.host_fd >= 0 && entry.host_fd == host_fd
    })
}

// ---------------------------------------------------------------------------
// ioctl number decoding (matches <asm-generic/ioctl.h>)

mod ioc {
    pub const NRBITS: u32 = 8;
    pub const TYPEBITS: u32 = 8;
    pub const SIZEBITS: u32 = 14;
    pub const NRSHIFT: u32 = 0;
    pub const TYPESHIFT: u32 = NRSHIFT + NRBITS;
    pub const SIZESHIFT: u32 = TYPESHIFT + TYPEBITS;

    pub const NRMASK: u32 = (1 << NRBITS) - 1;
    pub const TYPEMASK: u32 = (1 << TYPEBITS) - 1;
    pub const SIZEMASK: u32 = (1 << SIZEBITS) - 1;

    pub fn nr(cmd: u32) -> u32 {
        (cmd >> NRSHIFT) & NRMASK
    }
    pub fn ty(cmd: u32) -> u32 {
        (cmd >> TYPESHIFT) & TYPEMASK
    }
    pub fn size(cmd: u32) -> u32 {
        (cmd >> SIZESHIFT) & SIZEMASK
    }
}

const KFD_MAGIC: u32 = b'K' as u32;
const DRM_MAGIC: u32 = b'd' as u32;

// ---------------------------------------------------------------------------
#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct DrmVersion {
    version_major: i32,
    version_minor: i32,
    version_patchlevel: i32,
    name_len: size_t,
    name: *mut c_char,
    date_len: size_t,
    date: *mut c_char,
    desc_len: size_t,
    desc: *mut c_char,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct DrmAuth {
    magic: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct DrmClient {
    idx: i32,
    auth: i32,
    pid: c_ulong,
    uid: c_ulong,
    magic: c_ulong,
    iocs: c_ulong,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct DrmGetCap {
    capability: u64,
    value: u64,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct DrmSetClientCap {
    capability: u64,
    value: u64,
}

const DRM_IOCTL_VERSION_NR: u32 = 0x00;
const DRM_IOCTL_GET_MAGIC_NR: u32 = 0x02;
const DRM_IOCTL_GET_CLIENT_NR: u32 = 0x05;
const DRM_IOCTL_GET_CAP_NR: u32 = 0x0c;
const DRM_IOCTL_SET_CLIENT_CAP_NR: u32 = 0x0d;
const DRM_IOCTL_AUTH_MAGIC_NR: u32 = 0x11;

fn check_size<T>(size: usize) -> Result<(), c_int> {
    if size < core::mem::size_of::<T>() {
        Err(errno_to_rc(libc::EINVAL))
    } else {
        Ok(())
    }
}

unsafe fn copy_u8_slice(dst: *mut u8, capacity: usize, data: &[u8]) {
    if capacity == 0 || dst.is_null() {
        return;
    }
    let len = capacity.min(data.len());
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), dst, len) };
}

unsafe fn copy_c_string(dst: *mut c_char, capacity: usize, value: &[u8]) {
    if capacity == 0 || dst.is_null() {
        return;
    }
    let text_len = value.len().min(capacity.saturating_sub(1));
    unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), dst.cast::<u8>(), text_len) };
    unsafe { *dst.add(text_len) = 0 };
}

unsafe fn copy_apertures_to_user(
    ptr: u64,
    apertures: &[amdgpu::KfdProcessDeviceAperture],
) -> Result<(), c_int> {
    if apertures.is_empty() {
        return Ok(());
    }
    if ptr == 0 {
        return Err(errno_to_rc(libc::EFAULT));
    }
    let dst = unsafe {
        std::slice::from_raw_parts_mut(
            ptr as usize as *mut kfd::kfd_process_device_apertures,
            apertures.len(),
        )
    };
    for (raw, aperture) in dst.iter_mut().zip(apertures.iter()) {
        *raw = kfd::kfd_process_device_apertures {
            lds_base: aperture.lds_base,
            lds_limit: aperture.lds_limit,
            scratch_base: aperture.scratch_base,
            scratch_limit: aperture.scratch_limit,
            gpuvm_base: aperture.gpuvm_base,
            gpuvm_limit: aperture.gpuvm_limit,
            gpu_id: aperture.gpu_id,
            pad: 0,
        };
    }
    Ok(())
}

unsafe fn read_u32s_from_user(ptr: u64, count: usize) -> Result<Vec<u32>, c_int> {
    if count == 0 {
        return Ok(Vec::new());
    }
    if ptr == 0 {
        return Err(errno_to_rc(libc::EFAULT));
    }
    Ok(unsafe { std::slice::from_raw_parts(ptr as usize as *const u32, count).to_vec() })
}

fn cache_policy_from_raw(value: u32) -> Option<amdgpu::KfdCachePolicy> {
    match value {
        0 => Some(amdgpu::KfdCachePolicy::Coherent),
        1 => Some(amdgpu::KfdCachePolicy::Noncoherent),
        _ => None,
    }
}

fn event_type_from_raw(value: u32) -> Option<amdgpu::KfdEventType> {
    match value {
        0 => Some(amdgpu::KfdEventType::Signal),
        1 => Some(amdgpu::KfdEventType::NodeChange),
        2 => Some(amdgpu::KfdEventType::DeviceStateChange),
        3 => Some(amdgpu::KfdEventType::HwException),
        4 => Some(amdgpu::KfdEventType::SystemEvent),
        5 => Some(amdgpu::KfdEventType::DebugEvent),
        6 => Some(amdgpu::KfdEventType::ProfileEvent),
        7 => Some(amdgpu::KfdEventType::QueueEvent),
        8 => Some(amdgpu::KfdEventType::Memory),
        _ => None,
    }
}

// Raw C structs matching the kernel's kfd_event_data layout for WAIT_EVENTS.
#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdMemoryExceptionFailureRaw {
    not_present: u32,
    read_only: u32,
    no_execute: u32,
    imprecise: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdMemoryExceptionDataRaw {
    failure: KfdMemoryExceptionFailureRaw,
    va: u64,
    gpu_id: u32,
    error_type: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdHwExceptionDataRaw {
    reset_type: u32,
    reset_cause: u32,
    memory_lost: u32,
    gpu_id: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
union KfdEventUnionRaw {
    memory_exception_data: KfdMemoryExceptionDataRaw,
    hw_exception_data: KfdHwExceptionDataRaw,
}

impl Default for KfdEventUnionRaw {
    fn default() -> Self {
        Self {
            memory_exception_data: KfdMemoryExceptionDataRaw::default(),
        }
    }
}

#[repr(C)]
#[derive(Default, Copy, Clone)]
struct KfdEventDataRaw {
    payload: KfdEventUnionRaw,
    kfd_event_data_ext: u64,
    event_id: u32,
    pad: u32,
}

fn event_data_to_schema(raw: &KfdEventDataRaw) -> amdgpu::KfdEventData {
    amdgpu::KfdEventData {
        event_id: raw.event_id,
        memory_exception_data: None,
        hw_exception_data: None,
        signal_event_data: None,
        kfd_event_data_ext: raw.kfd_event_data_ext,
    }
}

fn event_data_from_schema(event: &amdgpu::KfdEventData, raw: &mut KfdEventDataRaw) {
    raw.event_id = event.event_id;
    raw.kfd_event_data_ext = event.kfd_event_data_ext;
    if let Some(mem) = &event.memory_exception_data {
        raw.payload = KfdEventUnionRaw {
            memory_exception_data: KfdMemoryExceptionDataRaw {
                failure: KfdMemoryExceptionFailureRaw {
                    not_present: mem.failure.not_present,
                    read_only: mem.failure.read_only,
                    no_execute: mem.failure.no_execute,
                    imprecise: mem.failure.imprecise,
                },
                va: mem.va,
                gpu_id: mem.gpu_id,
                error_type: mem.error_type,
            },
        };
    } else if let Some(hw) = &event.hw_exception_data {
        raw.payload = KfdEventUnionRaw {
            hw_exception_data: KfdHwExceptionDataRaw {
                reset_type: hw.reset_type,
                reset_cause: hw.reset_cause,
                memory_lost: hw.memory_lost,
                gpu_id: hw.gpu_id,
            },
        };
    }
}

fn dispatch_kfd(remote: &RemoteEmulator, cmd: u32, arg: *mut c_void, _host_fd: c_int) -> c_int {
    let ctx = current_ctx();
    let nr = ioc::nr(cmd);
    let size = ioc::size(cmd) as usize;

    // Every KFD ioctl is forwarded to the daemon. The interceptor never
    // talks to /dev/kfd directly — all kernel interactions are the
    // daemon's responsibility.

    match nr {
        x if x == (amdgpu::AMDKFD_IOC_GET_VERSION & 0xff) => {
            if check_size::<kfd::kfd_ioctl_get_version_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_get_version_args) };
            match remote.amdkfd_ioc_get_version(ctx, amdgpu::AmdkfdIocGetVersionRequest {}) {
                Ok(resp) => {
                    args.major_version = resp.major_version;
                    args.minor_version = resp.minor_version;
                    0
                }
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_SET_MEMORY_POLICY & 0xff) => {
            if check_size::<kfd::kfd_ioctl_set_memory_policy_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_set_memory_policy_args) };
            let Some(default_policy) = cache_policy_from_raw(args.default_policy) else {
                return errno_to_rc(libc::EINVAL);
            };
            let Some(alternate_policy) = cache_policy_from_raw(args.alternate_policy) else {
                return errno_to_rc(libc::EINVAL);
            };
            match remote.amdkfd_ioc_set_memory_policy(
                ctx,
                amdgpu::AmdkfdIocSetMemoryPolicyRequest {
                    alternate_aperture_base: args.alternate_aperture_base,
                    alternate_aperture_size: args.alternate_aperture_size,
                    gpu_id: args.gpu_id,
                    default_policy,
                    alternate_policy,
                    misc_process_flag: args.misc_process_flag,
                },
            ) {
                Ok(_) => 0,
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_GET_CLOCK_COUNTERS & 0xff) => {
            if check_size::<kfd::kfd_ioctl_get_clock_counters_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_get_clock_counters_args) };
            match remote.amdkfd_ioc_get_clock_counters(
                ctx,
                amdgpu::AmdkfdIocGetClockCountersRequest {
                    gpu_id: args.gpu_id,
                },
            ) {
                Ok(resp) => {
                    args.gpu_clock_counter = resp.gpu_clock_counter;
                    args.cpu_clock_counter = resp.cpu_clock_counter;
                    args.system_clock_counter = resp.system_clock_counter;
                    args.system_clock_freq = resp.system_clock_freq;
                    0
                }
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_CREATE_EVENT & 0xff) => {
            if check_size::<kfd::kfd_ioctl_create_event_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_create_event_args) };
            let Some(event_type) = event_type_from_raw(args.event_type) else {
                return errno_to_rc(libc::EINVAL);
            };
            match remote.amdkfd_ioc_create_event(
                ctx,
                amdgpu::AmdkfdIocCreateEventRequest {
                    event_type,
                    auto_reset: args.auto_reset,
                    node_id: args.node_id,
                },
            ) {
                Ok(resp) => {
                    args.event_page_offset = resp.event_page_offset;
                    args.event_trigger_data = resp.event_trigger_data;
                    args.event_id = resp.event_id;
                    args.event_slot_index = resp.event_slot_index;
                    0
                }
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_DESTROY_EVENT & 0xff) => {
            if check_size::<kfd::kfd_ioctl_destroy_event_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_destroy_event_args) };
            match remote.amdkfd_ioc_destroy_event(
                ctx,
                amdgpu::AmdkfdIocDestroyEventRequest {
                    event_id: args.event_id,
                },
            ) {
                Ok(_) => 0,
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_SET_EVENT & 0xff) => {
            if check_size::<kfd::kfd_ioctl_set_event_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_set_event_args) };
            match remote.amdkfd_ioc_set_event(
                ctx,
                amdgpu::AmdkfdIocSetEventRequest {
                    event_id: args.event_id,
                },
            ) {
                Ok(_) => 0,
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_GET_PROCESS_APERTURES_NEW & 0xff) => {
            if check_size::<kfd::kfd_ioctl_get_process_apertures_new_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_get_process_apertures_new_args) };
            match remote.amdkfd_ioc_get_process_apertures_new(
                ctx,
                amdgpu::AmdkfdIocGetProcessAperturesNewRequest {
                    max_nodes: args.num_of_nodes,
                },
            ) {
                Ok(resp) => {
                    if unsafe {
                        copy_apertures_to_user(
                            args.kfd_process_device_apertures_ptr,
                            &resp.apertures,
                        )
                    }
                    .is_err()
                    {
                        return errno_to_rc(libc::EFAULT);
                    }
                    args.num_of_nodes = resp.apertures.len() as u32;
                    0
                }
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_ACQUIRE_VM & 0xff) => {
            if check_size::<kfd::kfd_ioctl_acquire_vm_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_acquire_vm_args) };
            let drm_fd = translate_remote_fd(args.drm_fd as c_int).unwrap_or(args.drm_fd as c_int);
            match remote.amdkfd_ioc_acquire_vm(
                ctx,
                amdgpu::AmdkfdIocAcquireVmRequest {
                    drm_fd: drm_fd as u32,
                    gpu_id: args.gpu_id,
                },
            ) {
                Ok(_) => 0,
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_ALLOC_MEMORY_OF_GPU & 0xff) => {
            if check_size::<kfd::kfd_ioctl_alloc_memory_of_gpu_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_alloc_memory_of_gpu_args) };

            // USERPTR allocations register container-side user memory.
            // The daemon can't access those addresses, so handle locally.
            const KFD_IOC_ALLOC_MEM_FLAGS_USERPTR: u32 = 0x4;
            if (args.flags & KFD_IOC_ALLOC_MEM_FLAGS_USERPTR) != 0 {
                let handle = SYNTHETIC_HANDLE_COUNTER.fetch_add(1, Ordering::Relaxed);
                args.handle = handle;
                tracing::debug!(
                    va_addr = format_args!("0x{:x}", args.va_addr),
                    size = args.size,
                    handle = format_args!("0x{:x}", handle),
                    "ALLOC_MEMORY_OF_GPU USERPTR handled locally",
                );
                return 0;
            }

            match remote.amdkfd_ioc_alloc_memory_of_gpu(
                ctx,
                amdgpu::AmdkfdIocAllocMemoryOfGpuRequest {
                    va_addr: args.va_addr,
                    size: args.size,
                    gpu_id: args.gpu_id,
                    flags: args.flags,
                    mmap_offset: args.mmap_offset,
                },
            ) {
                Ok(resp) => {
                    args.handle = resp.handle;
                    args.mmap_offset = resp.mmap_offset;
                    args.va_addr = resp.va_addr;
                    0
                }
                Err(err) => {
                    tracing::debug!(
                        va_addr = format_args!("0x{:x}", args.va_addr),
                        size = args.size,
                        gpu_id = args.gpu_id,
                        flags = format_args!("0x{:x}", args.flags),
                        error = ?err,
                        "ALLOC_MEMORY_OF_GPU failed",
                    );
                    errno_to_rc(err.errno())
                }
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_FREE_MEMORY_OF_GPU & 0xff) => {
            if check_size::<kfd::kfd_ioctl_free_memory_of_gpu_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_free_memory_of_gpu_args) };
            // Synthetic handles (USERPTR) are not tracked by daemon.
            if args.handle >= 0xFFFF_0000_0000_0000 {
                return 0;
            }
            match remote.amdkfd_ioc_free_memory_of_gpu(
                ctx,
                amdgpu::AmdkfdIocFreeMemoryOfGpuRequest {
                    handle: args.handle,
                },
            ) {
                Ok(_) => 0,
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_MAP_MEMORY_TO_GPU & 0xff) => {
            if check_size::<kfd::kfd_ioctl_map_memory_to_gpu_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_map_memory_to_gpu_args) };
            let Ok(device_ids) = (unsafe {
                read_u32s_from_user(args.device_ids_array_ptr, args.n_devices as usize)
            }) else {
                return errno_to_rc(libc::EFAULT);
            };
            // Synthetic handles (USERPTR) are not tracked by daemon.
            if args.handle >= 0xFFFF_0000_0000_0000 {
                args.n_success = args.n_devices;
                return 0;
            }
            match remote.amdkfd_ioc_map_memory_to_gpu(
                ctx,
                amdgpu::AmdkfdIocMapMemoryToGpuRequest {
                    handle: args.handle,
                    device_ids,
                },
            ) {
                Ok(resp) => {
                    args.n_success = resp.n_success;
                    0
                }
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_UNMAP_MEMORY_FROM_GPU & 0xff) => {
            if check_size::<kfd::kfd_ioctl_unmap_memory_from_gpu_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_unmap_memory_from_gpu_args) };
            let Ok(device_ids) = (unsafe {
                read_u32s_from_user(args.device_ids_array_ptr, args.n_devices as usize)
            }) else {
                return errno_to_rc(libc::EFAULT);
            };
            // Synthetic handles (USERPTR) are not tracked by daemon.
            if args.handle >= 0xFFFF_0000_0000_0000 {
                args.n_success = args.n_devices;
                return 0;
            }
            match remote.amdkfd_ioc_unmap_memory_from_gpu(
                ctx,
                amdgpu::AmdkfdIocUnmapMemoryFromGpuRequest {
                    handle: args.handle,
                    device_ids,
                },
            ) {
                Ok(resp) => {
                    args.n_success = resp.n_success;
                    0
                }
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_SET_XNACK_MODE & 0xff) => {
            if check_size::<kfd::kfd_ioctl_set_xnack_mode_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_set_xnack_mode_args) };
            match remote.amdkfd_ioc_set_xnack_mode(
                ctx,
                amdgpu::AmdkfdIocSetXnackModeRequest {
                    xnack_enabled: args.xnack_enabled,
                },
            ) {
                Ok(resp) => {
                    args.xnack_enabled = resp.xnack_enabled;
                    0
                }
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_RUNTIME_ENABLE & 0xff) => {
            if check_size::<kfd::kfd_ioctl_runtime_enable_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_runtime_enable_args) };
            match remote.amdkfd_ioc_runtime_enable(
                ctx,
                amdgpu::AmdkfdIocRuntimeEnableRequest {
                    flags: (args.mode_mask as u64) | ((args.capabilities_mask as u64) << 32),
                },
            ) {
                Ok(_) => 0,
                Err(err) => {
                    tracing::debug!(
                        name = err.name(),
                        errno = err.errno(),
                        r_debug = args.r_debug,
                        mode_mask = format_args!("0x{:x}", args.mode_mask),
                        caps = format_args!("0x{:x}", args.capabilities_mask),
                        "RUNTIME_ENABLE failed",
                    );
                    errno_to_rc(err.errno())
                }
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_RESET_EVENT & 0xff) => {
            if check_size::<kfd::kfd_ioctl_reset_event_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_reset_event_args) };
            match remote.amdkfd_ioc_reset_event(
                ctx,
                amdgpu::AmdkfdIocResetEventRequest {
                    event_id: args.event_id,
                },
            ) {
                Ok(_) => 0,
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_SET_SCRATCH_BACKING_VA & 0xff) => {
            if check_size::<kfd::kfd_ioctl_set_scratch_backing_va_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_set_scratch_backing_va_args) };
            match remote.amdkfd_ioc_set_scratch_backing_va(
                ctx,
                amdgpu::AmdkfdIocSetScratchBackingVaRequest {
                    va_addr: args.va_addr,
                    gpu_id: args.gpu_id,
                },
            ) {
                Ok(_) => 0,
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_WAIT_EVENTS & 0xff) => {
            if check_size::<kfd::kfd_ioctl_wait_events_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_wait_events_args) };
            let num = args.num_events as usize;
            let events_ptr = args.events_ptr as usize as *mut KfdEventDataRaw;
            if events_ptr.is_null() || num == 0 {
                return errno_to_rc(libc::EINVAL);
            }
            let raw_events = unsafe { std::slice::from_raw_parts(events_ptr, num) };
            let schema_events: Vec<amdgpu::KfdEventData> =
                raw_events.iter().map(event_data_to_schema).collect();
            match remote.amdkfd_ioc_wait_events(
                ctx,
                amdgpu::AmdkfdIocWaitEventsRequest {
                    events: schema_events,
                    wait_for_all: args.wait_for_all != 0,
                    timeout: args.timeout,
                },
            ) {
                Ok(resp) => {
                    args.wait_result = resp.wait_result;
                    let out_events = unsafe { std::slice::from_raw_parts_mut(events_ptr, num) };
                    for (raw, schema) in out_events.iter_mut().zip(resp.events.iter()) {
                        event_data_from_schema(schema, raw);
                    }
                    0
                }
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        x if x == (amdgpu::AMDKFD_IOC_SET_TRAP_HANDLER & 0xff) => {
            if check_size::<kfd::kfd_ioctl_set_trap_handler_args>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut kfd::kfd_ioctl_set_trap_handler_args) };
            match remote.amdkfd_ioc_set_trap_handler(
                ctx,
                amdgpu::AmdkfdIocSetTrapHandlerRequest {
                    tba_addr: args.tba_addr,
                    tma_addr: args.tma_addr,
                    gpu_id: args.gpu_id,
                },
            ) {
                Ok(_) => 0,
                Err(err) => errno_to_rc(err.errno()),
            }
        }
        _ => {
            tracing::warn!(
                nr = format_args!("0x{nr:02x}"),
                size,
                "unhandled kfd ioctl (not forwarded to daemon)",
            );
            errno_to_rc(libc::ENOSYS)
        }
    }
}

fn dispatch_drm(remote: &RemoteEmulator, cmd: u32, arg: *mut c_void, host_fd: c_int) -> c_int {
    let ctx = current_ctx();
    let nr = ioc::nr(cmd);
    let size = ioc::size(cmd) as usize;

    match nr {
        DRM_IOCTL_VERSION_NR => {
            if check_size::<DrmVersion>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut DrmVersion) };
            let name = b"amdgpu";
            let date = b"mirage";
            let desc = b"mirage-amdgpu";
            args.version_major = 3;
            args.version_minor = 0;
            args.version_patchlevel = 0;
            let name_len = name.len();
            let date_len = date.len();
            let desc_len = desc.len();
            unsafe {
                copy_c_string(args.name, args.name_len, name);
                copy_c_string(args.date, args.date_len, date);
                copy_c_string(args.desc, args.desc_len, desc);
            }
            args.name_len = name_len;
            args.date_len = date_len;
            args.desc_len = desc_len;
            0
        }
        DRM_IOCTL_GET_MAGIC_NR => {
            if check_size::<DrmAuth>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut DrmAuth) };
            args.magic = 1;
            0
        }
        DRM_IOCTL_GET_CLIENT_NR => {
            if check_size::<DrmClient>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut DrmClient) };
            if args.idx != 0 {
                return errno_to_rc(libc::ENOENT);
            }
            args.auth = 1;
            args.pid = unsafe { libc::getpid() as c_ulong };
            args.uid = unsafe { libc::geteuid() as c_ulong };
            args.magic = 1;
            args.iocs = 0;
            0
        }
        DRM_IOCTL_GET_CAP_NR => {
            if check_size::<DrmGetCap>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut DrmGetCap) };
            args.value = 0;
            0
        }
        DRM_IOCTL_SET_CLIENT_CAP_NR => {
            if check_size::<DrmSetClientCap>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            0
        }
        DRM_IOCTL_AUTH_MAGIC_NR => {
            if check_size::<DrmAuth>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            0
        }
        x if x == (uapi_ioc::DRM_COMMAND_BASE + drm::DRM_AMDGPU_INFO) => {
            if check_size::<drm::drm_amdgpu_info>(size).is_err() {
                return errno_to_rc(libc::EINVAL);
            }
            let args = unsafe { &mut *(arg as *mut drm::drm_amdgpu_info) };
            let (sub_query, sub_query2, sub_query3, flags) = unsafe {
                let words = (&mut args.__bindgen_anon_1 as *mut drm::drm_amdgpu_info__bindgen_ty_1)
                    .cast::<u32>();
                (*words.add(0), *words.add(1), *words.add(2), *words.add(3))
            };
            match remote.drm_amdgpu_info(
                ctx,
                amdgpu::DrmAmdgpuInfoRequest {
                    query: args.query,
                    return_size: args.return_size,
                    sub_query,
                    sub_query2,
                    sub_query3,
                    flags,
                },
            ) {
                Ok(resp) => {
                    unsafe {
                        copy_u8_slice(
                            args.return_pointer as usize as *mut u8,
                            args.return_size as usize,
                            &resp.raw_data,
                        );
                    }
                    0
                }
                Err(err) => errno_to_rc(err.errno())
            }
        }
        _ => {
            tracing::debug!(
                nr = format_args!("0x{nr:02x}"),
                size,
                host_fd,
                "passthrough drm ioctl",
            );
            passthrough_ioctl(host_fd, cmd, arg)
        }
    }
}

fn current_ctx() -> IoctlCtx {
    // SAFETY: getpid/gettid are always safe.
    let pid = unsafe { libc::getpid() } as u32;
    let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
    IoctlCtx { pid, tid }
}

fn errno_to_rc(errno: i32) -> c_int {
    // SAFETY: setting the thread-local errno is always sound.
    unsafe {
        *libc::__errno_location() = errno;
    }
    -1
}

/// Fall through to the real kernel ioctl on the host fd for unrecognized
/// commands. This lets HIP and libdrm use ioctls that the interceptor
/// doesn't explicitly handle.
fn passthrough_ioctl(host_fd: c_int, cmd: u32, arg: *mut c_void) -> c_int {
    if host_fd < 0 {
        return errno_to_rc(libc::ENOSYS);
    }
    static REAL_IOCTL: OnceLock<usize> = OnceLock::new();
    let p = *REAL_IOCTL.get_or_init(|| {
        let s = c"ioctl";
        unsafe { libc::dlsym(libc::RTLD_NEXT, s.as_ptr()) as usize }
    });
    if p == 0 {
        return errno_to_rc(libc::ENOSYS);
    }
    let real: fn(c_int, libc::c_ulong, *mut c_void) -> c_int = unsafe { std::mem::transmute(p) };
    real(host_fd, cmd as libc::c_ulong, arg)
}

fn init_tracing() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_target(false)
            .init();
    });
}

// ---------------------------------------------------------------------------
// Public dispatch entry points — callable from tests and from the
// LD_PRELOAD hooks below.

/// Dispatch a tracked ioctl using an explicit [`RemoteEmulator`].
/// Primarily used by tests; the production path is
/// [`dispatch_tracked_ioctl`] which pulls the global from the env.
pub fn dispatch_ioctl_with(
    remote: &RemoteEmulator,
    kind: DeviceKind,
    cmd: u32,
    arg: *mut c_void,
    host_fd: c_int,
) -> c_int {
    let ty = ioc::ty(cmd);
    match kind {
        DeviceKind::Kfd if ty == KFD_MAGIC => dispatch_kfd(remote, cmd, arg, host_fd),
        DeviceKind::DrmRender if ty == DRM_MAGIC => dispatch_drm(remote, cmd, arg, host_fd),
        _ => errno_to_rc(libc::ENOTTY),
    }
}

/// Dispatch a tracked ioctl using the global [`RemoteEmulator`]
/// configured through `MIRAGE_INTERCEPTOR_SOCKET`. Returns `ENOSYS` when
/// the env var is unset.
pub fn dispatch_tracked_ioctl(kind: DeviceKind, fd: c_int, cmd: u32, arg: *mut c_void) -> c_int {
    let Some(remote) = remote() else {
        return errno_to_rc(AmdgpuError::NoSys.errno());
    };
    // Only pass a real host_fd for passthrough; synthetic memfds must not
    // be used for real kernel ioctls.
    let host_fd = lookup_entry(fd)
        .filter(|e| !e.synthetic_host)
        .map_or(-1, |e| e.host_fd);
    tracing::debug!(
        ?kind,
        ty = format_args!("0x{:02x}", ioc::ty(cmd)),
        nr = format_args!("0x{:02x}", ioc::nr(cmd)),
        size = ioc::size(cmd),
        "dispatch",
    );
    let rc = dispatch_ioctl_with(remote, kind, cmd, arg, host_fd);
    tracing::debug!(
        ?kind,
        nr = format_args!("0x{:02x}", ioc::nr(cmd)),
        rc,
        "dispatch result",
    );
    rc
}

// ---------------------------------------------------------------------------
// LD_PRELOAD symbol exports
//
// These resolve the next symbol in the dynamic link chain via
// `dlsym(RTLD_NEXT, ...)` so non-GPU paths fall through unchanged.

fn next_symbol(name: &CStr) -> *mut c_void {
    // SAFETY: dlsym with RTLD_NEXT is defined behaviour in glibc.
    unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) }
}

macro_rules! next_fn {
    ($name:ident : fn($($arg:ident : $ty:ty),*) -> $ret:ty) => {{
        static SYM: OnceLock<usize> = OnceLock::new();
        let p = *SYM.get_or_init(|| {
            let s = CString::new(stringify!($name)).unwrap();
            next_symbol(&s) as usize
        });
        if p == 0 {
            None
        } else {
            // SAFETY: the resolved symbol matches libc's declared signature.
            let f: unsafe extern "C" fn($($ty),*) -> $ret = unsafe { std::mem::transmute(p) };
            Some(f)
        }
    }};
}

fn cstr_to_path(p: *const c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    // SAFETY: caller guarantees a NUL-terminated string.
    let s = unsafe { CStr::from_ptr(p) };
    Some(PathBuf::from(s.to_str().ok()?))
}

fn create_cookie_fd() -> c_int {
    // Allocate a real fd so that the program sees a normal integer.
    // A `memfd` is a convenient, non-conflicting source.
    let name = CString::new("mirage-cookie").unwrap();
    // SAFETY: memfd_create is a standard glibc call.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC as u32) };
    fd
}

/// Attempt to open the real device node.  Returns `(fd, synthetic)` where
/// `synthetic == true` means the real device was absent so a memfd
/// placeholder was created instead.
///
/// For KFD devices the interceptor never opens the real `/dev/kfd`;
/// all KFD kernel interactions are the daemon's responsibility.  A
/// synthetic memfd is always used so that mmaps produce anonymous
/// pages rather than requiring device-backed offsets.
fn open_host_path_or_memfd(
    path: *const c_char,
    flags: c_int,
    mode: libc::mode_t,
    kind: DeviceKind,
) -> (c_int, bool) {
    // KFD fds are always synthetic — the daemon owns /dev/kfd.
    if kind != DeviceKind::Kfd {
        if let Some(real) = next_fn!(open : fn(p: *const c_char, f: c_int, m: libc::mode_t) -> c_int) {
            let fd = unsafe { real(path, flags, mode) };
            if fd >= 0 {
                return (fd, false);
            }
        }
    }
    // Real device not available or deliberately skipped — create a memfd
    // placeholder so the rest of the interceptor pipeline (cookie fd,
    // remote emulator) still works.  mmap on this fd will use
    // MAP_ANONYMOUS instead.
    let name = CString::new("mirage-hostfd").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC as u32) };
    (fd, true)
}

fn openat_host_path_or_memfd(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mode: libc::mode_t,
    kind: DeviceKind,
) -> (c_int, bool) {
    // KFD fds are always synthetic — the daemon owns /dev/kfd.
    if kind != DeviceKind::Kfd {
        if let Some(real) =
            next_fn!(openat : fn(d: c_int, p: *const c_char, f: c_int, m: libc::mode_t) -> c_int)
        {
            let fd = unsafe { real(dirfd, path, flags, mode) };
            if fd >= 0 {
                return (fd, false);
            }
        }
    }
    let name = CString::new("mirage-hostfd").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC as u32) };
    (fd, true)
}

fn tracked_path_request(_kind: DeviceKind, path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn stat_request(remote: &RemoteEmulator, kind: DeviceKind, path: &Path) -> Result<FakeStat, c_int> {
    remote
        .syscall_stat_device(
            current_ctx(),
            syscalls::SyscallStatDeviceRequest {
                class: kind.device_class(),
                path: tracked_path_request(kind, path),
            },
        )
        .map(|resp| resp.stat)
        .map_err(|err| errno_to_rc(err.errno()))
}

unsafe fn fill_fake_stat(buf: *mut libc::stat, stat: FakeStat) {
    unsafe { std::ptr::write_bytes(buf, 0, 1) };
    unsafe {
        (*buf).st_mode = stat.mode as libc::mode_t;
        (*buf).st_nlink = stat.nlink as libc::nlink_t;
        (*buf).st_rdev = stat.rdev as libc::dev_t;
        (*buf).st_size = stat.size as libc::off_t;
    }
}

unsafe fn fill_fake_stat64(buf: *mut libc::stat64, stat: FakeStat) {
    unsafe { std::ptr::write_bytes(buf, 0, 1) };
    unsafe {
        (*buf).st_mode = stat.mode as libc::mode_t;
        (*buf).st_nlink = stat.nlink as libc::nlink_t;
        (*buf).st_rdev = stat.rdev as libc::dev_t;
        (*buf).st_size = stat.size as libc::off64_t;
    }
}

fn tracked_fd_from_proc_path(path: &Path) -> Option<TrackedFd> {
    let s = path.to_str()?;
    let suffix = s.strip_prefix("/proc/self/fd/")?;
    let fd = suffix.parse::<c_int>().ok()?;
    lookup_entry(fd)
}

/// `open(const char *, int, ...)` — variadic in C. We match the two
/// common forms (with and without mode) to avoid the varargs dance.
///
/// # Safety
///
/// Called by the dynamic linker on behalf of user code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: libc::mode_t) -> c_int {
    if let Some(p) = cstr_to_path(path) {
        let ps = p.to_str().unwrap_or("");
        if ps.contains("dri") || ps.contains("kfd") || ps.contains("render") || ps.contains("gpu") {
            tracing::debug!(path = %p.display(), "open ALL");
        }
        // Serve topology files from cached Topology instead of hitting
        // the filesystem or forwarding to the daemon.
        if is_topology_path(ps) {
            if let Some(data) = topology_lookup(ps) {
                let fd = memfd_from_topology_data(data);
                if fd >= 0 {
                    return fd;
                }
            }
            // Fall through to real open (may hit bind-mounted topology).
        }
        if let Some(kind) = DeviceKind::classify(&p)
            && let Some(remote) = remote()
        {
            tracing::debug!(path = %p.display(), ?kind, "open");
            let (host_fd, synthetic_host) = open_host_path_or_memfd(path, flags, mode, kind);
            if host_fd < 0 {
                return host_fd;
            }
            let fake_stat = match stat_request(remote, kind, &p) {
                Ok(stat) => stat,
                Err(rc) => {
                    if let Some(real_close) = next_fn!(close : fn(f: c_int) -> c_int) {
                        unsafe { real_close(host_fd) };
                    }
                    return rc;
                }
            };
            let remote_fd = match remote.syscall_open(
                current_ctx(),
                syscalls::SyscallOpenRequest {
                    path: tracked_path_request(kind, &p),
                    flags: flags as u32,
                    mode: mode as u32,
                    class: kind.device_class(),
                },
            ) {
                Ok(resp) => resp.virtual_fd,
                Err(err) => {
                    if let Some(real_close) = next_fn!(close : fn(f: c_int) -> c_int) {
                        unsafe { real_close(host_fd) };
                    }
                    return errno_to_rc(err.errno());
                }
            };
            let fd = create_cookie_fd();
            if fd < 0 {
                let _ = remote.syscall_close(
                    current_ctx(),
                    syscalls::SyscallCloseRequest {
                        virtual_fd: remote_fd,
                    },
                );
                if let Some(real_close) = next_fn!(close : fn(f: c_int) -> c_int) {
                    unsafe { real_close(host_fd) };
                }
                return fd;
            }
            register_entry(TrackedFd {
                cookie_fd: fd,
                remote_fd,
                host_fd,
                synthetic_host,
                kind,
                path: p,
                fake_stat,
            });
            return fd;
        }
    }
    let Some(real) = next_fn!(open : fn(p: *const c_char, f: c_int, m: libc::mode_t) -> c_int)
    else {
        return errno_to_rc(libc::ENOSYS);
    };
    // SAFETY: forwarded to libc with identical signature.
    unsafe { real(path, flags | O_CLOEXEC & O_CLOEXEC, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openat(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mode: libc::mode_t,
) -> c_int {
    if let Some(p) = cstr_to_path(path) {
        let ps = p.to_str().unwrap_or("");
        if ps.contains("dri") || ps.contains("kfd") || ps.contains("render") || ps.contains("gpu") {
            tracing::debug!(dirfd, path = %p.display(), "openat ALL");
        }
        // Serve topology files from cached Topology.
        if dirfd == libc::AT_FDCWD && is_topology_path(ps) {
            if let Some(data) = topology_lookup(ps) {
                let fd = memfd_from_topology_data(data);
                if fd >= 0 {
                    return fd;
                }
            }
        }
    }
    if dirfd == libc::AT_FDCWD
        && let Some(p) = cstr_to_path(path)
        && let Some(kind) = DeviceKind::classify(&p)
        && let Some(remote) = remote()
    {
        tracing::debug!(path = %p.display(), ?kind, "openat");
        let (host_fd, synthetic_host) = openat_host_path_or_memfd(dirfd, path, flags, mode, kind);
        if host_fd < 0 {
            return host_fd;
        }
        let fake_stat = match stat_request(remote, kind, &p) {
            Ok(stat) => stat,
            Err(rc) => {
                if let Some(real_close) = next_fn!(close : fn(f: c_int) -> c_int) {
                    unsafe { real_close(host_fd) };
                }
                return rc;
            }
        };
        let remote_fd = match remote.syscall_open(
            current_ctx(),
            syscalls::SyscallOpenRequest {
                path: tracked_path_request(kind, &p),
                flags: flags as u32,
                mode: mode as u32,
                class: kind.device_class(),
            },
        ) {
            Ok(resp) => resp.virtual_fd,
            Err(err) => {
                if let Some(real_close) = next_fn!(close : fn(f: c_int) -> c_int) {
                    unsafe { real_close(host_fd) };
                }
                return errno_to_rc(err.errno());
            }
        };
        let fd = create_cookie_fd();
        if fd < 0 {
            let _ = remote.syscall_close(
                current_ctx(),
                syscalls::SyscallCloseRequest {
                    virtual_fd: remote_fd,
                },
            );
            if let Some(real_close) = next_fn!(close : fn(f: c_int) -> c_int) {
                unsafe { real_close(host_fd) };
            }
            return fd;
        }
        register_entry(TrackedFd {
            cookie_fd: fd,
            remote_fd,
            host_fd,
            synthetic_host,
            kind,
            path: p,
            fake_stat,
        });
        return fd;
    }
    let Some(real) =
        next_fn!(openat : fn(d: c_int, p: *const c_char, f: c_int, m: libc::mode_t) -> c_int)
    else {
        return errno_to_rc(libc::ENOSYS);
    };
    // SAFETY: forwarded to libc.
    unsafe { real(dirfd, path, flags, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn open64(path: *const c_char, flags: c_int, mode: libc::mode_t) -> c_int {
    unsafe { open(path, flags, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openat64(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mode: libc::mode_t,
) -> c_int {
    unsafe { openat(dirfd, path, flags, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __open_2(path: *const c_char, flags: c_int) -> c_int {
    unsafe { open(path, flags, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __open64_2(path: *const c_char, flags: c_int) -> c_int {
    unsafe { open(path, flags, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __openat_2(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int {
    unsafe { openat(dirfd, path, flags, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __openat64_2(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int {
    unsafe { openat(dirfd, path, flags, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    if let Some(entry) = lookup_entry(fd) {
        tracing::debug!(
            cookie_fd = entry.cookie_fd,
            remote_fd = entry.remote_fd,
            host_fd = entry.host_fd,
            kind = ?entry.kind,
            "close",
        );
        if entry.remote_fd >= 0
            && !has_other_aliases(entry.remote_fd, fd)
            && let Some(remote) = remote()
        {
            let _ = remote.syscall_close(
                current_ctx(),
                syscalls::SyscallCloseRequest {
                    virtual_fd: entry.remote_fd,
                },
            );
        }
        if entry.host_fd >= 0
            && !has_other_host_aliases(entry.host_fd, fd)
            && let Some(real_close) = next_fn!(close : fn(f: c_int) -> c_int)
        {
            unsafe { real_close(entry.host_fd) };
        }
        forget_fd(fd);
    }
    let Some(real) = next_fn!(close : fn(f: c_int) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    // SAFETY: forwarded to libc.
    unsafe { real(fd) }
}

/// `ioctl(int, unsigned long, ...)` — we match the common
/// `(fd, cmd, void *)` shape.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn ioctl(fd: c_int, cmd: libc::c_ulong, arg: *mut c_void) -> c_int {
    // Guard against TLS being destroyed during process exit.
    // The HSA runtime calls ioctl from shutdown hooks after TLS is gone.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some(kind) = lookup_fd(fd) {
            return dispatch_tracked_ioctl(kind, fd, cmd as u32, arg);
        }
        let Some(real) = next_fn!(ioctl : fn(f: c_int, c: libc::c_ulong, a: *mut c_void) -> c_int)
        else {
            return errno_to_rc(libc::ENOSYS);
        };
        // SAFETY: forwarded to libc.
        unsafe { real(fd, cmd, arg) }
    }));
    match result {
        Ok(rc) => rc,
        Err(_) => errno_to_rc(libc::ENOSYS),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn drmIoctl(fd: c_int, request: c_ulong, arg: *mut c_void) -> c_int {
    if let Some(kind) = lookup_fd(fd) {
        return dispatch_tracked_ioctl(kind, fd, request as u32, arg);
    }
    let Some(real) = next_fn!(drmIoctl : fn(f: c_int, r: c_ulong, a: *mut c_void) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    unsafe { real(fd, request, arg) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn drmCommandWriteRead(
    fd: c_int,
    drm_command_index: c_ulong,
    data: *mut c_void,
    size: c_ulong,
) -> c_int {
    if let Some(DeviceKind::DrmRender) = lookup_fd(fd) {
        let cmd = uapi_ioc::iowr(
            uapi_ioc::DRM_MAGIC,
            uapi_ioc::DRM_COMMAND_BASE + drm_command_index as u32,
            size as u32,
        );
        return dispatch_tracked_ioctl(DeviceKind::DrmRender, fd, cmd, data);
    }
    let Some(real) = next_fn!(drmCommandWriteRead : fn(f: c_int, i: c_ulong, d: *mut c_void, s: c_ulong) -> c_int)
    else {
        return errno_to_rc(libc::ENOSYS);
    };
    unsafe { real(fd, drm_command_index, data, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn drmCommandWrite(
    fd: c_int,
    drm_command_index: c_ulong,
    data: *mut c_void,
    size: c_ulong,
) -> c_int {
    tracing::debug!(
        fd,
        index = drm_command_index,
        size,
        tracked = ?lookup_fd(fd),
        "drmCommandWrite",
    );
    if let Some(DeviceKind::DrmRender) = lookup_fd(fd) {
        let cmd = uapi_ioc::iow(
            uapi_ioc::DRM_MAGIC,
            uapi_ioc::DRM_COMMAND_BASE + drm_command_index as u32,
            size as u32,
        );
        return dispatch_tracked_ioctl(DeviceKind::DrmRender, fd, cmd, data);
    }
    let Some(real) =
        next_fn!(drmCommandWrite : fn(f: c_int, i: c_ulong, d: *mut c_void, s: c_ulong) -> c_int)
    else {
        return errno_to_rc(libc::ENOSYS);
    };
    unsafe { real(fd, drm_command_index, data, size) }
}

fn register_alias(new_fd: c_int, source_fd: c_int) {
    if new_fd < 0 {
        return;
    }
    let Some(mut entry) = lookup_entry(source_fd) else {
        return;
    };
    tracing::debug!(
        source_fd,
        new_fd,
        remote_fd = entry.remote_fd,
        kind = ?entry.kind,
        "alias",
    );
    if entry.remote_fd >= 0
        && let Some(remote) = remote()
    {
        let _ = remote.syscall_dup(
            current_ctx(),
            syscalls::SyscallDupRequest {
                virtual_fd: entry.remote_fd,
            },
        );
    }
    entry.cookie_fd = new_fd;
    register_entry(entry);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dup(fd: c_int) -> c_int {
    let Some(real) = next_fn!(dup : fn(f: c_int) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    let new_fd = unsafe { real(fd) };
    register_alias(new_fd, fd);
    new_fd
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dup2(fd: c_int, new_fd: c_int) -> c_int {
    let Some(real) = next_fn!(dup2 : fn(f: c_int, n: c_int) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    let rc = unsafe { real(fd, new_fd) };
    if rc >= 0 {
        register_alias(rc, fd);
    }
    rc
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dup3(fd: c_int, new_fd: c_int, flags: c_int) -> c_int {
    let Some(real) = next_fn!(dup3 : fn(f: c_int, n: c_int, fl: c_int) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    let rc = unsafe { real(fd, new_fd, flags) };
    if rc >= 0 {
        register_alias(rc, fd);
    }
    rc
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fcntl(fd: c_int, cmd: c_int, arg: c_long) -> c_int {
    let Some(real) = next_fn!(fcntl : fn(f: c_int, c: c_int, a: c_long) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    let rc = unsafe { real(fd, cmd, arg) };
    if matches!(cmd, libc::F_DUPFD | libc::F_DUPFD_CLOEXEC) && rc >= 0 {
        register_alias(rc, fd);
    }
    rc
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fcntl64(fd: c_int, cmd: c_int, arg: c_long) -> c_int {
    let Some(real) = next_fn!(fcntl64 : fn(f: c_int, c: c_int, a: c_long) -> c_int) else {
        return unsafe { fcntl(fd, cmd, arg) };
    };
    let rc = unsafe { real(fd, cmd, arg) };
    if matches!(cmd, libc::F_DUPFD | libc::F_DUPFD_CLOEXEC) && rc >= 0 {
        register_alias(rc, fd);
    }
    rc
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn access(path: *const c_char, mode: c_int) -> c_int {
    if let Some(p) = cstr_to_path(path)
        && let Some(kind) = DeviceKind::classify(&p)
        && let Some(remote) = remote()
    {
        return match remote.syscall_access(
            current_ctx(),
            syscalls::SyscallAccessRequest {
                class: kind.device_class(),
                path: tracked_path_request(kind, &p),
                mode: mode as u32,
            },
        ) {
            Ok(resp) if resp.allowed => 0,
            Ok(_) => errno_to_rc(libc::EACCES),
            Err(err) => errno_to_rc(err.errno()),
        };
    }
    let Some(real) = next_fn!(access : fn(p: *const c_char, m: c_int) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    unsafe { real(path, mode) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, count: size_t) -> libc::ssize_t {
    if let Some(entry) = lookup_entry(fd) {
        tracing::debug!(
            fd,
            remote_fd = entry.remote_fd,
            host_fd = entry.host_fd,
            count,
            "read",
        );
        if entry.host_fd >= 0 {
            let Some(real) =
                next_fn!(read : fn(f: c_int, b: *mut c_void, c: size_t) -> libc::ssize_t)
            else {
                return errno_to_rc(libc::ENOSYS) as libc::ssize_t;
            };
            return unsafe { real(entry.host_fd, buf, count) };
        }
        if entry.remote_fd >= 0
            && let Some(remote) = remote()
        {
            match remote.syscall_read_device(
                current_ctx(),
                syscalls::SyscallReadDeviceRequest {
                    virtual_fd: entry.remote_fd,
                    count: count as u64,
                },
            ) {
                Ok(resp) => {
                    let len = resp.data.len().min(count);
                    if len > 0 {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                resp.data.as_ptr(),
                                buf.cast::<u8>(),
                                len,
                            );
                        }
                    }
                    return len as libc::ssize_t;
                }
                Err(err) => return errno_to_rc(err.errno()) as libc::ssize_t,
            }
        }
    }
    let Some(real) = next_fn!(read : fn(f: c_int, b: *mut c_void, c: size_t) -> libc::ssize_t)
    else {
        return errno_to_rc(libc::ENOSYS) as libc::ssize_t;
    };
    unsafe { real(fd, buf, count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mmap(
    addr: *mut c_void,
    length: size_t,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: libc::off_t,
) -> *mut c_void {
    if let Some(entry) = lookup_entry(fd) {
        tracing::debug!(
            fd,
            remote_fd = entry.remote_fd,
            host_fd = entry.host_fd,
            synthetic = entry.synthetic_host,
            len = length,
            prot = format_args!("0x{:x}", prot),
            flags = format_args!("0x{:x}", flags),
            offset,
            "mmap",
        );
        let Some(real) = next_fn!(mmap : fn(a: *mut c_void, l: size_t, p: c_int, f: c_int, d: c_int, o: libc::off_t) -> *mut c_void)
        else {
            errno_to_rc(libc::ENOSYS);
            return libc::MAP_FAILED;
        };
        // When a daemon is present (remote_fd >= 0) the interceptor
        // never owns kernel resources directly — handles and mmap
        // offsets belong to the daemon's process context.  Provide
        // anonymous memory so the application gets a valid, writable
        // mapping.  The daemon is responsible for the actual
        // device-backed mapping on its side.
        if entry.remote_fd >= 0 || entry.synthetic_host {
            return unsafe { real(addr, length, prot, flags | libc::MAP_ANONYMOUS, -1, 0) };
        }
        if entry.host_fd >= 0 {
            return unsafe { real(addr, length, prot, flags, entry.host_fd, offset) };
        }
    }
    let Some(real) = next_fn!(mmap : fn(a: *mut c_void, l: size_t, p: c_int, f: c_int, d: c_int, o: libc::off_t) -> *mut c_void)
    else {
        errno_to_rc(libc::ENOSYS);
        return libc::MAP_FAILED;
    };
    unsafe { real(addr, length, prot, flags, fd, offset) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn munmap(addr: *mut c_void, length: size_t) -> c_int {
    tracing::debug!(?addr, len = length, "munmap");
    let Some(real) = next_fn!(munmap : fn(a: *mut c_void, l: size_t) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    unsafe { real(addr, length) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn readlink(
    path: *const c_char,
    buf: *mut c_char,
    bufsiz: size_t,
) -> libc::ssize_t {
    if let Some(p) = cstr_to_path(path)
        && let Some(entry) = tracked_fd_from_proc_path(&p)
    {
        let bytes = entry.path.as_os_str().as_bytes();
        if bufsiz == 0 || buf.is_null() {
            return 0;
        }
        let len = bytes.len().min(bufsiz);
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.cast::<u8>(), len) };
        return len as libc::ssize_t;
    }
    let Some(real) =
        next_fn!(readlink : fn(p: *const c_char, b: *mut c_char, n: size_t) -> libc::ssize_t)
    else {
        return errno_to_rc(libc::ENOSYS) as libc::ssize_t;
    };
    unsafe { real(path, buf, bufsiz) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat(fd: c_int, statbuf: *mut libc::stat) -> c_int {
    if let Some(entry) = lookup_entry(fd) {
        unsafe { fill_fake_stat(statbuf, entry.fake_stat) };
        return 0;
    }
    let Some(real) = next_fn!(fstat : fn(f: c_int, s: *mut libc::stat) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    unsafe { real(fd, statbuf) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat64(fd: c_int, statbuf: *mut libc::stat64) -> c_int {
    if let Some(entry) = lookup_entry(fd) {
        unsafe { fill_fake_stat64(statbuf, entry.fake_stat) };
        return 0;
    }
    let Some(real) = next_fn!(fstat64 : fn(f: c_int, s: *mut libc::stat64) -> c_int) else {
        return errno_to_rc(libc::ENOSYS);
    };
    unsafe { real(fd, statbuf) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __fxstat(version: c_int, fd: c_int, statbuf: *mut libc::stat) -> c_int {
    let _ = version;
    unsafe { fstat(fd, statbuf) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __fxstat64(
    version: c_int,
    fd: c_int,
    statbuf: *mut libc::stat64,
) -> c_int {
    let _ = version;
    unsafe { fstat64(fd, statbuf) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn newfstatat(
    dirfd: c_int,
    path: *const c_char,
    statbuf: *mut libc::stat,
    flags: c_int,
) -> c_int {
    let is_empty_path = !path.is_null() && unsafe { *path } == 0;
    if (flags & libc::AT_EMPTY_PATH) != 0
        && is_empty_path
        && let Some(entry) = lookup_entry(dirfd)
    {
        unsafe { fill_fake_stat(statbuf, entry.fake_stat) };
        return 0;
    }
    let Some(real) = next_fn!(newfstatat : fn(d: c_int, p: *const c_char, s: *mut libc::stat, f: c_int) -> c_int)
    else {
        return errno_to_rc(libc::ENOSYS);
    };
    unsafe { real(dirfd, path, statbuf, flags) }
}

// Silence "unused" for the mode size helper.
#[allow(dead_code)]
fn _mode_size_guard() -> size_t {
    0
}

#[cfg(test)]
mod tests;
