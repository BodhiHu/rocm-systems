//! [`RealEmulator`] — the [`Emulator`](mirage_schema::emulator::Emulator)
//! implementation that forwards every ioctl to a host AMD GPU.
//!
//! This crate intentionally holds *real* file descriptors to `/dev/kfd`
//! and the DRM render nodes, and dispatches each typed ioctl request
//! down to the kernel via `libc::ioctl`. A matching `RemoteEmulator`
//! running in an intercepted process therefore sees exactly the
//! behaviour the host kernel would provide.
//!
//! # Scope
//!
//! Every variant of [`AnyKfdIoctlRequest`] and [`AnyDrmIoctlRequest`] is
//! dispatched through a single match inside [`RealEmulator`]. Variants
//! that are not yet wired to a raw kernel ioctl return [`AmdgpuError::NoSys`]
//! so that unsupported paths surface cleanly rather than as memory
//! corruption. [`AMDKFD_IOC_GET_VERSION`] is wired as a worked example.
//!
//! # Availability
//!
//! [`RealEmulator::detect`] returns `None` on machines without a KFD
//! device — tests use this to skip gracefully.

use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mirage_schema::amdgpu::{
    AmdkfdIocGetVersionResponse, HandleAnyDrmIoctl, HandleAnyKfdIoctl, HandleDrmIoctl,
    HandleKfdIoctl, IoctlCtx,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_schema::syscalls::{HandleAnyFsSyscalls, HandleFsSyscalls};

macro_rules! nosys_methods {
    ($(fn $method:ident($request:ty) -> $response:ty;)*) => {
        $(
            fn $method(
                &self,
                _ctx: IoctlCtx,
                _request: $request,
            ) -> AmdgpuResult<$response> {
                Err(AmdgpuError::NoSys)
            }
        )*
    };
}

const KFD_DEVICE_PATH: &str = "/dev/kfd";
const DRI_DIR: &str = "/dev/dri";

/// Linux `_IO` direction bits (same layout as `<asm-generic/ioctl.h>`).
mod ioc {
    pub const NRBITS: u32 = 8;
    pub const TYPEBITS: u32 = 8;
    pub const SIZEBITS: u32 = 14;

    pub const NRSHIFT: u32 = 0;
    pub const TYPESHIFT: u32 = NRSHIFT + NRBITS;
    pub const SIZESHIFT: u32 = TYPESHIFT + TYPEBITS;
    pub const DIRSHIFT: u32 = SIZESHIFT + SIZEBITS;

    #[allow(dead_code)]
    pub const NONE: u32 = 0;
    pub const WRITE: u32 = 1;
    pub const READ: u32 = 2;

    pub const fn ioc(dir: u32, ty: u32, nr: u32, size: u32) -> u32 {
        (dir << DIRSHIFT) | (ty << TYPESHIFT) | (nr << NRSHIFT) | (size << SIZESHIFT)
    }

    pub const fn iowr(ty: u32, nr: u32, size: u32) -> u32 {
        ioc(READ | WRITE, ty, nr, size)
    }
}

/// KFD char device ioctl type.
const KFDIOC_MAGIC: u32 = b'K' as u32;

/// Handle to the real hardware.
pub struct RealEmulator {
    kfd: OwnedFd,
    /// DRM render nodes, keyed by minor number so the GPU index is stable
    /// across calls. Lazily populated on first access. A `Mutex` is fine
    /// here — these are not hot paths compared to ioctl round-trips.
    render_nodes: Mutex<Vec<RenderNode>>,
}

struct RenderNode {
    path: PathBuf,
    #[allow(dead_code)] // held open to keep the kernel-side fd alive
    fd: OwnedFd,
}

impl std::fmt::Debug for RealEmulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealEmulator")
            .field("kfd_fd", &self.kfd.as_raw_fd())
            .finish_non_exhaustive()
    }
}

impl RealEmulator {
    /// Return `true` if this host has a KFD device node at all, meaning
    /// [`RealEmulator::detect`] has a chance of succeeding.
    pub fn hardware_available() -> bool {
        Path::new(KFD_DEVICE_PATH).exists()
    }

    /// Open `/dev/kfd` and enumerate `/dev/dri/renderD*`. Returns `None`
    /// if no KFD device is present, otherwise propagates the underlying
    /// `io::Error`.
    pub fn detect() -> io::Result<Option<Self>> {
        if !Self::hardware_available() {
            return Ok(None);
        }
        let kfd = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(KFD_DEVICE_PATH)?;
        let render_nodes = Self::open_render_nodes();
        Ok(Some(Self {
            kfd: kfd.into(),
            render_nodes: Mutex::new(render_nodes),
        }))
    }

    fn open_render_nodes() -> Vec<RenderNode> {
        let mut nodes = Vec::new();
        let Ok(dir) = std::fs::read_dir(DRI_DIR) else {
            return nodes;
        };
        let mut entries: Vec<_> = dir
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            if let Ok(fd) = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(entry.path())
            {
                nodes.push(RenderNode {
                    path: entry.path(),
                    fd: fd.into(),
                });
            }
        }
        nodes
    }

    /// List of DRM render-node paths successfully opened.
    pub fn render_node_paths(&self) -> Vec<PathBuf> {
        self.render_nodes
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.path.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Raw ioctl helpers.

/// `ioctl(fd, nr, &mut arg)` — returns the kernel errno on failure.
///
/// # Safety
///
/// Caller must ensure that `arg` is a valid pointer of the exact C
/// layout expected by the driver for ioctl number `nr`.
unsafe fn raw_ioctl<T>(fd: i32, nr: u32, arg: &mut T) -> AmdgpuResult<()> {
    // SAFETY: caller invariants.
    let rc = unsafe { libc::ioctl(fd, nr as _, arg as *mut T) };
    if rc == 0 {
        Ok(())
    } else {
        let err = io::Error::last_os_error();
        Err(err
            .raw_os_error()
            .map(AmdgpuError::from_errno)
            .unwrap_or(AmdgpuError::Io))
    }
}

// --- KFD: GET_VERSION -------------------------------------------------------

#[repr(C)]
struct KfdIocGetVersionArgs {
    major: u32,
    minor: u32,
}

impl RealEmulator {
    fn kfd_get_version(&self) -> AmdgpuResult<AmdkfdIocGetVersionResponse> {
        let nr = ioc::iowr(
            KFDIOC_MAGIC,
            0x01,
            core::mem::size_of::<KfdIocGetVersionArgs>() as u32,
        );
        let mut args = KfdIocGetVersionArgs { major: 0, minor: 0 };
        // SAFETY: `args` has the exact layout the kernel expects.
        unsafe { raw_ioctl(self.kfd.as_raw_fd(), nr, &mut args)? };
        Ok(AmdkfdIocGetVersionResponse {
            major_version: args.major,
            minor_version: args.minor,
        })
    }
}

// ---------------------------------------------------------------------------
// Direct handler impls.

impl HandleKfdIoctl for RealEmulator {
    fn amdkfd_ioc_get_version(
        &self,
        _ctx: IoctlCtx,
        _request: mirage_schema::amdgpu::AmdkfdIocGetVersionRequest,
    ) -> AmdgpuResult<mirage_schema::amdgpu::AmdkfdIocGetVersionResponse> {
        self.kfd_get_version()
    }

    nosys_methods!(
        fn amdkfd_ioc_create_queue(mirage_schema::amdgpu::AmdkfdIocCreateQueueRequest) -> mirage_schema::amdgpu::AmdkfdIocCreateQueueResponse;
        fn amdkfd_ioc_destroy_queue(mirage_schema::amdgpu::AmdkfdIocDestroyQueueRequest) -> mirage_schema::amdgpu::AmdkfdIocDestroyQueueResponse;
        fn amdkfd_ioc_set_memory_policy(mirage_schema::amdgpu::AmdkfdIocSetMemoryPolicyRequest) -> mirage_schema::amdgpu::AmdkfdIocSetMemoryPolicyResponse;
        fn amdkfd_ioc_get_clock_counters(mirage_schema::amdgpu::AmdkfdIocGetClockCountersRequest) -> mirage_schema::amdgpu::AmdkfdIocGetClockCountersResponse;
        fn amdkfd_ioc_get_process_apertures(mirage_schema::amdgpu::AmdkfdIocGetProcessAperturesRequest) -> mirage_schema::amdgpu::AmdkfdIocGetProcessAperturesResponse;
        fn amdkfd_ioc_update_queue(mirage_schema::amdgpu::AmdkfdIocUpdateQueueRequest) -> mirage_schema::amdgpu::AmdkfdIocUpdateQueueResponse;
        fn amdkfd_ioc_create_event(mirage_schema::amdgpu::AmdkfdIocCreateEventRequest) -> mirage_schema::amdgpu::AmdkfdIocCreateEventResponse;
        fn amdkfd_ioc_destroy_event(mirage_schema::amdgpu::AmdkfdIocDestroyEventRequest) -> mirage_schema::amdgpu::AmdkfdIocDestroyEventResponse;
        fn amdkfd_ioc_set_event(mirage_schema::amdgpu::AmdkfdIocSetEventRequest) -> mirage_schema::amdgpu::AmdkfdIocSetEventResponse;
        fn amdkfd_ioc_reset_event(mirage_schema::amdgpu::AmdkfdIocResetEventRequest) -> mirage_schema::amdgpu::AmdkfdIocResetEventResponse;
        fn amdkfd_ioc_wait_events(mirage_schema::amdgpu::AmdkfdIocWaitEventsRequest) -> mirage_schema::amdgpu::AmdkfdIocWaitEventsResponse;
        fn amdkfd_ioc_dbg_register_deprecated(mirage_schema::amdgpu::AmdkfdIocDbgRegisterDeprecatedRequest) -> mirage_schema::amdgpu::AmdkfdIocDbgRegisterDeprecatedResponse;
        fn amdkfd_ioc_dbg_unregister_deprecated(mirage_schema::amdgpu::AmdkfdIocDbgUnregisterDeprecatedRequest) -> mirage_schema::amdgpu::AmdkfdIocDbgUnregisterDeprecatedResponse;
        fn amdkfd_ioc_dbg_address_watch_deprecated(mirage_schema::amdgpu::AmdkfdIocDbgAddressWatchDeprecatedRequest) -> mirage_schema::amdgpu::AmdkfdIocDbgAddressWatchDeprecatedResponse;
        fn amdkfd_ioc_dbg_wave_control_deprecated(mirage_schema::amdgpu::AmdkfdIocDbgWaveControlDeprecatedRequest) -> mirage_schema::amdgpu::AmdkfdIocDbgWaveControlDeprecatedResponse;
        fn amdkfd_ioc_set_scratch_backing_va(mirage_schema::amdgpu::AmdkfdIocSetScratchBackingVaRequest) -> mirage_schema::amdgpu::AmdkfdIocSetScratchBackingVaResponse;
        fn amdkfd_ioc_get_tile_config(mirage_schema::amdgpu::AmdkfdIocGetTileConfigRequest) -> mirage_schema::amdgpu::AmdkfdIocGetTileConfigResponse;
        fn amdkfd_ioc_set_trap_handler(mirage_schema::amdgpu::AmdkfdIocSetTrapHandlerRequest) -> mirage_schema::amdgpu::AmdkfdIocSetTrapHandlerResponse;
        fn amdkfd_ioc_get_process_apertures_new(mirage_schema::amdgpu::AmdkfdIocGetProcessAperturesNewRequest) -> mirage_schema::amdgpu::AmdkfdIocGetProcessAperturesNewResponse;
        fn amdkfd_ioc_acquire_vm(mirage_schema::amdgpu::AmdkfdIocAcquireVmRequest) -> mirage_schema::amdgpu::AmdkfdIocAcquireVmResponse;
        fn amdkfd_ioc_alloc_memory_of_gpu(mirage_schema::amdgpu::AmdkfdIocAllocMemoryOfGpuRequest) -> mirage_schema::amdgpu::AmdkfdIocAllocMemoryOfGpuResponse;
        fn amdkfd_ioc_free_memory_of_gpu(mirage_schema::amdgpu::AmdkfdIocFreeMemoryOfGpuRequest) -> mirage_schema::amdgpu::AmdkfdIocFreeMemoryOfGpuResponse;
        fn amdkfd_ioc_map_memory_to_gpu(mirage_schema::amdgpu::AmdkfdIocMapMemoryToGpuRequest) -> mirage_schema::amdgpu::AmdkfdIocMapMemoryToGpuResponse;
        fn amdkfd_ioc_unmap_memory_from_gpu(mirage_schema::amdgpu::AmdkfdIocUnmapMemoryFromGpuRequest) -> mirage_schema::amdgpu::AmdkfdIocUnmapMemoryFromGpuResponse;
        fn amdkfd_ioc_set_cu_mask(mirage_schema::amdgpu::AmdkfdIocSetCuMaskRequest) -> mirage_schema::amdgpu::AmdkfdIocSetCuMaskResponse;
        fn amdkfd_ioc_get_queue_wave_state(mirage_schema::amdgpu::AmdkfdIocGetQueueWaveStateRequest) -> mirage_schema::amdgpu::AmdkfdIocGetQueueWaveStateResponse;
        fn amdkfd_ioc_get_dmabuf_info(mirage_schema::amdgpu::AmdkfdIocGetDmabufInfoRequest) -> mirage_schema::amdgpu::AmdkfdIocGetDmabufInfoResponse;
        fn amdkfd_ioc_import_dmabuf(mirage_schema::amdgpu::AmdkfdIocImportDmabufRequest) -> mirage_schema::amdgpu::AmdkfdIocImportDmabufResponse;
        fn amdkfd_ioc_alloc_queue_gws(mirage_schema::amdgpu::AmdkfdIocAllocQueueGwsRequest) -> mirage_schema::amdgpu::AmdkfdIocAllocQueueGwsResponse;
        fn amdkfd_ioc_smi_events(mirage_schema::amdgpu::AmdkfdIocSmiEventsRequest) -> mirage_schema::amdgpu::AmdkfdIocSmiEventsResponse;
        fn amdkfd_ioc_svm(mirage_schema::amdgpu::AmdkfdIocSvmRequest) -> mirage_schema::amdgpu::AmdkfdIocSvmResponse;
        fn amdkfd_ioc_set_xnack_mode(mirage_schema::amdgpu::AmdkfdIocSetXnackModeRequest) -> mirage_schema::amdgpu::AmdkfdIocSetXnackModeResponse;
        fn amdkfd_ioc_criu_op(mirage_schema::amdgpu::AmdkfdIocCriuOpRequest) -> mirage_schema::amdgpu::AmdkfdIocCriuOpResponse;
        fn amdkfd_ioc_available_memory(mirage_schema::amdgpu::AmdkfdIocAvailableMemoryRequest) -> mirage_schema::amdgpu::AmdkfdIocAvailableMemoryResponse;
        fn amdkfd_ioc_export_dmabuf(mirage_schema::amdgpu::AmdkfdIocExportDmabufRequest) -> mirage_schema::amdgpu::AmdkfdIocExportDmabufResponse;
        fn amdkfd_ioc_runtime_enable(mirage_schema::amdgpu::AmdkfdIocRuntimeEnableRequest) -> mirage_schema::amdgpu::AmdkfdIocRuntimeEnableResponse;
        fn amdkfd_ioc_dbg_trap(mirage_schema::amdgpu::AmdkfdIocDbgTrapRequest) -> mirage_schema::amdgpu::AmdkfdIocDbgTrapResponse;
        fn amdkfd_ioc_create_process(mirage_schema::amdgpu::AmdkfdIocCreateProcessRequest) -> mirage_schema::amdgpu::AmdkfdIocCreateProcessResponse;
        fn amdkfd_ioc_ipc_import_handle(mirage_schema::amdgpu::AmdkfdIocIpcImportHandleRequest) -> mirage_schema::amdgpu::AmdkfdIocIpcImportHandleResponse;
        fn amdkfd_ioc_ipc_export_handle(mirage_schema::amdgpu::AmdkfdIocIpcExportHandleRequest) -> mirage_schema::amdgpu::AmdkfdIocIpcExportHandleResponse;
        fn amdkfd_ioc_cross_memory_copy(mirage_schema::amdgpu::AmdkfdIocCrossMemoryCopyRequest) -> mirage_schema::amdgpu::AmdkfdIocCrossMemoryCopyResponse;
        fn amdkfd_ioc_rlc_spm(mirage_schema::amdgpu::AmdkfdIocRlcSpmRequest) -> mirage_schema::amdgpu::AmdkfdIocRlcSpmResponse;
        fn amdkfd_ioc_pc_sample(mirage_schema::amdgpu::AmdkfdIocPcSampleRequest) -> mirage_schema::amdgpu::AmdkfdIocPcSampleResponse;
        fn amdkfd_ioc_profiler(mirage_schema::amdgpu::AmdkfdIocProfilerRequest) -> mirage_schema::amdgpu::AmdkfdIocProfilerResponse;
        fn amdkfd_ioc_ais_op(mirage_schema::amdgpu::AmdkfdIocAisOpRequest) -> mirage_schema::amdgpu::AmdkfdIocAisOpResponse;
    );
}

impl HandleAnyKfdIoctl for RealEmulator {}

impl HandleDrmIoctl for RealEmulator {
    nosys_methods!(
        fn drm_amdgpu_gem_create(mirage_schema::amdgpu::DrmAmdgpuGemCreateRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemCreateResponse;
        fn drm_amdgpu_gem_mmap(mirage_schema::amdgpu::DrmAmdgpuGemMmapRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemMmapResponse;
        fn drm_amdgpu_ctx(mirage_schema::amdgpu::DrmAmdgpuCtxRequest) -> mirage_schema::amdgpu::DrmAmdgpuCtxResponse;
        fn drm_amdgpu_bo_list(mirage_schema::amdgpu::DrmAmdgpuBoListRequest) -> mirage_schema::amdgpu::DrmAmdgpuBoListResponse;
        fn drm_amdgpu_cs(mirage_schema::amdgpu::DrmAmdgpuCsRequest) -> mirage_schema::amdgpu::DrmAmdgpuCsResponse;
        fn drm_amdgpu_info(mirage_schema::amdgpu::DrmAmdgpuInfoRequest) -> mirage_schema::amdgpu::DrmAmdgpuInfoResponse;
        fn drm_amdgpu_gem_metadata(mirage_schema::amdgpu::DrmAmdgpuGemMetadataRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemMetadataResponse;
        fn drm_amdgpu_gem_wait_idle(mirage_schema::amdgpu::DrmAmdgpuGemWaitIdleRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemWaitIdleResponse;
        fn drm_amdgpu_gem_va(mirage_schema::amdgpu::DrmAmdgpuGemVaRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemVaResponse;
        fn drm_amdgpu_wait_cs(mirage_schema::amdgpu::DrmAmdgpuWaitCsRequest) -> mirage_schema::amdgpu::DrmAmdgpuWaitCsResponse;
        fn drm_amdgpu_gem_op(mirage_schema::amdgpu::DrmAmdgpuGemOpRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemOpResponse;
        fn drm_amdgpu_gem_userptr(mirage_schema::amdgpu::DrmAmdgpuGemUserptrRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemUserptrResponse;
        fn drm_amdgpu_wait_fences(mirage_schema::amdgpu::DrmAmdgpuWaitFencesRequest) -> mirage_schema::amdgpu::DrmAmdgpuWaitFencesResponse;
        fn drm_amdgpu_vm(mirage_schema::amdgpu::DrmAmdgpuVmRequest) -> mirage_schema::amdgpu::DrmAmdgpuVmResponse;
        fn drm_amdgpu_fence_to_handle(mirage_schema::amdgpu::DrmAmdgpuFenceToHandleRequest) -> mirage_schema::amdgpu::DrmAmdgpuFenceToHandleResponse;
        fn drm_amdgpu_sched(mirage_schema::amdgpu::DrmAmdgpuSchedRequest) -> mirage_schema::amdgpu::DrmAmdgpuSchedResponse;
        fn drm_amdgpu_userq(mirage_schema::amdgpu::DrmAmdgpuUserqRequest) -> mirage_schema::amdgpu::DrmAmdgpuUserqResponse;
        fn drm_amdgpu_userq_signal(mirage_schema::amdgpu::DrmAmdgpuUserqSignalRequest) -> mirage_schema::amdgpu::DrmAmdgpuUserqSignalResponse;
        fn drm_amdgpu_userq_wait(mirage_schema::amdgpu::DrmAmdgpuUserqWaitRequest) -> mirage_schema::amdgpu::DrmAmdgpuUserqWaitResponse;
        fn drm_amdgpu_gem_list_handles(mirage_schema::amdgpu::DrmAmdgpuGemListHandlesRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemListHandlesResponse;
        fn drm_amdgpu_sem(mirage_schema::amdgpu::DrmAmdgpuSemRequest) -> mirage_schema::amdgpu::DrmAmdgpuSemResponse;
        fn drm_amdgpu_gem_dgma(mirage_schema::amdgpu::DrmAmdgpuGemDgmaRequest) -> mirage_schema::amdgpu::DrmAmdgpuGemDgmaResponse;
    );
}

impl HandleAnyDrmIoctl for RealEmulator {}

// Filesystem-syscall surface: the real host kernel already provides
// these. We simply proxy the request back to libc (for `stat`-family
// and `access`) or report `NoSys` for the ones that only make sense
// against the *virtual* fd table a `RemoteEmulator` server would own.
impl HandleFsSyscalls for RealEmulator {
    nosys_methods!(
        fn syscall_open(mirage_schema::syscalls::SyscallOpenRequest) -> mirage_schema::syscalls::SyscallOpenResponse;
        fn syscall_close(mirage_schema::syscalls::SyscallCloseRequest) -> mirage_schema::syscalls::SyscallCloseResponse;
        fn syscall_readlink_fd(mirage_schema::syscalls::SyscallReadlinkFdRequest) -> mirage_schema::syscalls::SyscallReadlinkFdResponse;
        fn syscall_mmap(mirage_schema::syscalls::SyscallMmapRequest) -> mirage_schema::syscalls::SyscallMmapResponse;
        fn syscall_munmap(mirage_schema::syscalls::SyscallMunmapRequest) -> mirage_schema::syscalls::SyscallMunmapResponse;
        fn syscall_read_device(mirage_schema::syscalls::SyscallReadDeviceRequest) -> mirage_schema::syscalls::SyscallReadDeviceResponse;
        fn syscall_dup(mirage_schema::syscalls::SyscallDupRequest) -> mirage_schema::syscalls::SyscallDupResponse;
        fn syscall_atfork_child(mirage_schema::syscalls::SyscallAtforkChildRequest) -> mirage_schema::syscalls::SyscallAtforkChildResponse;
    );

    fn syscall_stat_device(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallStatDeviceRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallStatDeviceResponse> {
        stat_real_device(&request.path)
            .map(|stat| mirage_schema::syscalls::SyscallStatDeviceResponse { stat })
    }

    fn syscall_access(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallAccessRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallAccessResponse> {
        let c = std::ffi::CString::new(request.path).map_err(|_| AmdgpuError::Invalid)?;
        // SAFETY: `access` takes a NUL-terminated string.
        let rc = unsafe { libc::access(c.as_ptr(), request.mode as i32) };
        Ok(mirage_schema::syscalls::SyscallAccessResponse { allowed: rc == 0 })
    }

    fn syscall_sysfs_read(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallSysfsReadRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallSysfsReadResponse> {
        let mut data = std::fs::read(&request.path).map_err(|e| {
            e.raw_os_error()
                .map(AmdgpuError::from_errno)
                .unwrap_or(AmdgpuError::NoEntry)
        })?;
        data.truncate(request.max_size as usize);
        Ok(mirage_schema::syscalls::SyscallSysfsReadResponse { data })
    }
}

impl HandleAnyFsSyscalls for RealEmulator {}

fn stat_real_device(path: &str) -> AmdgpuResult<mirage_schema::syscalls::FakeStat> {
    let c = std::ffi::CString::new(path).map_err(|_| AmdgpuError::Invalid)?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `stat(2)` with a NUL-terminated path and valid out pointer.
    let rc = unsafe { libc::stat(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()
            .raw_os_error()
            .map(AmdgpuError::from_errno)
            .unwrap_or(AmdgpuError::Io));
    }
    Ok(mirage_schema::syscalls::FakeStat {
        mode: st.st_mode as u32,
        nlink: st.st_nlink as u32,
        rdev: st.st_rdev as u64,
        size: st.st_size as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_hardware_presence_without_panicking() {
        // Whatever the answer, this must not crash.
        let _ = RealEmulator::hardware_available();
    }

    #[test]
    fn detect_returns_none_without_kfd() {
        if RealEmulator::hardware_available() {
            // skip on hosts with real hardware
            return;
        }
        assert!(RealEmulator::detect().unwrap().is_none());
    }

    #[test]
    fn get_version_succeeds_when_hardware_present() {
        let Some(emu) = RealEmulator::detect().unwrap() else {
            eprintln!("no /dev/kfd; skipping");
            return;
        };
        let resp = emu.kfd_get_version().expect("get_version on real hw");
        assert!(resp.major_version >= 1, "kfd reports version {resp:?}");
    }
}
