//! [`RocjitsuEmulator`] — an [`Emulator`](mirage_schema::emulator::Emulator)
//! implementation backed by the [rocjitsu] simulated kernel-mode driver.
//!
//! Where [`mirage_real`](../mirage_real) forwards every ioctl to a real
//! AMD GPU via `/dev/kfd`, `RocjitsuEmulator` forwards those same
//! requests to the rocjitsu `SimulatedDriver` through the `rj_kmd_*`
//! C API. This makes it possible to run the full Mirage stack against a
//! simulated GPU on machines without real hardware.
//!
//! # KFD ioctl surface
//!
//! Every [`HandleKfdIoctl`] method constructs the corresponding
//! `mirage_uapi::kfd` struct from the schema request, calls
//! [`rj_kmd_ioctl`](rocjitsu_sys::rj_kmd_ioctl), and extracts the
//! response fields — mirroring the pattern in `mirage_real`.
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
    AmdkfdIocAcquireVmRequest, AmdkfdIocAcquireVmResponse, AmdkfdIocAisOpRequest,
    AmdkfdIocAisOpResponse, AmdkfdIocAllocMemoryOfGpuRequest, AmdkfdIocAllocMemoryOfGpuResponse,
    AmdkfdIocAllocQueueGwsRequest, AmdkfdIocAllocQueueGwsResponse,
    AmdkfdIocAvailableMemoryRequest, AmdkfdIocAvailableMemoryResponse,
    AmdkfdIocCreateEventRequest, AmdkfdIocCreateEventResponse, AmdkfdIocCreateProcessRequest,
    AmdkfdIocCreateProcessResponse, AmdkfdIocCreateQueueRequest, AmdkfdIocCreateQueueResponse,
    AmdkfdIocCriuOpRequest, AmdkfdIocCriuOpResponse, AmdkfdIocCrossMemoryCopyRequest,
    AmdkfdIocCrossMemoryCopyResponse, AmdkfdIocDbgAddressWatchDeprecatedRequest,
    AmdkfdIocDbgAddressWatchDeprecatedResponse, AmdkfdIocDbgRegisterDeprecatedRequest,
    AmdkfdIocDbgRegisterDeprecatedResponse, AmdkfdIocDbgTrapRequest, AmdkfdIocDbgTrapResponse,
    AmdkfdIocDbgUnregisterDeprecatedRequest, AmdkfdIocDbgUnregisterDeprecatedResponse,
    AmdkfdIocDbgWaveControlDeprecatedRequest, AmdkfdIocDbgWaveControlDeprecatedResponse,
    AmdkfdIocDestroyEventRequest, AmdkfdIocDestroyEventResponse, AmdkfdIocDestroyQueueRequest,
    AmdkfdIocDestroyQueueResponse, AmdkfdIocExportDmabufRequest, AmdkfdIocExportDmabufResponse,
    AmdkfdIocFreeMemoryOfGpuRequest, AmdkfdIocFreeMemoryOfGpuResponse,
    AmdkfdIocGetClockCountersRequest, AmdkfdIocGetClockCountersResponse,
    AmdkfdIocGetDmabufInfoRequest, AmdkfdIocGetDmabufInfoResponse,
    AmdkfdIocGetProcessAperturesNewRequest, AmdkfdIocGetProcessAperturesNewResponse,
    AmdkfdIocGetProcessAperturesRequest, AmdkfdIocGetProcessAperturesResponse,
    AmdkfdIocGetQueueWaveStateRequest, AmdkfdIocGetQueueWaveStateResponse,
    AmdkfdIocGetTileConfigRequest, AmdkfdIocGetTileConfigResponse, AmdkfdIocGetVersionRequest,
    AmdkfdIocGetVersionResponse, AmdkfdIocImportDmabufRequest, AmdkfdIocImportDmabufResponse,
    AmdkfdIocIpcExportHandleRequest, AmdkfdIocIpcExportHandleResponse,
    AmdkfdIocIpcImportHandleRequest, AmdkfdIocIpcImportHandleResponse,
    AmdkfdIocMapMemoryToGpuRequest, AmdkfdIocMapMemoryToGpuResponse, AmdkfdIocPcSampleRequest,
    AmdkfdIocPcSampleResponse, AmdkfdIocProfilerRequest, AmdkfdIocProfilerResponse,
    AmdkfdIocResetEventRequest, AmdkfdIocResetEventResponse, AmdkfdIocRlcSpmRequest,
    AmdkfdIocRlcSpmResponse, AmdkfdIocRuntimeEnableRequest, AmdkfdIocRuntimeEnableResponse,
    AmdkfdIocSetCuMaskRequest, AmdkfdIocSetCuMaskResponse, AmdkfdIocSetEventRequest,
    AmdkfdIocSetEventResponse, AmdkfdIocSetMemoryPolicyRequest, AmdkfdIocSetMemoryPolicyResponse,
    AmdkfdIocSetScratchBackingVaRequest, AmdkfdIocSetScratchBackingVaResponse,
    AmdkfdIocSetTrapHandlerRequest, AmdkfdIocSetTrapHandlerResponse,
    AmdkfdIocSetXnackModeRequest, AmdkfdIocSetXnackModeResponse, AmdkfdIocSmiEventsRequest,
    AmdkfdIocSmiEventsResponse, AmdkfdIocSvmRequest, AmdkfdIocSvmResponse,
    AmdkfdIocUnmapMemoryFromGpuRequest, AmdkfdIocUnmapMemoryFromGpuResponse,
    AmdkfdIocUpdateQueueRequest, AmdkfdIocUpdateQueueResponse, AmdkfdIocWaitEventsRequest,
    AmdkfdIocWaitEventsResponse, HandleAnyDrmIoctl, HandleAnyKfdIoctl, HandleDrmIoctl,
    HandleKfdIoctl, IoctlCtx, KfdCriuBoBucket, KfdCriuDeviceBucket, KfdEventData,
    KfdPcSampleArgs, KfdPcSampleInfo, KfdProcessDeviceAperture,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_schema::syscalls::{HandleAnyFsSyscalls, HandleFsSyscalls};
use mirage_uapi::ioctl::{kfd_ior, kfd_iow, kfd_iowr, maybe_mut_ptr, IoctlCmd};
use mirage_uapi::kfd;
use mirage_uapi::kfd_marshal::{
    DbgTrapOwned, KfdCreateQueueArgsCompat, KfdCriuBoBucketRaw, KfdCriuDeviceBucketRaw,
    KfdEventDataRaw, KfdMemoryRangeRaw, KfdPcSampleInfoRaw, KfdProfilerOwned,
    KfdSetMemoryPolicyArgsCompat, KfdSvmOwned, ais_op_response_from_c, dbg_trap_from_c,
    dbg_trap_to_c, profiler_args_from_c, profiler_args_to_c,
};
use mirage_uapi::{FromC, ToC};

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
struct VmHandle(ptr::NonNull<rocjitsu_sys::rj_vm_t>);

unsafe impl Send for VmHandle {}

impl Drop for VmHandle {
    fn drop(&mut self) {
        unsafe {
            rocjitsu_sys::rj_vm_destroy(self.0.as_ptr());
        }
    }
}

// ---------------------------------------------------------------------------
// Safe handle to a `rj_kmd_t`.

/// Owning wrapper around a `rocjitsu_sys::rj_kmd_t *` that destroys the
/// simulated driver on drop.
struct KmdHandle(ptr::NonNull<rocjitsu_sys::rj_kmd_t>);

// SAFETY: the rj_kmd_t is thread-safe when protected by the surrounding
// mutex — the SimulatedDriver serialises access to the simulation engine.
unsafe impl Send for KmdHandle {}

impl Drop for KmdHandle {
    fn drop(&mut self) {
        unsafe {
            rocjitsu_sys::rj_kmd_destroy(self.0.as_ptr());
        }
    }
}

// ---------------------------------------------------------------------------
// Emulator.

/// A Mirage [`Emulator`](mirage_schema::emulator::Emulator) backed by
/// the rocjitsu simulated kernel-mode driver.
///
/// Construct via [`Self::from_default_kmd`] for the KFD ioctl path, or
/// via [`Self::from_config_string`] / [`Self::from_config_file`] for
/// the legacy VM step/run path.
pub struct RocjitsuEmulator {
    /// KMD driver handle for the KFD ioctl surface.
    kmd: Mutex<Option<KmdHandle>>,
    /// Legacy VM handle for step/run (standalone simulation).
    vm: Mutex<Option<VmHandle>>,
}

impl std::fmt::Debug for RocjitsuEmulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocjitsuEmulator")
            .field("has_kmd", &self.kmd.lock().unwrap().is_some())
            .field("has_vm", &self.vm.lock().unwrap().is_some())
            .finish()
    }
}

impl RocjitsuEmulator {
    /// Returns `true` if this crate was built against the rocjitsu C
    /// headers.
    pub const fn available() -> bool {
        true
    }

    /// Build an emulator that owns no driver. Every forwarded request
    /// returns [`AmdgpuError::NoSys`]. Useful in tests on hosts that
    /// do not link against `librocjitsu`.
    pub fn new_stub() -> Self {
        Self {
            kmd: Mutex::new(None),
            vm: Mutex::new(None),
        }
    }

    /// Build an emulator backed by the simulated KFD driver.
    ///
    /// Reads `RJ_CONFIG` / `RJ_SCHEMA` environment variables, creates
    /// the simulation engine, opens the simulated `/dev/kfd`, and is
    /// immediately ready to accept KFD ioctls.
    pub fn from_default_kmd() -> Result<Self, RocjitsuError> {
        let mut kmd: *mut rocjitsu_sys::rj_kmd_t = ptr::null_mut();
        let status = unsafe { rocjitsu_sys::rj_kmd_create_default(&mut kmd) };
        check(status)?;
        let handle = ptr::NonNull::new(kmd).ok_or(RocjitsuError::Status(
            rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_ERROR,
        ))?;

        // Open the simulated KFD device so ioctls can flow.
        let mut fd: i32 = -1;
        let status = unsafe { rocjitsu_sys::rj_kmd_open(handle.as_ptr(), &mut fd) };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            unsafe { rocjitsu_sys::rj_kmd_destroy(handle.as_ptr()) };
            return Err(RocjitsuError::Status(status));
        }

        Ok(Self {
            kmd: Mutex::new(Some(KmdHandle(handle))),
            vm: Mutex::new(None),
        })
    }

    /// Build an emulator from an in-memory rocjitsu JSON config
    /// string (legacy VM path — no KFD ioctl surface).
    pub fn from_config_string(json: &str, schema_path: &Path) -> Result<Self, RocjitsuError> {
        let json_c = CString::new(json)?;
        let schema_c = CString::new(schema_path.as_os_str().as_encoded_bytes())?;
        let mut vm: *mut rocjitsu_sys::rj_vm_t = ptr::null_mut();
        let status = unsafe {
            rocjitsu_sys::rj_vm_create_from_string(json_c.as_ptr(), schema_c.as_ptr(), &mut vm)
        };
        check(status)?;
        let handle = ptr::NonNull::new(vm).ok_or(RocjitsuError::Status(
            rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_ERROR,
        ))?;
        Ok(Self {
            kmd: Mutex::new(None),
            vm: Mutex::new(Some(VmHandle(handle))),
        })
    }

    /// Build an emulator from an on-disk rocjitsu JSON config file
    /// (legacy VM path — no KFD ioctl surface).
    pub fn from_config_file(json_path: &Path, schema_path: &Path) -> Result<Self, RocjitsuError> {
        let json_c = CString::new(json_path.as_os_str().as_encoded_bytes())?;
        let schema_c = CString::new(schema_path.as_os_str().as_encoded_bytes())?;
        let mut vm: *mut rocjitsu_sys::rj_vm_t = ptr::null_mut();
        let status =
            unsafe { rocjitsu_sys::rj_vm_create(json_c.as_ptr(), schema_c.as_ptr(), &mut vm) };
        check(status)?;
        let handle = ptr::NonNull::new(vm).ok_or(RocjitsuError::Status(
            rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_ERROR,
        ))?;
        Ok(Self {
            kmd: Mutex::new(None),
            vm: Mutex::new(Some(VmHandle(handle))),
        })
    }

    /// Step the underlying VM by one tick.
    pub fn step(&self) -> AmdgpuResult<bool> {
        let guard = self.vm.lock().unwrap();
        let handle = guard.as_ref().ok_or(AmdgpuError::NoSys)?;
        let mut active: i32 = 0;
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
        let status = unsafe { rocjitsu_sys::rj_vm_run(handle.0.as_ptr(), &mut ticks) };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            return Err(AmdgpuError::Invalid);
        }
        Ok(ticks)
    }

    /// Return the sysfs topology path from the simulated driver, if
    /// available.
    pub fn topology_path(&self) -> Option<String> {
        let guard = self.kmd.lock().unwrap();
        let handle = guard.as_ref()?;
        let ptr = unsafe { rocjitsu_sys::rj_kmd_topology_path(handle.0.as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        let cstr = unsafe { std::ffi::CStr::from_ptr(ptr) };
        cstr.to_str().ok().map(|s| s.to_owned())
    }
}

// ---------------------------------------------------------------------------
// KFD ioctl helper — mirrors `RealEmulator::kfd_ioctl` but dispatches
// through `rj_kmd_ioctl` instead of `libc::ioctl`.

impl RocjitsuEmulator {
    fn sim_ioctl<T>(&self, cmd: IoctlCmd<T>, arg: &mut T) -> AmdgpuResult<()> {
        let guard = self.kmd.lock().unwrap();
        let handle = guard.as_ref().ok_or(AmdgpuError::NoSys)?;
        let mut result: i32 = 0;
        let status = unsafe {
            rocjitsu_sys::rj_kmd_ioctl(
                handle.0.as_ptr(),
                cmd.nr() as _,
                arg as *mut T as *mut std::ffi::c_void,
                &mut result,
            )
        };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            return Err(AmdgpuError::NoSys);
        }
        if result != 0 {
            return Err(AmdgpuError::from_errno(-result));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// KFD ioctl implementations — each method mirrors mirage_real/src/kfd.rs.

impl HandleKfdIoctl for RocjitsuEmulator {
    fn amdkfd_ioc_get_version(
        &self,
        _ctx: IoctlCtx,
        _request: AmdkfdIocGetVersionRequest,
    ) -> AmdgpuResult<AmdkfdIocGetVersionResponse> {
        let mut args = kfd::kfd_ioctl_get_version_args::default();
        self.sim_ioctl(
            kfd_ior::<kfd::kfd_ioctl_get_version_args>(0x01),
            &mut args,
        )?;
        Ok(AmdkfdIocGetVersionResponse {
            major_version: args.major_version,
            minor_version: args.minor_version,
        })
    }

    fn amdkfd_ioc_create_queue(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCreateQueueRequest,
    ) -> AmdgpuResult<AmdkfdIocCreateQueueResponse> {
        let mut args = KfdCreateQueueArgsCompat {
            ring_base_address: request.ring_base_address,
            write_pointer_address: request.write_pointer_address,
            read_pointer_address: request.read_pointer_address,
            ring_size: request.ring_size,
            gpu_id: request.gpu_id,
            queue_type: request.queue_type as u32,
            queue_percentage: request.queue_percentage,
            queue_priority: request.queue_priority,
            eop_buffer_address: request.eop_buffer_address,
            eop_buffer_size: request.eop_buffer_size,
            ctx_save_restore_address: request.ctx_save_restore_address,
            ctx_save_restore_size: request.ctx_save_restore_size,
            ctl_stack_size: request.ctl_stack_size,
            sdma_engine_id: request.sdma_engine_id,
            ..Default::default()
        };
        self.sim_ioctl(kfd_iowr::<KfdCreateQueueArgsCompat>(0x02), &mut args)?;
        Ok(AmdkfdIocCreateQueueResponse {
            doorbell_offset: args.doorbell_offset,
            queue_id: args.queue_id,
        })
    }

    fn amdkfd_ioc_destroy_queue(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocDestroyQueueRequest,
    ) -> AmdgpuResult<AmdkfdIocDestroyQueueResponse> {
        let mut args = kfd::kfd_ioctl_destroy_queue_args {
            queue_id: request.queue_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iow::<kfd::kfd_ioctl_destroy_queue_args>(0x03),
            &mut args,
        )?;
        Ok(AmdkfdIocDestroyQueueResponse {})
    }

    fn amdkfd_ioc_set_memory_policy(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetMemoryPolicyRequest,
    ) -> AmdgpuResult<AmdkfdIocSetMemoryPolicyResponse> {
        let mut args = KfdSetMemoryPolicyArgsCompat {
            alternate_aperture_base: request.alternate_aperture_base,
            alternate_aperture_size: request.alternate_aperture_size,
            gpu_id: request.gpu_id,
            default_policy: request.default_policy as u32,
            alternate_policy: request.alternate_policy as u32,
            misc_process_flag: request.misc_process_flag,
            pad: 0,
        };
        self.sim_ioctl(kfd_iow::<KfdSetMemoryPolicyArgsCompat>(0x04), &mut args)?;
        Ok(AmdkfdIocSetMemoryPolicyResponse {})
    }

    fn amdkfd_ioc_get_clock_counters(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocGetClockCountersRequest,
    ) -> AmdgpuResult<AmdkfdIocGetClockCountersResponse> {
        let mut args = kfd::kfd_ioctl_get_clock_counters_args {
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_get_clock_counters_args>(0x05),
            &mut args,
        )?;
        Ok(AmdkfdIocGetClockCountersResponse {
            gpu_clock_counter: args.gpu_clock_counter,
            cpu_clock_counter: args.cpu_clock_counter,
            system_clock_counter: args.system_clock_counter,
            system_clock_freq: args.system_clock_freq,
        })
    }

    fn amdkfd_ioc_get_process_apertures(
        &self,
        _ctx: IoctlCtx,
        _request: AmdkfdIocGetProcessAperturesRequest,
    ) -> AmdgpuResult<AmdkfdIocGetProcessAperturesResponse> {
        let mut args = kfd::kfd_ioctl_get_process_apertures_args::default();
        self.sim_ioctl(
            kfd_ior::<kfd::kfd_ioctl_get_process_apertures_args>(0x06),
            &mut args,
        )?;
        let count = args.num_of_nodes.min(args.process_apertures.len() as u32) as usize;
        Ok(AmdkfdIocGetProcessAperturesResponse {
            apertures: args.process_apertures[..count]
                .iter()
                .copied()
                .map(KfdProcessDeviceAperture::from_c)
                .collect(),
        })
    }

    fn amdkfd_ioc_update_queue(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocUpdateQueueRequest,
    ) -> AmdgpuResult<AmdkfdIocUpdateQueueResponse> {
        let mut args = kfd::kfd_ioctl_update_queue_args {
            ring_base_address: request.ring_base_address,
            queue_id: request.queue_id,
            ring_size: request.ring_size,
            queue_percentage: request.queue_percentage,
            queue_priority: request.queue_priority,
        };
        self.sim_ioctl(kfd_iow::<kfd::kfd_ioctl_update_queue_args>(0x07), &mut args)?;
        Ok(AmdkfdIocUpdateQueueResponse {})
    }

    fn amdkfd_ioc_create_event(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCreateEventRequest,
    ) -> AmdgpuResult<AmdkfdIocCreateEventResponse> {
        let mut args = kfd::kfd_ioctl_create_event_args {
            event_type: request.event_type as u32,
            auto_reset: request.auto_reset,
            node_id: request.node_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_create_event_args>(0x08),
            &mut args,
        )?;
        Ok(AmdkfdIocCreateEventResponse {
            event_page_offset: args.event_page_offset,
            event_trigger_data: args.event_trigger_data,
            event_id: args.event_id,
            event_slot_index: args.event_slot_index,
        })
    }

    fn amdkfd_ioc_destroy_event(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocDestroyEventRequest,
    ) -> AmdgpuResult<AmdkfdIocDestroyEventResponse> {
        let mut args = kfd::kfd_ioctl_destroy_event_args {
            event_id: request.event_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iow::<kfd::kfd_ioctl_destroy_event_args>(0x09),
            &mut args,
        )?;
        Ok(AmdkfdIocDestroyEventResponse {})
    }

    fn amdkfd_ioc_set_event(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetEventRequest,
    ) -> AmdgpuResult<AmdkfdIocSetEventResponse> {
        let mut args = kfd::kfd_ioctl_set_event_args {
            event_id: request.event_id,
            ..Default::default()
        };
        self.sim_ioctl(kfd_iow::<kfd::kfd_ioctl_set_event_args>(0x0A), &mut args)?;
        Ok(AmdkfdIocSetEventResponse {})
    }

    fn amdkfd_ioc_reset_event(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocResetEventRequest,
    ) -> AmdgpuResult<AmdkfdIocResetEventResponse> {
        let mut args = kfd::kfd_ioctl_reset_event_args {
            event_id: request.event_id,
            ..Default::default()
        };
        self.sim_ioctl(kfd_iow::<kfd::kfd_ioctl_reset_event_args>(0x0B), &mut args)?;
        Ok(AmdkfdIocResetEventResponse {})
    }

    fn amdkfd_ioc_wait_events(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocWaitEventsRequest,
    ) -> AmdgpuResult<AmdkfdIocWaitEventsResponse> {
        let mut events: Vec<KfdEventDataRaw> = request
            .events
            .iter()
            .map(|event| event.to_c(&mut ()))
            .collect();
        let mut args = kfd::kfd_ioctl_wait_events_args {
            events_ptr: maybe_mut_ptr(&mut events),
            num_events: events.len() as u32,
            wait_for_all: request.wait_for_all as u32,
            timeout: request.timeout,
            ..Default::default()
        };
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_wait_events_args>(0x0C), &mut args)?;
        Ok(AmdkfdIocWaitEventsResponse {
            wait_result: args.wait_result,
            events: events.into_iter().map(KfdEventData::from_c).collect(),
        })
    }

    fn amdkfd_ioc_dbg_register_deprecated(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocDbgRegisterDeprecatedRequest,
    ) -> AmdgpuResult<AmdkfdIocDbgRegisterDeprecatedResponse> {
        let mut args = kfd::kfd_ioctl_dbg_register_args {
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_register_args>(0x0D), &mut args)?;
        Ok(AmdkfdIocDbgRegisterDeprecatedResponse {})
    }

    fn amdkfd_ioc_dbg_unregister_deprecated(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocDbgUnregisterDeprecatedRequest,
    ) -> AmdgpuResult<AmdkfdIocDbgUnregisterDeprecatedResponse> {
        let mut args = kfd::kfd_ioctl_dbg_unregister_args {
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iow::<kfd::kfd_ioctl_dbg_unregister_args>(0x0E),
            &mut args,
        )?;
        Ok(AmdkfdIocDbgUnregisterDeprecatedResponse {})
    }

    fn amdkfd_ioc_dbg_address_watch_deprecated(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocDbgAddressWatchDeprecatedRequest,
    ) -> AmdgpuResult<AmdkfdIocDbgAddressWatchDeprecatedResponse> {
        let mut content = request.content;
        let mut args = kfd::kfd_ioctl_dbg_address_watch_args {
            content_ptr: maybe_mut_ptr(&mut content),
            gpu_id: request.gpu_id,
            buf_size_in_bytes: content.len() as u32,
        };
        self.sim_ioctl(
            kfd_iow::<kfd::kfd_ioctl_dbg_address_watch_args>(0x0F),
            &mut args,
        )?;
        Ok(AmdkfdIocDbgAddressWatchDeprecatedResponse {})
    }

    fn amdkfd_ioc_dbg_wave_control_deprecated(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocDbgWaveControlDeprecatedRequest,
    ) -> AmdgpuResult<AmdkfdIocDbgWaveControlDeprecatedResponse> {
        let mut content = request.content;
        let mut args = kfd::kfd_ioctl_dbg_wave_control_args {
            content_ptr: maybe_mut_ptr(&mut content),
            gpu_id: request.gpu_id,
            buf_size_in_bytes: content.len() as u32,
        };
        self.sim_ioctl(
            kfd_iow::<kfd::kfd_ioctl_dbg_wave_control_args>(0x10),
            &mut args,
        )?;
        Ok(AmdkfdIocDbgWaveControlDeprecatedResponse {})
    }

    fn amdkfd_ioc_set_scratch_backing_va(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetScratchBackingVaRequest,
    ) -> AmdgpuResult<AmdkfdIocSetScratchBackingVaResponse> {
        let mut args = kfd::kfd_ioctl_set_scratch_backing_va_args {
            va_addr: request.va_addr,
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_set_scratch_backing_va_args>(0x11),
            &mut args,
        )?;
        Ok(AmdkfdIocSetScratchBackingVaResponse {})
    }

    fn amdkfd_ioc_get_tile_config(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocGetTileConfigRequest,
    ) -> AmdgpuResult<AmdkfdIocGetTileConfigResponse> {
        let mut tile_config = vec![0u32; request.max_tile_configs as usize];
        let mut macro_tile_config = vec![0u32; request.max_macro_tile_configs as usize];
        let mut args = kfd::kfd_ioctl_get_tile_config_args {
            tile_config_ptr: maybe_mut_ptr(&mut tile_config),
            macro_tile_config_ptr: maybe_mut_ptr(&mut macro_tile_config),
            num_tile_configs: tile_config.len() as u32,
            num_macro_tile_configs: macro_tile_config.len() as u32,
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_get_tile_config_args>(0x12),
            &mut args,
        )?;
        tile_config.truncate(args.num_tile_configs as usize);
        macro_tile_config.truncate(args.num_macro_tile_configs as usize);
        Ok(AmdkfdIocGetTileConfigResponse {
            tile_config,
            macro_tile_config,
            gb_addr_config: args.gb_addr_config,
            num_banks: args.num_banks,
            num_ranks: args.num_ranks,
        })
    }

    fn amdkfd_ioc_set_trap_handler(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetTrapHandlerRequest,
    ) -> AmdgpuResult<AmdkfdIocSetTrapHandlerResponse> {
        let mut args = kfd::kfd_ioctl_set_trap_handler_args {
            tba_addr: request.tba_addr,
            tma_addr: request.tma_addr,
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iow::<kfd::kfd_ioctl_set_trap_handler_args>(0x13),
            &mut args,
        )?;
        Ok(AmdkfdIocSetTrapHandlerResponse {})
    }

    fn amdkfd_ioc_get_process_apertures_new(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocGetProcessAperturesNewRequest,
    ) -> AmdgpuResult<AmdkfdIocGetProcessAperturesNewResponse> {
        let mut apertures =
            vec![kfd::kfd_process_device_apertures::default(); request.max_nodes as usize];
        let mut args = kfd::kfd_ioctl_get_process_apertures_new_args {
            kfd_process_device_apertures_ptr: maybe_mut_ptr(&mut apertures),
            num_of_nodes: apertures.len() as u32,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_get_process_apertures_new_args>(0x14),
            &mut args,
        )?;
        apertures.truncate(args.num_of_nodes as usize);
        Ok(AmdkfdIocGetProcessAperturesNewResponse {
            apertures: apertures
                .into_iter()
                .map(KfdProcessDeviceAperture::from_c)
                .collect(),
        })
    }

    fn amdkfd_ioc_acquire_vm(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocAcquireVmRequest,
    ) -> AmdgpuResult<AmdkfdIocAcquireVmResponse> {
        // The simulated driver does not use the drm_fd; pass 0.
        let mut args = kfd::kfd_ioctl_acquire_vm_args {
            drm_fd: 0,
            gpu_id: request.gpu_id,
        };
        self.sim_ioctl(kfd_iow::<kfd::kfd_ioctl_acquire_vm_args>(0x15), &mut args)?;
        Ok(AmdkfdIocAcquireVmResponse {})
    }

    fn amdkfd_ioc_alloc_memory_of_gpu(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocAllocMemoryOfGpuRequest,
    ) -> AmdgpuResult<AmdkfdIocAllocMemoryOfGpuResponse> {
        let mut args = kfd::kfd_ioctl_alloc_memory_of_gpu_args {
            va_addr: request.va_addr,
            size: request.size,
            mmap_offset: request.mmap_offset,
            gpu_id: request.gpu_id,
            flags: request.flags,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_alloc_memory_of_gpu_args>(0x16),
            &mut args,
        )?;
        Ok(AmdkfdIocAllocMemoryOfGpuResponse {
            handle: args.handle,
            mmap_offset: args.mmap_offset,
            va_addr: args.va_addr,
        })
    }

    fn amdkfd_ioc_free_memory_of_gpu(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocFreeMemoryOfGpuRequest,
    ) -> AmdgpuResult<AmdkfdIocFreeMemoryOfGpuResponse> {
        let mut args = kfd::kfd_ioctl_free_memory_of_gpu_args {
            handle: request.handle,
        };
        self.sim_ioctl(
            kfd_iow::<kfd::kfd_ioctl_free_memory_of_gpu_args>(0x17),
            &mut args,
        )?;
        Ok(AmdkfdIocFreeMemoryOfGpuResponse {})
    }

    fn amdkfd_ioc_map_memory_to_gpu(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocMapMemoryToGpuRequest,
    ) -> AmdgpuResult<AmdkfdIocMapMemoryToGpuResponse> {
        let mut device_ids = request.device_ids;
        let mut args = kfd::kfd_ioctl_map_memory_to_gpu_args {
            handle: request.handle,
            device_ids_array_ptr: maybe_mut_ptr(&mut device_ids),
            n_devices: device_ids.len() as u32,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_map_memory_to_gpu_args>(0x18),
            &mut args,
        )?;
        Ok(AmdkfdIocMapMemoryToGpuResponse {
            n_success: args.n_success,
        })
    }

    fn amdkfd_ioc_unmap_memory_from_gpu(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocUnmapMemoryFromGpuRequest,
    ) -> AmdgpuResult<AmdkfdIocUnmapMemoryFromGpuResponse> {
        let mut device_ids = request.device_ids;
        let mut args = kfd::kfd_ioctl_unmap_memory_from_gpu_args {
            handle: request.handle,
            device_ids_array_ptr: maybe_mut_ptr(&mut device_ids),
            n_devices: device_ids.len() as u32,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_unmap_memory_from_gpu_args>(0x19),
            &mut args,
        )?;
        Ok(AmdkfdIocUnmapMemoryFromGpuResponse {
            n_success: args.n_success,
        })
    }

    fn amdkfd_ioc_set_cu_mask(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetCuMaskRequest,
    ) -> AmdgpuResult<AmdkfdIocSetCuMaskResponse> {
        let mut cu_mask = request.cu_mask;
        let mut args = kfd::kfd_ioctl_set_cu_mask_args {
            queue_id: request.queue_id,
            num_cu_mask: request.num_cu_mask,
            cu_mask_ptr: maybe_mut_ptr(&mut cu_mask),
        };
        self.sim_ioctl(kfd_iow::<kfd::kfd_ioctl_set_cu_mask_args>(0x1A), &mut args)?;
        Ok(AmdkfdIocSetCuMaskResponse {})
    }

    fn amdkfd_ioc_get_queue_wave_state(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocGetQueueWaveStateRequest,
    ) -> AmdgpuResult<AmdkfdIocGetQueueWaveStateResponse> {
        let mut args = kfd::kfd_ioctl_get_queue_wave_state_args {
            ctl_stack_address: request.ctl_stack_address,
            queue_id: request.queue_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_get_queue_wave_state_args>(0x1B),
            &mut args,
        )?;
        Ok(AmdkfdIocGetQueueWaveStateResponse {
            ctl_stack_used_size: args.ctl_stack_used_size,
            save_area_used_size: args.save_area_used_size,
        })
    }

    fn amdkfd_ioc_get_dmabuf_info(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocGetDmabufInfoRequest,
    ) -> AmdgpuResult<AmdkfdIocGetDmabufInfoResponse> {
        let mut metadata = vec![0u8; request.metadata_size as usize];
        let mut args = kfd::kfd_ioctl_get_dmabuf_info_args {
            metadata_ptr: maybe_mut_ptr(&mut metadata),
            metadata_size: metadata.len() as u32,
            dmabuf_fd: request.dmabuf_fd,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_get_dmabuf_info_args>(0x1C),
            &mut args,
        )?;
        metadata.truncate(args.metadata_size as usize);
        Ok(AmdkfdIocGetDmabufInfoResponse {
            size: args.size,
            metadata,
            gpu_id: args.gpu_id,
            flags: args.flags,
        })
    }

    fn amdkfd_ioc_import_dmabuf(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocImportDmabufRequest,
    ) -> AmdgpuResult<AmdkfdIocImportDmabufResponse> {
        let mut args = kfd::kfd_ioctl_import_dmabuf_args {
            va_addr: request.va_addr,
            gpu_id: request.gpu_id,
            dmabuf_fd: request.dmabuf_fd,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_import_dmabuf_args>(0x1D),
            &mut args,
        )?;
        Ok(AmdkfdIocImportDmabufResponse {
            handle: args.handle,
        })
    }

    fn amdkfd_ioc_alloc_queue_gws(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocAllocQueueGwsRequest,
    ) -> AmdgpuResult<AmdkfdIocAllocQueueGwsResponse> {
        let mut args = kfd::kfd_ioctl_alloc_queue_gws_args {
            queue_id: request.queue_id,
            num_gws: request.num_gws,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_alloc_queue_gws_args>(0x1E),
            &mut args,
        )?;
        Ok(AmdkfdIocAllocQueueGwsResponse {
            first_gws: args.first_gws,
        })
    }

    fn amdkfd_ioc_smi_events(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSmiEventsRequest,
    ) -> AmdgpuResult<AmdkfdIocSmiEventsResponse> {
        let mut args = kfd::kfd_ioctl_smi_events_args {
            gpuid: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_smi_events_args>(0x1F), &mut args)?;
        Ok(AmdkfdIocSmiEventsResponse {
            anon_fd: args.anon_fd,
        })
    }

    fn amdkfd_ioc_svm(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSvmRequest,
    ) -> AmdgpuResult<AmdkfdIocSvmResponse> {
        let mut owned = KfdSvmOwned::from_request(&request);
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_svm_args>(0x20), owned.raw_mut())?;
        Ok(owned.to_response())
    }

    fn amdkfd_ioc_set_xnack_mode(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetXnackModeRequest,
    ) -> AmdgpuResult<AmdkfdIocSetXnackModeResponse> {
        let mut args = kfd::kfd_ioctl_set_xnack_mode_args {
            xnack_enabled: request.xnack_enabled,
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_set_xnack_mode_args>(0x21),
            &mut args,
        )?;
        Ok(AmdkfdIocSetXnackModeResponse {
            xnack_enabled: args.xnack_enabled,
        })
    }

    fn amdkfd_ioc_criu_op(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCriuOpRequest,
    ) -> AmdgpuResult<AmdkfdIocCriuOpResponse> {
        let mut devices: Vec<KfdCriuDeviceBucketRaw> = request
            .devices
            .iter()
            .map(|device| device.to_c(&mut ()))
            .collect();
        let mut bos: Vec<KfdCriuBoBucketRaw> =
            request.bos.iter().map(|bo| bo.to_c(&mut ())).collect();
        let mut priv_data = request.priv_data;
        let mut args = kfd::kfd_ioctl_criu_args {
            devices: maybe_mut_ptr(&mut devices),
            bos: maybe_mut_ptr(&mut bos),
            priv_data: maybe_mut_ptr(&mut priv_data),
            priv_data_size: priv_data.len() as u64,
            num_devices: request.num_devices,
            num_bos: request.num_bos,
            num_objects: request.num_objects,
            pid: request.pid,
            op: request.op as u32,
        };
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_criu_args>(0x22), &mut args)?;
        devices.truncate(args.num_devices as usize);
        bos.truncate(args.num_bos as usize);
        priv_data.truncate(args.priv_data_size as usize);
        Ok(AmdkfdIocCriuOpResponse {
            num_devices: args.num_devices,
            num_bos: args.num_bos,
            num_objects: args.num_objects,
            priv_data_size: args.priv_data_size,
            pid: args.pid,
            devices: devices
                .into_iter()
                .map(KfdCriuDeviceBucket::from_c)
                .collect(),
            bos: bos.into_iter().map(KfdCriuBoBucket::from_c).collect(),
            priv_data,
        })
    }

    fn amdkfd_ioc_available_memory(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocAvailableMemoryRequest,
    ) -> AmdgpuResult<AmdkfdIocAvailableMemoryResponse> {
        let mut args = kfd::kfd_ioctl_get_available_memory_args {
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_get_available_memory_args>(0x23),
            &mut args,
        )?;
        Ok(AmdkfdIocAvailableMemoryResponse {
            available: args.available,
        })
    }

    fn amdkfd_ioc_export_dmabuf(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocExportDmabufRequest,
    ) -> AmdgpuResult<AmdkfdIocExportDmabufResponse> {
        let mut args = kfd::kfd_ioctl_export_dmabuf_args {
            handle: request.handle,
            flags: request.flags,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_export_dmabuf_args>(0x24),
            &mut args,
        )?;
        Ok(AmdkfdIocExportDmabufResponse {
            dmabuf_fd: args.dmabuf_fd,
        })
    }

    fn amdkfd_ioc_runtime_enable(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocRuntimeEnableRequest,
    ) -> AmdgpuResult<AmdkfdIocRuntimeEnableResponse> {
        let mut args = kfd::kfd_ioctl_runtime_enable_args {
            r_debug: 0,
            mode_mask: request.flags as u32,
            capabilities_mask: (request.flags >> 32) as u32,
        };
        match self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_runtime_enable_args>(0x25),
            &mut args,
        ) {
            Ok(()) => Ok(AmdkfdIocRuntimeEnableResponse {}),
            Err(AmdgpuError::Busy) => Ok(AmdkfdIocRuntimeEnableResponse {}),
            Err(e) => Err(e),
        }
    }

    fn amdkfd_ioc_dbg_trap(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocDbgTrapRequest,
    ) -> AmdgpuResult<AmdkfdIocDbgTrapResponse> {
        let mut owned = DbgTrapOwned::default();
        let mut args = dbg_trap_to_c(&request, &mut owned);
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_dbg_trap_args>(0x26), &mut args)?;
        Ok(AmdkfdIocDbgTrapResponse {
            args: dbg_trap_from_c(request.op, &args, &owned),
        })
    }

    fn amdkfd_ioc_create_process(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCreateProcessRequest,
    ) -> AmdgpuResult<AmdkfdIocCreateProcessResponse> {
        let mut args = kfd::kfd_ioctl_create_process_args {
            flags: request.flags,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_create_process_args>(0x27),
            &mut args,
        )?;
        Ok(AmdkfdIocCreateProcessResponse {})
    }

    fn amdkfd_ioc_ipc_import_handle(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocIpcImportHandleRequest,
    ) -> AmdgpuResult<AmdkfdIocIpcImportHandleResponse> {
        let mut args = kfd::kfd_ioctl_ipc_import_handle_args {
            va_addr: request.va_addr,
            share_handle: request.share_handle,
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_ipc_import_handle_args>(0x80),
            &mut args,
        )?;
        Ok(AmdkfdIocIpcImportHandleResponse {
            handle: args.handle,
            mmap_offset: args.mmap_offset,
            flags: args.flags,
        })
    }

    fn amdkfd_ioc_ipc_export_handle(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocIpcExportHandleRequest,
    ) -> AmdgpuResult<AmdkfdIocIpcExportHandleResponse> {
        let mut args = kfd::kfd_ioctl_ipc_export_handle_args {
            handle: request.handle,
            gpu_id: request.gpu_id,
            flags: request.flags,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_ipc_export_handle_args>(0x81),
            &mut args,
        )?;
        Ok(AmdkfdIocIpcExportHandleResponse {
            share_handle: args.share_handle,
        })
    }

    fn amdkfd_ioc_cross_memory_copy(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCrossMemoryCopyRequest,
    ) -> AmdgpuResult<AmdkfdIocCrossMemoryCopyResponse> {
        let mut src: Vec<KfdMemoryRangeRaw> = request
            .src_mem_range_array
            .iter()
            .map(|range| range.to_c(&mut ()))
            .collect();
        let mut dst: Vec<KfdMemoryRangeRaw> = request
            .dst_mem_range_array
            .iter()
            .map(|range| range.to_c(&mut ()))
            .collect();
        let mut args = kfd::kfd_ioctl_cross_memory_copy_args {
            pid: request.pid,
            flags: request.flags,
            src_mem_range_array: maybe_mut_ptr(&mut src),
            src_mem_array_size: src.len() as u64,
            dst_mem_range_array: maybe_mut_ptr(&mut dst),
            dst_mem_array_size: dst.len() as u64,
            ..Default::default()
        };
        self.sim_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_cross_memory_copy_args>(0x83),
            &mut args,
        )?;
        Ok(AmdkfdIocCrossMemoryCopyResponse {
            bytes_copied: args.bytes_copied,
        })
    }

    fn amdkfd_ioc_rlc_spm(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocRlcSpmRequest,
    ) -> AmdgpuResult<AmdkfdIocRlcSpmResponse> {
        let mut args = kfd::kfd_ioctl_spm_args {
            dest_buf: request.dest_buf,
            buf_size: request.buf_size,
            op: request.op as u32,
            timeout: request.timeout,
            gpu_id: request.gpu_id,
            ..Default::default()
        };
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_spm_args>(0x84), &mut args)?;
        Ok(AmdkfdIocRlcSpmResponse {
            timeout: args.timeout,
            bytes_copied: args.bytes_copied,
            has_data_loss: args.has_data_loss,
        })
    }

    fn amdkfd_ioc_pc_sample(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocPcSampleRequest,
    ) -> AmdgpuResult<AmdkfdIocPcSampleResponse> {
        let mut sample_info: Vec<KfdPcSampleInfoRaw> = request
            .args
            .sample_info
            .iter()
            .map(|info| info.to_c(&mut ()))
            .collect();
        let mut args = kfd::kfd_ioctl_pc_sample_args {
            sample_info_ptr: maybe_mut_ptr(&mut sample_info),
            num_sample_info: request.args.num_sample_info,
            op: request.args.op as u32,
            gpu_id: request.args.gpu_id,
            trace_id: request.args.trace_id,
            flags: request.args.flags,
            reserved: request.args.reserved,
        };
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_pc_sample_args>(0x85), &mut args)?;
        sample_info.truncate(args.num_sample_info as usize);
        Ok(AmdkfdIocPcSampleResponse {
            args: KfdPcSampleArgs {
                sample_info: sample_info
                    .into_iter()
                    .map(KfdPcSampleInfo::from_c)
                    .collect(),
                num_sample_info: args.num_sample_info,
                op: request.args.op,
                gpu_id: args.gpu_id,
                trace_id: args.trace_id,
                flags: args.flags,
                reserved: args.reserved,
            },
        })
    }

    fn amdkfd_ioc_profiler(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocProfilerRequest,
    ) -> AmdgpuResult<AmdkfdIocProfilerResponse> {
        let mut owned = KfdProfilerOwned::default();
        let mut args = kfd::kfd_ioctl_profiler_args {
            op: request.op as u32,
            ..Default::default()
        };
        args.__bindgen_anon_1 = profiler_args_to_c(&request.args, &mut owned);
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_profiler_args>(0x86), &mut args)?;
        Ok(AmdkfdIocProfilerResponse {
            args: profiler_args_from_c(request.op, &args, &owned),
        })
    }

    fn amdkfd_ioc_ais_op(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocAisOpRequest,
    ) -> AmdgpuResult<AmdkfdIocAisOpResponse> {
        let mut args = kfd::kfd_ioctl_ais_args {
            __bindgen_anon_1: kfd::kfd_ioctl_ais_args__bindgen_ty_1 {
                in_: kfd::kfd_ais_in_args {
                    handle: request.handle,
                    handle_offset: request.handle_offset,
                    file_offset: request.file_offset,
                    size: request.size,
                    op: request.op as u32,
                    fd: request.fd,
                },
            },
        };
        self.sim_ioctl(kfd_iowr::<kfd::kfd_ioctl_ais_args>(0x87), &mut args)?;
        Ok(ais_op_response_from_c(&args))
    }
}

impl HandleAnyKfdIoctl for RocjitsuEmulator {}

// ---------------------------------------------------------------------------
// DRM ioctls — the simulated driver is KFD-only; DRM stays NoSys.

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

// ---------------------------------------------------------------------------
// FS syscalls — mmap/munmap go through the simulated driver; rest is NoSys.

impl HandleFsSyscalls for RocjitsuEmulator {
    fn syscall_mmap(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallMmapRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallMmapResponse> {
        let guard = self.kmd.lock().unwrap();
        let handle = guard.as_ref().ok_or(AmdgpuError::NoSys)?;
        let mut result: *mut std::ffi::c_void = std::ptr::null_mut();
        let status = unsafe {
            rocjitsu_sys::rj_kmd_mmap(
                handle.0.as_ptr(),
                request.addr_hint as usize as *mut std::ffi::c_void,
                request.length as usize,
                request.prot as i32,
                request.flags as i32,
                request.offset as i64,
                &mut result as *mut *mut std::ffi::c_void,
            )
        };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            return Err(AmdgpuError::NoSys);
        }
        if result == libc::MAP_FAILED {
            return Err(AmdgpuError::NoMemory);
        }
        Ok(mirage_schema::syscalls::SyscallMmapResponse {
            server_addr: result as usize as u64,
            mapping_id: result as usize as u64,
        })
    }

    fn syscall_munmap(
        &self,
        _ctx: IoctlCtx,
        request: mirage_schema::syscalls::SyscallMunmapRequest,
    ) -> AmdgpuResult<mirage_schema::syscalls::SyscallMunmapResponse> {
        let guard = self.kmd.lock().unwrap();
        let handle = guard.as_ref().ok_or(AmdgpuError::NoSys)?;
        let mut result: i32 = 0;
        let status = unsafe {
            rocjitsu_sys::rj_kmd_munmap(
                handle.0.as_ptr(),
                request.addr as usize as *mut std::ffi::c_void,
                request.length as usize,
                &mut result,
            )
        };
        if status != rocjitsu_sys::rj_status_e_ROCJITSU_STATUS_SUCCESS {
            return Err(AmdgpuError::NoSys);
        }
        if result != 0 {
            return Err(AmdgpuError::from_errno(-result));
        }
        Ok(mirage_schema::syscalls::SyscallMunmapResponse {})
    }

    nosys_methods!(
        fn syscall_open(mirage_schema::syscalls::SyscallOpenRequest) -> mirage_schema::syscalls::SyscallOpenResponse;
        fn syscall_sysfs_read(mirage_schema::syscalls::SyscallSysfsReadRequest) -> mirage_schema::syscalls::SyscallSysfsReadResponse;
        fn syscall_close(mirage_schema::syscalls::SyscallCloseRequest) -> mirage_schema::syscalls::SyscallCloseResponse;
        fn syscall_stat_device(mirage_schema::syscalls::SyscallStatDeviceRequest) -> mirage_schema::syscalls::SyscallStatDeviceResponse;
        fn syscall_access(mirage_schema::syscalls::SyscallAccessRequest) -> mirage_schema::syscalls::SyscallAccessResponse;
        fn syscall_readlink_fd(mirage_schema::syscalls::SyscallReadlinkFdRequest) -> mirage_schema::syscalls::SyscallReadlinkFdResponse;
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
        assert!(s.contains("has_kmd: false"));
        assert!(s.contains("has_vm: false"));
    }

    /// End-to-end smoke test: construct a real VM from the bundled
    /// CDNA4 topology config and run it to completion.
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
                eprintln!("rj_vm_create failed ({e}); likely linked against stub — skipping");
                return;
            }
        };
        let _ = emu.step();
    }
}
