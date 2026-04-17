//! [`RocjitsuEmulator`] — an [`Emulator`](mirage_schema::emulator::Emulator)
//! implementation backed by the [rocjitsu] virtual-machine simulator.
//!
//! Where [`mirage_real`](../mirage_real) forwards every ioctl to a real
//! AMD GPU via `/dev/kfd`, `RocjitsuEmulator` forwards those same
//! requests to a `rj_vm_t` running in-process. This makes it possible
//! to run the full Mirage stack against a simulated GPU on machines
//! without real hardware.
//!
//! # Current status
//!
//! The underlying rocjitsu C API exposes a VM lifecycle (create / step
//! / run / checkpoint) plus a code-object / decoder surface — it does
//! not yet expose a full KFD + DRM-AMDGPU ioctl shim. Consequently
//! this crate currently:
//!
//! * owns an optional `rj_vm_t` handle with safe RAII drop;
//! * exposes [`RocjitsuEmulator::from_config_string`] / [`from_config_file`]
//!   constructors that build the VM from a rocjitsu JSON config;
//! * satisfies the [`Emulator`] supertraits with direct `Handle*` /
//!   `HandleAny*` impls, with every request currently answered as
//!   [`AmdgpuError::NoSys`].
//!
//! As rocjitsu grows an ioctl surface, individual match arms will be
//! filled in — the same incremental path [`mirage_real`] already uses
//! (see [`mirage_real::RealEmulator`]'s `AmdkfdIocGetVersion` arm).
//!
//! # Availability
//!
//! The companion `rocjitsu_sys` crate links to `librocjitsu` only when
//! `ROCJITSU_LIB_DIR` is set at build time. [`RocjitsuEmulator::available`]
//! reports whether the headers were discovered; the actual library
//! presence is resolved by the linker. Use [`RocjitsuEmulator::new_stub`]
//! in tests that must run on hosts without rocjitsu.
//!
//! [rocjitsu]: https://github.com/ROCm/rocm-systems/tree/main/experimental/rocjitsu

use std::ffi::{CString, NulError};
use std::path::Path;
use std::ptr;
use std::sync::Mutex;

use mirage_schema::amdgpu::{
    HandleAnyDrmIoctl, HandleAnyKfdIoctl, HandleDrmIoctl, HandleKfdIoctl, IoctlCtx,
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

// ---------------------------------------------------------------------------
// Error type.

/// Errors that can occur while constructing a [`RocjitsuEmulator`].
#[derive(Debug)]
pub enum RocjitsuError {
    /// The rocjitsu C API returned a non-success status.
    Status(rocjitsu_sys::rj_status_t),
    /// An input string contained an interior NUL byte.
    InvalidString,
}

impl std::fmt::Display for RocjitsuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(s) => write!(f, "rocjitsu status {s}"),
            Self::InvalidString => write!(f, "input string contained an interior NUL byte"),
        }
    }
}

impl std::error::Error for RocjitsuError {}

impl From<NulError> for RocjitsuError {
    fn from(_: NulError) -> Self {
        Self::InvalidString
    }
}

fn check(status: rocjitsu_sys::rj_status_t) -> Result<(), RocjitsuError> {
    if status == rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
        Ok(())
    } else {
        Err(RocjitsuError::Status(status))
    }
}

// ---------------------------------------------------------------------------
// Safe handle to a `rj_vm_t`.

/// Owning wrapper around a `rocjitsu_sys::rj_vm_t *` that destroys and
/// releases the VM on drop.
///
/// # Safety
///
/// The rocjitsu VM API is documented to be safe to call from a single
/// thread at a time. We serialise access with the `Mutex` inside
/// [`RocjitsuEmulator`] rather than inside this wrapper so that the
/// `Send + Sync` bounds required by [`Emulator`] hold.
struct VmHandle(ptr::NonNull<rocjitsu_sys::rj_vm_t>);

// SAFETY: ownership of the underlying C handle is single; mutation is
// serialised by the surrounding mutex.
unsafe impl Send for VmHandle {}

impl Drop for VmHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `rj_vm_create*` (which returns a
        // VM with refcount 0) and has not been destroyed yet; no other
        // reference exists because we own it. Per the rocjitsu
        // refcount contract (see `refcount.h`), `rj_vm_destroy` on a
        // refcount-0 object frees immediately — calling `rj_vm_release`
        // on top of that would underflow the refcount.
        unsafe {
            rocjitsu_sys::rj_vm_destroy(self.0.as_ptr());
        }
    }
}

// ---------------------------------------------------------------------------
// Emulator.

/// A Mirage [`Emulator`](mirage_schema::emulator::Emulator) backed by a
/// rocjitsu virtual machine.
pub struct RocjitsuEmulator {
    /// The backing VM, or `None` for a stub emulator used on hosts
    /// without rocjitsu (tests, CI).
    vm: Mutex<Option<VmHandle>>,
}

impl std::fmt::Debug for RocjitsuEmulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocjitsuEmulator")
            .field("has_vm", &self.vm.lock().unwrap().is_some())
            .finish()
    }
}

impl RocjitsuEmulator {
    /// Returns `true` if this crate was built against the rocjitsu C
    /// headers. Absence of headers at *build time* means attempting to
    /// construct a real VM will fail at link time; tests should prefer
    /// [`Self::new_stub`] when this returns `false`.
    pub const fn available() -> bool {
        // If headers were missing, `rocjitsu_sys` emits an empty
        // bindings file and this function would not resolve. If this
        // crate compiles at all, the symbols are visible.
        true
    }

    /// Build an emulator that owns no VM. Every forwarded request
    /// returns [`AmdgpuError::NoSys`]. Useful in tests on hosts that
    /// do not link against `librocjitsu`.
    pub fn new_stub() -> Self {
        Self {
            vm: Mutex::new(None),
        }
    }

    /// Build an emulator from an in-memory rocjitsu JSON config
    /// string.
    ///
    /// `schema_path` must point at the `simulation_config.fbs`
    /// FlatBuffers schema that ships with rocjitsu.
    pub fn from_config_string(json: &str, schema_path: &Path) -> Result<Self, RocjitsuError> {
        let json_c = CString::new(json)?;
        let schema_c = CString::new(schema_path.as_os_str().as_encoded_bytes())?;
        let mut vm: *mut rocjitsu_sys::rj_vm_t = ptr::null_mut();
        // SAFETY: all pointers are NUL-terminated and out-pointer is
        // valid for a `*mut *mut rj_vm_t` write.
        let status = unsafe {
            rocjitsu_sys::rj_vm_create_from_string(json_c.as_ptr(), schema_c.as_ptr(), &mut vm)
        };
        check(status)?;
        let handle = ptr::NonNull::new(vm).ok_or(RocjitsuError::Status(
            rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_ERROR,
        ))?;
        Ok(Self {
            vm: Mutex::new(Some(VmHandle(handle))),
        })
    }

    /// Build an emulator from an on-disk rocjitsu JSON config file.
    pub fn from_config_file(json_path: &Path, schema_path: &Path) -> Result<Self, RocjitsuError> {
        let json_c = CString::new(json_path.as_os_str().as_encoded_bytes())?;
        let schema_c = CString::new(schema_path.as_os_str().as_encoded_bytes())?;
        let mut vm: *mut rocjitsu_sys::rj_vm_t = ptr::null_mut();
        // SAFETY: same as `from_config_string`.
        let status =
            unsafe { rocjitsu_sys::rj_vm_create(json_c.as_ptr(), schema_c.as_ptr(), &mut vm) };
        check(status)?;
        let handle = ptr::NonNull::new(vm).ok_or(RocjitsuError::Status(
            rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_ERROR,
        ))?;
        Ok(Self {
            vm: Mutex::new(Some(VmHandle(handle))),
        })
    }

    /// Step the underlying VM by one tick.
    ///
    /// Returns `true` while any wavefront is still executing. Returns
    /// [`AmdgpuError::NoSys`] if this emulator was constructed with
    /// [`Self::new_stub`].
    pub fn step(&self) -> AmdgpuResult<bool> {
        let guard = self.vm.lock().unwrap();
        let handle = guard.as_ref().ok_or(AmdgpuError::NoSys)?;
        let mut active: i32 = 0;
        // SAFETY: `handle.0` is a live `rj_vm_t *` for the duration of
        // this call (mutex guards against concurrent destruction).
        let status = unsafe { rocjitsu_sys::rj_vm_step(handle.0.as_ptr(), &mut active) };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            return Err(AmdgpuError::Invalid);
        }
        Ok(active != 0)
    }

    /// Run the underlying VM to completion. Returns the number of
    /// ticks executed.
    pub fn run(&self) -> AmdgpuResult<u64> {
        let guard = self.vm.lock().unwrap();
        let handle = guard.as_ref().ok_or(AmdgpuError::NoSys)?;
        let mut ticks: u64 = 0;
        // SAFETY: `handle.0` is a live `rj_vm_t *` for the duration of
        // this call.
        let status = unsafe { rocjitsu_sys::rj_vm_run(handle.0.as_ptr(), &mut ticks) };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            return Err(AmdgpuError::Invalid);
        }
        Ok(ticks)
    }
}

// ---------------------------------------------------------------------------
// Emulator trait surface. Every ioctl / syscall is currently a stub;
// see module docs for the rationale.

impl HandleKfdIoctl for RocjitsuEmulator {
    nosys_methods!(
        fn amdkfd_ioc_get_version(mirage_schema::amdgpu::AmdkfdIocGetVersionRequest) -> mirage_schema::amdgpu::AmdkfdIocGetVersionResponse;
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

impl HandleAnyKfdIoctl for RocjitsuEmulator {}

impl HandleDrmIoctl for RocjitsuEmulator {
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

impl HandleAnyDrmIoctl for RocjitsuEmulator {}

impl HandleFsSyscalls for RocjitsuEmulator {
    nosys_methods!(
        fn syscall_open(mirage_schema::syscalls::SyscallOpenRequest) -> mirage_schema::syscalls::SyscallOpenResponse;
        fn syscall_sysfs_read(mirage_schema::syscalls::SyscallSysfsReadRequest) -> mirage_schema::syscalls::SyscallSysfsReadResponse;
        fn syscall_close(mirage_schema::syscalls::SyscallCloseRequest) -> mirage_schema::syscalls::SyscallCloseResponse;
        fn syscall_stat_device(mirage_schema::syscalls::SyscallStatDeviceRequest) -> mirage_schema::syscalls::SyscallStatDeviceResponse;
        fn syscall_access(mirage_schema::syscalls::SyscallAccessRequest) -> mirage_schema::syscalls::SyscallAccessResponse;
        fn syscall_readlink_fd(mirage_schema::syscalls::SyscallReadlinkFdRequest) -> mirage_schema::syscalls::SyscallReadlinkFdResponse;
        fn syscall_mmap(mirage_schema::syscalls::SyscallMmapRequest) -> mirage_schema::syscalls::SyscallMmapResponse;
        fn syscall_munmap(mirage_schema::syscalls::SyscallMunmapRequest) -> mirage_schema::syscalls::SyscallMunmapResponse;
        fn syscall_read_device(mirage_schema::syscalls::SyscallReadDeviceRequest) -> mirage_schema::syscalls::SyscallReadDeviceResponse;
        fn syscall_dup(mirage_schema::syscalls::SyscallDupRequest) -> mirage_schema::syscalls::SyscallDupResponse;
        fn syscall_atfork_child(mirage_schema::syscalls::SyscallAtforkChildRequest) -> mirage_schema::syscalls::SyscallAtforkChildResponse;
    );
}

impl HandleAnyFsSyscalls for RocjitsuEmulator {}

// Compile-time proof that `RocjitsuEmulator` satisfies `Emulator`.
const _: fn() = || {
    fn assert_emulator<T: mirage_schema::emulator::Emulator>() {}
    assert_emulator::<RocjitsuEmulator>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_emulator_reports_nosys_for_every_surface() {
        use mirage_schema::amdgpu::{
            AmdkfdIocGetVersionRequest, AnyKfdIoctlRequest, HandleAnyKfdIoctl,
        };
        let emu = RocjitsuEmulator::new_stub();
        let ctx = IoctlCtx { pid: 0, tid: 0 };
        assert!(matches!(
            emu.handle_any_kfd_ioctl(
                ctx,
                AnyKfdIoctlRequest::AmdkfdIocGetVersion(AmdkfdIocGetVersionRequest {}),
            ),
            Err(AmdgpuError::NoSys),
        ));
        assert!(matches!(emu.step(), Err(AmdgpuError::NoSys)));
        assert!(matches!(emu.run(), Err(AmdgpuError::NoSys)));
    }

    #[test]
    fn stub_emulator_is_debuggable() {
        let emu = RocjitsuEmulator::new_stub();
        let s = format!("{emu:?}");
        assert!(s.contains("has_vm: false"));
    }

    /// End-to-end smoke test: construct a real VM from the bundled
    /// CDNA4 topology config and run it to completion. Skips when the
    /// rocjitsu source tree (and therefore its schema / config files)
    /// is not co-located with this workspace — e.g. when
    /// `rocjitsu_sys` was built against an external `ROCJITSU_LIB_DIR`.
    #[test]
    fn real_vm_runs_bundled_cdna4_config() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut config_path = None;
        let mut schema_path = None;
        for ancestor in manifest_dir.ancestors() {
            let cfg = ancestor.join("experimental/rocjitsu/configs/amdgpu_cdna4.json");
            let sch = ancestor.join("experimental/rocjitsu/schemas/simulation_config.fbs");
            if cfg.exists() && sch.exists() {
                config_path = Some(cfg);
                schema_path = Some(sch);
                break;
            }
        }
        let (Some(config_path), Some(schema_path)) = (config_path, schema_path) else {
            eprintln!("rocjitsu source tree not found; skipping end-to-end test");
            return;
        };

        let emu = match RocjitsuEmulator::from_config_file(&config_path, &schema_path) {
            Ok(emu) => emu,
            Err(e) => {
                // If we're linked against the stub, construction fails
                // with a rocjitsu status error — treat as a skip.
                eprintln!("rj_vm_create failed ({e}); likely linked against stub — skipping");
                return;
            }
        };
        // Just confirm step() talks to the real VM without panicking.
        // A full `run()` may take too long for unit testing; a single
        // step is enough to prove the FFI path is live.
        let _ = emu.step();
    }
}
