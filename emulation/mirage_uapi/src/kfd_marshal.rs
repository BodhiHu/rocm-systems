use std::mem::size_of;

use bytes::BytesMut;

use crate::amdgpu::{
    AmdkfdIocAisOpResponse, AmdkfdIocDbgTrapRequest, AmdkfdIocSvmRequest, AmdkfdIocSvmResponse,
    KfdCriuBoBucket, KfdCriuDeviceBucket, KfdDbgDeviceInfoEntry, KfdDbgTrapAddressWatchMode,
    KfdDbgTrapArgs, KfdDbgTrapOp, KfdDbgTrapOverrideMode, KfdDbgTrapWaveLaunchMode, KfdEventData,
    KfdHwExceptionData, KfdMemoryExceptionData, KfdMemoryExceptionFailure, KfdMemoryRange,
    KfdPcSampleArgs, KfdPcSampleInfo, KfdPcSampleMethod, KfdPcSampleOp, KfdPcSampleType,
    KfdPmcSettings, KfdProcessDeviceAperture, KfdProfilerArgs, KfdProfilerOp,
    KfdQueueSnapshotEntry, KfdSignalEventData, KfdSvmAttribute,
};

use crate::ioctl::{maybe_mut_ptr, maybe_ptr};
use crate::kfd;
use crate::{FromC, ToC};

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdCreateQueueArgsCompat {
    pub ring_base_address: u64,
    pub write_pointer_address: u64,
    pub read_pointer_address: u64,
    pub doorbell_offset: u64,
    pub ring_size: u32,
    pub gpu_id: u32,
    pub queue_type: u32,
    pub queue_percentage: u32,
    pub queue_priority: u32,
    pub queue_id: u32,
    pub eop_buffer_address: u64,
    pub eop_buffer_size: u64,
    pub ctx_save_restore_address: u64,
    pub ctx_save_restore_size: u32,
    pub ctl_stack_size: u32,
    pub sdma_engine_id: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdSetMemoryPolicyArgsCompat {
    pub alternate_aperture_base: u64,
    pub alternate_aperture_size: u64,
    pub gpu_id: u32,
    pub default_policy: u32,
    pub alternate_policy: u32,
    pub misc_process_flag: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdMemoryExceptionFailureRaw {
    pub not_present: u32,
    pub read_only: u32,
    pub no_execute: u32,
    pub imprecise: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdMemoryExceptionDataRaw {
    pub failure: KfdMemoryExceptionFailureRaw,
    pub va: u64,
    pub gpu_id: u32,
    pub error_type: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdHwExceptionDataRaw {
    pub reset_type: u32,
    pub reset_cause: u32,
    pub memory_lost: u32,
    pub gpu_id: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdSignalEventDataRaw {
    pub last_event_age: u64,
}

#[repr(C)]
pub union KfdEventUnionRaw {
    pub memory_exception_data: KfdMemoryExceptionDataRaw,
    pub hw_exception_data: KfdHwExceptionDataRaw,
    pub signal_event_data: KfdSignalEventDataRaw,
}

impl Default for KfdEventUnionRaw {
    fn default() -> Self {
        Self {
            signal_event_data: KfdSignalEventDataRaw::default(),
        }
    }
}

#[repr(C)]
#[derive(Default)]
pub struct KfdEventDataRaw {
    pub payload: KfdEventUnionRaw,
    pub kfd_event_data_ext: u64,
    pub event_id: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdQueueSnapshotEntryRaw {
    pub exception_status: u64,
    pub ring_base_address: u64,
    pub write_pointer_address: u64,
    pub read_pointer_address: u64,
    pub ctx_save_restore_address: u64,
    pub queue_id: u32,
    pub gpu_id: u32,
    pub ring_size: u32,
    pub queue_type: u32,
    pub ctx_save_restore_area_size: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdDbgDeviceInfoEntryRaw {
    pub exception_status: u64,
    pub lds_base: u64,
    pub lds_limit: u64,
    pub scratch_base: u64,
    pub scratch_limit: u64,
    pub gpuvm_base: u64,
    pub gpuvm_limit: u64,
    pub gpu_id: u32,
    pub location_id: u32,
    pub vendor_id: u32,
    pub device_id: u32,
    pub revision_id: u32,
    pub subsystem_vendor_id: u32,
    pub subsystem_device_id: u32,
    pub fw_version: u32,
    pub gfx_target_version: u32,
    pub simd_count: u32,
    pub max_waves_per_simd: u32,
    pub array_count: u32,
    pub simd_arrays_per_engine: u32,
    pub num_xcc: u32,
    pub capability: u32,
    pub debug_prop: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdCriuDeviceBucketRaw {
    pub user_gpu_id: u32,
    pub actual_gpu_id: u32,
    pub drm_fd: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdCriuBoBucketRaw {
    pub addr: u64,
    pub size: u64,
    pub offset: u64,
    pub restored_offset: u64,
    pub gpu_id: u32,
    pub alloc_flags: u32,
    pub dmabuf_fd: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdMemoryRangeRaw {
    pub va_addr: u64,
    pub size: u64,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct KfdPcSampleInfoRaw {
    pub interval: u64,
    pub interval_min: u64,
    pub interval_max: u64,
    pub flags: u64,
    pub method: u32,
    pub type_: u32,
}

#[derive(Default)]
pub struct DbgTrapOwned {
    pub queue_ids: Vec<u32>,
    pub info: Vec<u8>,
    pub queue_snapshot: Vec<KfdQueueSnapshotEntryRaw>,
    pub device_snapshot: Vec<KfdDbgDeviceInfoEntryRaw>,
}

#[derive(Default)]
pub struct KfdProfilerOwned {
    pub sample_info: Vec<KfdPcSampleInfoRaw>,
}

pub struct KfdSvmOwned {
    storage: BytesMut,
    attr_count: usize,
}

impl KfdSvmOwned {
    pub fn from_request(request: &AmdkfdIocSvmRequest) -> Self {
        let attrs: Vec<kfd::kfd_ioctl_svm_attribute> = request
            .attrs
            .iter()
            .map(|attr| kfd::kfd_ioctl_svm_attribute {
                type_: attr.attr_type,
                value: attr.value,
            })
            .collect();
        let base_len = size_of::<kfd::kfd_ioctl_svm_args>();
        let total_len = base_len + attrs.len() * size_of::<kfd::kfd_ioctl_svm_attribute>();
        let mut storage = BytesMut::with_capacity(total_len);
        storage.resize(total_len, 0);
        let args = storage.as_mut_ptr() as *mut kfd::kfd_ioctl_svm_args;
        unsafe {
            (*args).start_addr = request.start_addr;
            (*args).size = request.size;
            (*args).op = request.op as u32;
            (*args).nattr = attrs.len() as u32;
            std::ptr::copy_nonoverlapping(attrs.as_ptr(), (*args).attrs.as_mut_ptr(), attrs.len());
        }
        Self {
            storage,
            attr_count: attrs.len(),
        }
    }

    pub fn raw_mut(&mut self) -> &mut kfd::kfd_ioctl_svm_args {
        unsafe { &mut *(self.storage.as_mut_ptr() as *mut kfd::kfd_ioctl_svm_args) }
    }

    pub fn to_response(&self) -> AmdkfdIocSvmResponse {
        let args = unsafe { &*(self.storage.as_ptr() as *const kfd::kfd_ioctl_svm_args) };
        let attrs = unsafe { args.attrs.as_slice(self.attr_count) };
        AmdkfdIocSvmResponse {
            attrs: attrs
                .iter()
                .map(|attr| KfdSvmAttribute {
                    attr_type: attr.type_,
                    value: attr.value,
                })
                .collect(),
        }
    }
}

impl FromC<kfd::kfd_process_device_apertures> for KfdProcessDeviceAperture {
    fn from_c(raw: kfd::kfd_process_device_apertures) -> Self {
        Self {
            lds_base: raw.lds_base,
            lds_limit: raw.lds_limit,
            scratch_base: raw.scratch_base,
            scratch_limit: raw.scratch_limit,
            gpuvm_base: raw.gpuvm_base,
            gpuvm_limit: raw.gpuvm_limit,
            gpu_id: raw.gpu_id,
        }
    }
}

impl ToC<KfdEventDataRaw> for KfdEventData {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> KfdEventDataRaw {
        let mut raw = KfdEventDataRaw {
            kfd_event_data_ext: self.kfd_event_data_ext,
            event_id: self.event_id,
            ..Default::default()
        };
        raw.payload = if let Some(signal) = self.signal_event_data {
            KfdEventUnionRaw {
                signal_event_data: KfdSignalEventDataRaw {
                    last_event_age: signal.last_event_age,
                },
            }
        } else if let Some(hw) = self.hw_exception_data {
            KfdEventUnionRaw {
                hw_exception_data: KfdHwExceptionDataRaw {
                    reset_type: hw.reset_type,
                    reset_cause: hw.reset_cause,
                    memory_lost: hw.memory_lost,
                    gpu_id: hw.gpu_id,
                },
            }
        } else if let Some(memory) = self.memory_exception_data {
            KfdEventUnionRaw {
                memory_exception_data: KfdMemoryExceptionDataRaw {
                    failure: KfdMemoryExceptionFailureRaw {
                        not_present: memory.failure.not_present,
                        read_only: memory.failure.read_only,
                        no_execute: memory.failure.no_execute,
                        imprecise: memory.failure.imprecise,
                    },
                    va: memory.va,
                    gpu_id: memory.gpu_id,
                    error_type: memory.error_type,
                },
            }
        } else {
            KfdEventUnionRaw::default()
        };
        raw
    }
}

impl FromC<KfdEventDataRaw> for KfdEventData {
    fn from_c(raw: KfdEventDataRaw) -> Self {
        let signal = unsafe { raw.payload.signal_event_data };
        let hw = unsafe { raw.payload.hw_exception_data };
        let memory = unsafe { raw.payload.memory_exception_data };
        Self {
            event_id: raw.event_id,
            memory_exception_data: (memory.va != 0
                || memory.gpu_id != 0
                || memory.error_type != 0
                || memory.failure.not_present != 0
                || memory.failure.read_only != 0
                || memory.failure.no_execute != 0
                || memory.failure.imprecise != 0)
                .then_some(KfdMemoryExceptionData {
                    failure: KfdMemoryExceptionFailure {
                        not_present: memory.failure.not_present,
                        read_only: memory.failure.read_only,
                        no_execute: memory.failure.no_execute,
                        imprecise: memory.failure.imprecise,
                    },
                    va: memory.va,
                    gpu_id: memory.gpu_id,
                    error_type: memory.error_type,
                }),
            hw_exception_data: (hw.reset_type != 0
                || hw.reset_cause != 0
                || hw.memory_lost != 0
                || hw.gpu_id != 0)
                .then_some(KfdHwExceptionData {
                    reset_type: hw.reset_type,
                    reset_cause: hw.reset_cause,
                    memory_lost: hw.memory_lost,
                    gpu_id: hw.gpu_id,
                }),
            signal_event_data: Some(KfdSignalEventData {
                last_event_age: signal.last_event_age,
            }),
            kfd_event_data_ext: raw.kfd_event_data_ext,
        }
    }
}

impl ToC<KfdCriuDeviceBucketRaw> for KfdCriuDeviceBucket {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> KfdCriuDeviceBucketRaw {
        KfdCriuDeviceBucketRaw {
            user_gpu_id: self.user_gpu_id,
            actual_gpu_id: self.actual_gpu_id,
            drm_fd: self.drm_fd,
            pad: 0,
        }
    }
}

impl FromC<KfdCriuDeviceBucketRaw> for KfdCriuDeviceBucket {
    fn from_c(raw: KfdCriuDeviceBucketRaw) -> Self {
        Self {
            user_gpu_id: raw.user_gpu_id,
            actual_gpu_id: raw.actual_gpu_id,
            drm_fd: raw.drm_fd,
        }
    }
}

impl ToC<KfdCriuBoBucketRaw> for KfdCriuBoBucket {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> KfdCriuBoBucketRaw {
        KfdCriuBoBucketRaw {
            addr: self.addr,
            size: self.size,
            offset: self.offset,
            restored_offset: self.restored_offset,
            gpu_id: self.gpu_id,
            alloc_flags: self.alloc_flags,
            dmabuf_fd: self.dmabuf_fd,
            pad: 0,
        }
    }
}

impl FromC<KfdCriuBoBucketRaw> for KfdCriuBoBucket {
    fn from_c(raw: KfdCriuBoBucketRaw) -> Self {
        Self {
            addr: raw.addr,
            size: raw.size,
            offset: raw.offset,
            restored_offset: raw.restored_offset,
            gpu_id: raw.gpu_id,
            alloc_flags: raw.alloc_flags,
            dmabuf_fd: raw.dmabuf_fd,
        }
    }
}

impl ToC<KfdMemoryRangeRaw> for KfdMemoryRange {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> KfdMemoryRangeRaw {
        KfdMemoryRangeRaw {
            va_addr: self.va_addr,
            size: self.size,
        }
    }
}

impl ToC<KfdPcSampleInfoRaw> for KfdPcSampleInfo {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> KfdPcSampleInfoRaw {
        KfdPcSampleInfoRaw {
            interval: self.interval,
            interval_min: self.interval_min,
            interval_max: self.interval_max,
            flags: self.flags,
            method: self.method as u32,
            type_: self.sample_type as u32,
        }
    }
}

impl FromC<KfdPcSampleInfoRaw> for KfdPcSampleInfo {
    fn from_c(raw: KfdPcSampleInfoRaw) -> Self {
        Self {
            interval: raw.interval,
            interval_min: raw.interval_min,
            interval_max: raw.interval_max,
            flags: raw.flags,
            method: pc_sample_method_from_raw(raw.method),
            sample_type: pc_sample_type_from_raw(raw.type_),
        }
    }
}

pub fn dbg_trap_to_c(
    request: &AmdkfdIocDbgTrapRequest,
    owned: &mut DbgTrapOwned,
) -> kfd::kfd_ioctl_dbg_trap_args {
    let mut raw = kfd::kfd_ioctl_dbg_trap_args {
        pid: request.pid,
        op: request.op as u32,
        ..Default::default()
    };
    raw.__bindgen_anon_1 = match request.op {
        KfdDbgTrapOp::Enable => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            enable: {
                let enable = request
                    .args
                    .enable
                    .unwrap_or(crate::amdgpu::KfdDbgTrapEnableArgs {
                        exception_mask: 0,
                        rinfo_size: 0,
                        dbg_fd: 0,
                    });
                kfd::kfd_ioctl_dbg_trap_enable_args {
                    exception_mask: enable.exception_mask,
                    rinfo_ptr: 0,
                    rinfo_size: enable.rinfo_size,
                    dbg_fd: enable.dbg_fd,
                }
            },
        },
        KfdDbgTrapOp::Disable => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1::default(),
        KfdDbgTrapOp::SendRuntimeEvent => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            send_runtime_event: {
                let args = request.args.send_runtime_event.unwrap_or(
                    crate::amdgpu::KfdDbgTrapSendRuntimeEventArgs {
                        exception_mask: 0,
                        gpu_id: 0,
                        queue_id: 0,
                    },
                );
                kfd::kfd_ioctl_dbg_trap_send_runtime_event_args {
                    exception_mask: args.exception_mask,
                    gpu_id: args.gpu_id,
                    queue_id: args.queue_id,
                }
            },
        },
        KfdDbgTrapOp::SetExceptionsEnabled => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            set_exceptions_enabled: {
                let args = request.args.set_exceptions_enabled.unwrap_or(
                    crate::amdgpu::KfdDbgTrapSetExceptionsEnabledArgs { exception_mask: 0 },
                );
                kfd::kfd_ioctl_dbg_trap_set_exceptions_enabled_args {
                    exception_mask: args.exception_mask,
                }
            },
        },
        KfdDbgTrapOp::SetWaveLaunchOverride => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            launch_override: {
                let args = request.args.launch_override.unwrap_or(
                    crate::amdgpu::KfdDbgTrapSetWaveLaunchOverrideArgs {
                        override_mode: KfdDbgTrapOverrideMode::Or,
                        enable_mask: 0,
                        support_request_mask: 0,
                    },
                );
                kfd::kfd_ioctl_dbg_trap_set_wave_launch_override_args {
                    override_mode: args.override_mode as u32,
                    enable_mask: args.enable_mask,
                    support_request_mask: args.support_request_mask,
                    pad: 0,
                }
            },
        },
        KfdDbgTrapOp::SetWaveLaunchMode => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            launch_mode: {
                let args = request.args.launch_mode.unwrap_or(
                    crate::amdgpu::KfdDbgTrapSetWaveLaunchModeArgs {
                        launch_mode: KfdDbgTrapWaveLaunchMode::Normal,
                    },
                );
                kfd::kfd_ioctl_dbg_trap_set_wave_launch_mode_args {
                    launch_mode: args.launch_mode as u32,
                    pad: 0,
                }
            },
        },
        KfdDbgTrapOp::SuspendQueues => {
            let args = request.args.suspend_queues.clone().unwrap_or(
                crate::amdgpu::KfdDbgTrapSuspendQueuesArgs {
                    exception_mask: 0,
                    queue_ids: Vec::new(),
                    grace_period: 0,
                },
            );
            owned.queue_ids = args.queue_ids;
            kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                suspend_queues: kfd::kfd_ioctl_dbg_trap_suspend_queues_args {
                    exception_mask: args.exception_mask,
                    queue_array_ptr: maybe_ptr(&owned.queue_ids),
                    num_queues: owned.queue_ids.len() as u32,
                    grace_period: args.grace_period,
                },
            }
        }
        KfdDbgTrapOp::ResumeQueues => {
            let args = request.args.resume_queues.clone().unwrap_or(
                crate::amdgpu::KfdDbgTrapResumeQueuesArgs {
                    queue_ids: Vec::new(),
                },
            );
            owned.queue_ids = args.queue_ids;
            kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                resume_queues: kfd::kfd_ioctl_dbg_trap_resume_queues_args {
                    queue_array_ptr: maybe_ptr(&owned.queue_ids),
                    num_queues: owned.queue_ids.len() as u32,
                    pad: 0,
                },
            }
        }
        KfdDbgTrapOp::SetNodeAddressWatch => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            set_node_address_watch: {
                let args = request.args.set_node_address_watch.unwrap_or(
                    crate::amdgpu::KfdDbgTrapSetNodeAddressWatchArgs {
                        address: 0,
                        mode: KfdDbgTrapAddressWatchMode::Read,
                        mask: 0,
                        gpu_id: 0,
                        id: 0,
                    },
                );
                kfd::kfd_ioctl_dbg_trap_set_node_address_watch_args {
                    address: args.address,
                    mode: args.mode as u32,
                    mask: args.mask,
                    gpu_id: args.gpu_id,
                    id: args.id,
                }
            },
        },
        KfdDbgTrapOp::ClearNodeAddressWatch => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            clear_node_address_watch: {
                let args = request.args.clear_node_address_watch.unwrap_or(
                    crate::amdgpu::KfdDbgTrapClearNodeAddressWatchArgs { gpu_id: 0, id: 0 },
                );
                kfd::kfd_ioctl_dbg_trap_clear_node_address_watch_args {
                    gpu_id: args.gpu_id,
                    id: args.id,
                }
            },
        },
        KfdDbgTrapOp::SetFlags => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            set_flags: {
                let args = request
                    .args
                    .set_flags
                    .unwrap_or(crate::amdgpu::KfdDbgTrapSetFlagsArgs { flags: 0 });
                kfd::kfd_ioctl_dbg_trap_set_flags_args {
                    flags: args.flags,
                    pad: 0,
                }
            },
        },
        KfdDbgTrapOp::QueryDebugEvent => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
            query_debug_event: {
                let args = request.args.query_debug_event.unwrap_or(
                    crate::amdgpu::KfdDbgTrapQueryDebugEventArgs {
                        exception_mask: 0,
                        gpu_id: 0,
                        queue_id: 0,
                    },
                );
                kfd::kfd_ioctl_dbg_trap_query_debug_event_args {
                    exception_mask: args.exception_mask,
                    gpu_id: args.gpu_id,
                    queue_id: args.queue_id,
                }
            },
        },
        KfdDbgTrapOp::QueryExceptionInfo => {
            let args = request.args.query_exception_info.clone().unwrap_or(
                crate::amdgpu::KfdDbgTrapQueryExceptionInfoArgs {
                    info: Vec::new(),
                    info_size: 0,
                    source_id: 0,
                    exception_code: 0,
                    clear_exception: 0,
                },
            );
            owned.info = args.info;
            kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                query_exception_info: kfd::kfd_ioctl_dbg_trap_query_exception_info_args {
                    info_ptr: maybe_mut_ptr(&mut owned.info),
                    info_size: args.info_size,
                    source_id: args.source_id,
                    exception_code: args.exception_code,
                    clear_exception: args.clear_exception,
                },
            }
        }
        KfdDbgTrapOp::GetQueueSnapshot => {
            let args = request.args.queue_snapshot.clone().unwrap_or(
                crate::amdgpu::KfdDbgTrapQueueSnapshotArgs {
                    exception_mask: 0,
                    entries: Vec::new(),
                    num_queues: 0,
                    entry_size: 0,
                },
            );
            owned.queue_snapshot =
                vec![KfdQueueSnapshotEntryRaw::default(); args.num_queues as usize];
            kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                queue_snapshot: kfd::kfd_ioctl_dbg_trap_queue_snapshot_args {
                    exception_mask: args.exception_mask,
                    snapshot_buf_ptr: maybe_mut_ptr(&mut owned.queue_snapshot),
                    num_queues: owned.queue_snapshot.len() as u32,
                    entry_size: args.entry_size,
                },
            }
        }
        KfdDbgTrapOp::GetDeviceSnapshot => {
            let args = request.args.device_snapshot.clone().unwrap_or(
                crate::amdgpu::KfdDbgTrapDeviceSnapshotArgs {
                    exception_mask: 0,
                    entries: Vec::new(),
                    num_devices: 0,
                    entry_size: 0,
                },
            );
            owned.device_snapshot =
                vec![KfdDbgDeviceInfoEntryRaw::default(); args.num_devices as usize];
            kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                device_snapshot: kfd::kfd_ioctl_dbg_trap_device_snapshot_args {
                    exception_mask: args.exception_mask,
                    snapshot_buf_ptr: maybe_mut_ptr(&mut owned.device_snapshot),
                    num_devices: owned.device_snapshot.len() as u32,
                    entry_size: args.entry_size,
                },
            }
        }
    };
    raw
}

pub fn dbg_trap_from_c(
    op: KfdDbgTrapOp,
    raw: &kfd::kfd_ioctl_dbg_trap_args,
    owned: &DbgTrapOwned,
) -> KfdDbgTrapArgs {
    let mut args = KfdDbgTrapArgs {
        enable: None,
        send_runtime_event: None,
        set_exceptions_enabled: None,
        launch_override: None,
        launch_mode: None,
        suspend_queues: None,
        resume_queues: None,
        set_node_address_watch: None,
        clear_node_address_watch: None,
        set_flags: None,
        query_debug_event: None,
        query_exception_info: None,
        queue_snapshot: None,
        device_snapshot: None,
    };
    unsafe {
        match op {
            KfdDbgTrapOp::Enable => {
                let enable = raw.__bindgen_anon_1.enable;
                args.enable = Some(crate::amdgpu::KfdDbgTrapEnableArgs {
                    exception_mask: enable.exception_mask,
                    rinfo_size: enable.rinfo_size,
                    dbg_fd: enable.dbg_fd,
                });
            }
            KfdDbgTrapOp::SendRuntimeEvent => {
                let event = raw.__bindgen_anon_1.send_runtime_event;
                args.send_runtime_event = Some(crate::amdgpu::KfdDbgTrapSendRuntimeEventArgs {
                    exception_mask: event.exception_mask,
                    gpu_id: event.gpu_id,
                    queue_id: event.queue_id,
                });
            }
            KfdDbgTrapOp::SetExceptionsEnabled => {
                args.set_exceptions_enabled =
                    Some(crate::amdgpu::KfdDbgTrapSetExceptionsEnabledArgs {
                        exception_mask: raw.__bindgen_anon_1.set_exceptions_enabled.exception_mask,
                    });
            }
            KfdDbgTrapOp::SetWaveLaunchOverride => {
                let value = raw.__bindgen_anon_1.launch_override;
                args.launch_override = Some(crate::amdgpu::KfdDbgTrapSetWaveLaunchOverrideArgs {
                    override_mode: dbg_trap_override_mode_from_raw(value.override_mode),
                    enable_mask: value.enable_mask,
                    support_request_mask: value.support_request_mask,
                });
            }
            KfdDbgTrapOp::SetWaveLaunchMode => {
                args.launch_mode = Some(crate::amdgpu::KfdDbgTrapSetWaveLaunchModeArgs {
                    launch_mode: dbg_trap_wave_launch_mode_from_raw(
                        raw.__bindgen_anon_1.launch_mode.launch_mode,
                    ),
                });
            }
            KfdDbgTrapOp::SuspendQueues => {
                let value = raw.__bindgen_anon_1.suspend_queues;
                args.suspend_queues = Some(crate::amdgpu::KfdDbgTrapSuspendQueuesArgs {
                    exception_mask: value.exception_mask,
                    queue_ids: owned.queue_ids.clone(),
                    grace_period: value.grace_period,
                });
            }
            KfdDbgTrapOp::ResumeQueues => {
                args.resume_queues = Some(crate::amdgpu::KfdDbgTrapResumeQueuesArgs {
                    queue_ids: owned.queue_ids.clone(),
                });
            }
            KfdDbgTrapOp::SetNodeAddressWatch => {
                let value = raw.__bindgen_anon_1.set_node_address_watch;
                args.set_node_address_watch =
                    Some(crate::amdgpu::KfdDbgTrapSetNodeAddressWatchArgs {
                        address: value.address,
                        mode: dbg_trap_address_watch_mode_from_raw(value.mode),
                        mask: value.mask,
                        gpu_id: value.gpu_id,
                        id: value.id,
                    });
            }
            KfdDbgTrapOp::ClearNodeAddressWatch => {
                let value = raw.__bindgen_anon_1.clear_node_address_watch;
                args.clear_node_address_watch =
                    Some(crate::amdgpu::KfdDbgTrapClearNodeAddressWatchArgs {
                        gpu_id: value.gpu_id,
                        id: value.id,
                    });
            }
            KfdDbgTrapOp::SetFlags => {
                args.set_flags = Some(crate::amdgpu::KfdDbgTrapSetFlagsArgs {
                    flags: raw.__bindgen_anon_1.set_flags.flags,
                });
            }
            KfdDbgTrapOp::QueryDebugEvent => {
                let value = raw.__bindgen_anon_1.query_debug_event;
                args.query_debug_event = Some(crate::amdgpu::KfdDbgTrapQueryDebugEventArgs {
                    exception_mask: value.exception_mask,
                    gpu_id: value.gpu_id,
                    queue_id: value.queue_id,
                });
            }
            KfdDbgTrapOp::QueryExceptionInfo => {
                let value = raw.__bindgen_anon_1.query_exception_info;
                args.query_exception_info = Some(crate::amdgpu::KfdDbgTrapQueryExceptionInfoArgs {
                    info: owned.info.clone(),
                    info_size: value.info_size,
                    source_id: value.source_id,
                    exception_code: value.exception_code,
                    clear_exception: value.clear_exception,
                });
            }
            KfdDbgTrapOp::GetQueueSnapshot => {
                let value = raw.__bindgen_anon_1.queue_snapshot;
                args.queue_snapshot = Some(crate::amdgpu::KfdDbgTrapQueueSnapshotArgs {
                    exception_mask: value.exception_mask,
                    entries: owned
                        .queue_snapshot
                        .iter()
                        .take(value.num_queues as usize)
                        .map(queue_snapshot_entry_from_raw)
                        .collect(),
                    num_queues: value.num_queues,
                    entry_size: value.entry_size,
                });
            }
            KfdDbgTrapOp::GetDeviceSnapshot => {
                let value = raw.__bindgen_anon_1.device_snapshot;
                args.device_snapshot = Some(crate::amdgpu::KfdDbgTrapDeviceSnapshotArgs {
                    exception_mask: value.exception_mask,
                    entries: owned
                        .device_snapshot
                        .iter()
                        .take(value.num_devices as usize)
                        .map(device_snapshot_entry_from_raw)
                        .collect(),
                    num_devices: value.num_devices,
                    entry_size: value.entry_size,
                });
            }
            KfdDbgTrapOp::Disable => {}
        }
    }
    args
}

pub fn profiler_args_to_c(
    args: &KfdProfilerArgs,
    owned: &mut KfdProfilerOwned,
) -> kfd::kfd_ioctl_profiler_args__bindgen_ty_1 {
    if let Some(pc_sample) = &args.pc_sample {
        owned.sample_info = pc_sample
            .sample_info
            .iter()
            .map(|info| info.to_c(&mut ()))
            .collect();
        kfd::kfd_ioctl_profiler_args__bindgen_ty_1 {
            pc_sample: kfd::kfd_ioctl_pc_sample_args {
                sample_info_ptr: maybe_mut_ptr(&mut owned.sample_info),
                num_sample_info: pc_sample.num_sample_info,
                op: pc_sample.op as u32,
                gpu_id: pc_sample.gpu_id,
                trace_id: pc_sample.trace_id,
                flags: pc_sample.flags,
                reserved: pc_sample.reserved,
            },
        }
    } else if let Some(pmc) = args.pmc {
        kfd::kfd_ioctl_profiler_args__bindgen_ty_1 {
            pmc: kfd::kfd_ioctl_pmc_settings {
                gpu_id: pmc.gpu_id,
                lock: pmc.lock,
                perfcount_enable: pmc.perfcount_enable,
            },
        }
    } else {
        kfd::kfd_ioctl_profiler_args__bindgen_ty_1 {
            version: args.version.unwrap_or_default(),
        }
    }
}

pub fn profiler_args_from_c(
    op: KfdProfilerOp,
    args: &kfd::kfd_ioctl_profiler_args,
    owned: &KfdProfilerOwned,
) -> KfdProfilerArgs {
    unsafe {
        match op {
            KfdProfilerOp::Pmc => KfdProfilerArgs {
                pc_sample: None,
                pmc: Some(KfdPmcSettings {
                    gpu_id: args.__bindgen_anon_1.pmc.gpu_id,
                    lock: args.__bindgen_anon_1.pmc.lock,
                    perfcount_enable: args.__bindgen_anon_1.pmc.perfcount_enable,
                }),
                version: None,
            },
            KfdProfilerOp::PcSample => KfdProfilerArgs {
                pc_sample: Some(KfdPcSampleArgs {
                    sample_info: owned
                        .sample_info
                        .iter()
                        .copied()
                        .map(KfdPcSampleInfo::from_c)
                        .collect(),
                    num_sample_info: args.__bindgen_anon_1.pc_sample.num_sample_info,
                    op: pc_sample_op_from_raw(args.__bindgen_anon_1.pc_sample.op),
                    gpu_id: args.__bindgen_anon_1.pc_sample.gpu_id,
                    trace_id: args.__bindgen_anon_1.pc_sample.trace_id,
                    flags: args.__bindgen_anon_1.pc_sample.flags,
                    reserved: args.__bindgen_anon_1.pc_sample.reserved,
                }),
                pmc: None,
                version: None,
            },
            KfdProfilerOp::Version => KfdProfilerArgs {
                pc_sample: None,
                pmc: None,
                version: Some(args.__bindgen_anon_1.version),
            },
        }
    }
}

pub fn ais_op_response_from_c(args: &kfd::kfd_ioctl_ais_args) -> AmdkfdIocAisOpResponse {
    let out = unsafe { args.__bindgen_anon_1.out };
    AmdkfdIocAisOpResponse {
        size_copied: out.size_copied,
        status: out.status,
    }
}

fn queue_snapshot_entry_from_raw(entry: &KfdQueueSnapshotEntryRaw) -> KfdQueueSnapshotEntry {
    KfdQueueSnapshotEntry {
        exception_status: entry.exception_status,
        ring_base_address: entry.ring_base_address,
        write_pointer_address: entry.write_pointer_address,
        read_pointer_address: entry.read_pointer_address,
        ctx_save_restore_address: entry.ctx_save_restore_address,
        queue_id: entry.queue_id,
        gpu_id: entry.gpu_id,
        ring_size: entry.ring_size,
        queue_type: entry.queue_type,
        ctx_save_restore_area_size: entry.ctx_save_restore_area_size,
        reserved: entry.reserved,
    }
}

fn device_snapshot_entry_from_raw(entry: &KfdDbgDeviceInfoEntryRaw) -> KfdDbgDeviceInfoEntry {
    KfdDbgDeviceInfoEntry {
        exception_status: entry.exception_status,
        lds_base: entry.lds_base,
        lds_limit: entry.lds_limit,
        scratch_base: entry.scratch_base,
        scratch_limit: entry.scratch_limit,
        gpuvm_base: entry.gpuvm_base,
        gpuvm_limit: entry.gpuvm_limit,
        gpu_id: entry.gpu_id,
        location_id: entry.location_id,
        vendor_id: entry.vendor_id,
        device_id: entry.device_id,
        revision_id: entry.revision_id,
        subsystem_vendor_id: entry.subsystem_vendor_id,
        subsystem_device_id: entry.subsystem_device_id,
        fw_version: entry.fw_version,
        gfx_target_version: entry.gfx_target_version,
        simd_count: entry.simd_count,
        max_waves_per_simd: entry.max_waves_per_simd,
        array_count: entry.array_count,
        simd_arrays_per_engine: entry.simd_arrays_per_engine,
        num_xcc: entry.num_xcc,
        capability: entry.capability,
        debug_prop: entry.debug_prop,
    }
}

fn dbg_trap_override_mode_from_raw(value: u32) -> KfdDbgTrapOverrideMode {
    match value {
        0 => KfdDbgTrapOverrideMode::Or,
        1 => KfdDbgTrapOverrideMode::Replace,
        _ => KfdDbgTrapOverrideMode::Or,
    }
}

fn dbg_trap_wave_launch_mode_from_raw(value: u32) -> KfdDbgTrapWaveLaunchMode {
    match value {
        0 => KfdDbgTrapWaveLaunchMode::Normal,
        1 => KfdDbgTrapWaveLaunchMode::Halt,
        3 => KfdDbgTrapWaveLaunchMode::Debug,
        _ => KfdDbgTrapWaveLaunchMode::Normal,
    }
}

fn dbg_trap_address_watch_mode_from_raw(value: u32) -> KfdDbgTrapAddressWatchMode {
    match value {
        0 => KfdDbgTrapAddressWatchMode::Read,
        1 => KfdDbgTrapAddressWatchMode::Nonread,
        2 => KfdDbgTrapAddressWatchMode::Atomic,
        3 => KfdDbgTrapAddressWatchMode::All,
        _ => KfdDbgTrapAddressWatchMode::Read,
    }
}

fn pc_sample_method_from_raw(value: u32) -> KfdPcSampleMethod {
    match value {
        1 => KfdPcSampleMethod::Hosttrap,
        2 => KfdPcSampleMethod::Stochastic,
        _ => KfdPcSampleMethod::Hosttrap,
    }
}

fn pc_sample_type_from_raw(value: u32) -> KfdPcSampleType {
    match value {
        0 => KfdPcSampleType::TimeUs,
        1 => KfdPcSampleType::ClockCycles,
        2 => KfdPcSampleType::Instructions,
        _ => KfdPcSampleType::TimeUs,
    }
}

fn pc_sample_op_from_raw(value: u32) -> KfdPcSampleOp {
    match value {
        0 => KfdPcSampleOp::QueryCapabilities,
        1 => KfdPcSampleOp::Create,
        2 => KfdPcSampleOp::Destroy,
        3 => KfdPcSampleOp::Start,
        4 => KfdPcSampleOp::Stop,
        _ => KfdPcSampleOp::QueryCapabilities,
    }
}
