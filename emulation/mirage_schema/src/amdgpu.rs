//! AMD GPU ioctl request/response definitions for `/dev/kfd` and the DRM
//! render nodes, mirroring `schema/kfd.fbs` and `schema/drm_amdgpu.fbs`.
//!
//! For every ioctl inside, the [`ioctl_dsl!`] macro emits:
//!
//! * `pub const $NAME: u32` — the ioctl number.
//! * `pub struct $NameRequest` / `$NameResponse` — request / response
//!   payloads.
//! * a `Handle{Subsys}Ioctl` trait with one method per ioctl.

use serde::{Deserialize, Serialize};

use crate::ioctl_dsl;

/// Context passed to every ioctl handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoctlCtx {
    pub pid: u32,
    pub tid: u32,
}

macro_rules! simple_enum {
    (
        $(#[$meta:meta])*
        $name:ident : $repr:ty { $( $variant:ident = $val:expr ),* $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        #[repr($repr)]
        pub enum $name {
            $(
                $variant = $val,
            )*
        }
    };
}

simple_enum! {
    /// GPU hardware IP type for a command submission target.
    HwIpType : u32 {
        Gfx = 0,
        Compute = 1,
        Dma = 2,
        Uvd = 3,
        Vce = 4,
        UvdEnc = 5,
        VcnDec = 6,
        VcnEnc = 7,
        VcnJpeg = 8,
        Vpe = 9
    }
}

simple_enum! {
    /// Context operations for `DRM_AMDGPU_CTX`.
    CtxOp : u32 {
        None = 0,
        AllocCtx = 1,
        FreeCtx = 2,
        QueryState = 3,
        QueryState2 = 4,
        GetStablePstate = 5,
        SetStablePstate = 6
    }
}

simple_enum! {
    /// Virtual address operations for `DRM_AMDGPU_GEM_VA`.
    VaOp : u32 {
        None = 0,
        Map = 1,
        Unmap = 2,
        Clear = 3,
        Replace = 4
    }
}

simple_enum! {
    /// Buffer-object residency list operations.
    BoListOp : u32 {
        Create = 0,
        Destroy = 1,
        Update = 2
    }
}

simple_enum! {
    /// Command-submission chunk ID.
    ChunkId : u32 {
        None = 0,
        Ib = 1,
        Fence = 2,
        Dependencies = 3,
        SyncobjIn = 4,
        SyncobjOut = 5,
        BoHandles = 6,
        ScheduledDependencies = 7,
        SyncobjTimelineWait = 8,
        SyncobjTimelineSignal = 9,
        CpGfxShadow = 10
    }
}

simple_enum! {
    /// Semaphore operations (hybrid-specific).
    SemOp : u32 {
        None = 0,
        CreateSem = 1,
        WaitSem = 2,
        SignalSem = 3,
        DestroySem = 4,
        ImportSem = 5,
        ExportSem = 6
    }
}

simple_enum! {
    /// VM operations for `DRM_AMDGPU_VM`.
    VmOp : u32 {
        None = 0,
        ReserveVmid = 1,
        UnreserveVmid = 2
    }
}

simple_enum! {
    /// Scheduler operations.
    SchedOp : u32 {
        None = 0,
        ProcessPriorityOverride = 1,
        ContextPriorityOverride = 2
    }
}

simple_enum! {
    /// Fence-to-handle conversion target.
    FenceToHandleType : u32 {
        GetSyncobj = 0,
        GetSyncobjFd = 1,
        GetSyncFileFd = 2
    }
}

simple_enum! {
    /// User-queue operations.
    UserqOp : u32 {
        None = 0,
        UserqCreate = 1,
        UserqFree = 2
    }
}

simple_enum! {
    /// GEM metadata operations.
    GemMetadataOp : u32 {
        None = 0,
        SetMetadata = 1,
        GetMetadata = 2
    }
}

simple_enum! {
    /// GEM property operations.
    GemOpType : u32 {
        GetGemCreateInfo = 0,
        SetPlacement = 1,
        GetMappingInfo = 2
    }
}

simple_enum! {
    /// DGMA operations (hybrid-specific).
    DgmaOp : u32 {
        Import = 0,
        QueryPhysAddr = 1
    }
}

simple_enum! {
    /// KFD queue types.
    KfdQueueType : u32 {
        Compute = 0,
        Sdma = 1,
        ComputeAql = 2,
        SdmaXgmi = 3,
        SdmaByEngId = 4
    }
}

simple_enum! {
    /// KFD event types.
    KfdEventType : u32 {
        Signal = 0,
        NodeChange = 1,
        DeviceStateChange = 2,
        HwException = 3,
        SystemEvent = 4,
        DebugEvent = 5,
        ProfileEvent = 6,
        QueueEvent = 7,
        Memory = 8
    }
}

simple_enum! {
    /// KFD cache coherency policy.
    KfdCachePolicy : u32 {
        Coherent = 0,
        Noncoherent = 1
    }
}

simple_enum! {
    /// KFD CRIU operations.
    KfdCriuOp : u32 {
        ProcessInfo = 0,
        Checkpoint = 1,
        Unpause = 2,
        Restore = 3,
        Resume = 4
    }
}

simple_enum! {
    /// KFD SVM operations.
    KfdSvmOp : u32 {
        SetAttr = 0,
        GetAttr = 1
    }
}

simple_enum! {
    /// KFD RLC SPM (Streaming Performance Monitor) operations.
    KfdSpmOp : u32 {
        Acquire = 0,
        Release = 1,
        SetDestBuf = 2
    }
}

simple_enum! {
    /// KFD PC sampling operations.
    KfdPcSampleOp : u32 {
        QueryCapabilities = 0,
        Create = 1,
        Destroy = 2,
        Start = 3,
        Stop = 4
    }
}

simple_enum! {
    /// KFD profiler operations.
    KfdProfilerOp : u32 {
        Pmc = 0,
        PcSample = 1,
        Version = 2
    }
}

simple_enum! {
    /// KFD AIS (AMD Infinity Storage) operations.
    KfdAisOp : u32 {
        Read = 1,
        Write = 2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KfdMemoryRange {
    pub va_addr: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoListEntry {
    pub bo_handle: u32,
    pub bo_priority: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsChunkIb {
    pub flags: u32,
    pub va_address: u64,
    pub ib_bytes: u32,
    pub ip_type: HwIpType,
    pub ip_instance: u32,
    pub ring: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsChunkFence {
    pub handle: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsChunkDep {
    pub ip_type: u32,
    pub ip_instance: u32,
    pub ring: u32,
    pub ctx_id: u32,
    pub handle: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsChunkSyncobj {
    pub handle: u32,
    pub flags: u32,
    pub point: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsChunkCpGfxShadow {
    pub shadow_va: u64,
    pub csa_va: u64,
    pub gds_va: u64,
    pub flags: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CsChunk {
    pub chunk_id: ChunkId,
    pub ib: Option<CsChunkIb>,
    pub fence: Option<CsChunkFence>,
    pub dependencies: Vec<CsChunkDep>,
    pub syncobjs: Vec<CsChunkSyncobj>,
    pub bo_handles: Vec<u32>,
    pub cp_gfx_shadow: Option<CsChunkCpGfxShadow>,
    pub raw_data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fence {
    pub ctx_id: u32,
    pub ip_type: u32,
    pub ip_instance: u32,
    pub ring: u32,
    pub seq_no: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserqFenceInfo {
    pub gpu_va: u64,
    pub value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GemListHandlesEntry {
    pub gem_handle: u32,
    pub flags: u32,
    pub size: u64,
    pub preferred_domains: u64,
    pub alloc_flags: u64,
    pub alignment: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KfdProcessDeviceAperture {
    pub lds_base: u64,
    pub lds_limit: u64,
    pub scratch_base: u64,
    pub scratch_limit: u64,
    pub gpuvm_base: u64,
    pub gpuvm_limit: u64,
    pub gpu_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KfdEventData {
    pub event_id: u32,
    pub last_event_age: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KfdSvmAttribute {
    pub attr_type: u32,
    pub value: u32,
}

ioctl_dsl! {
    kfd {
        /// `AMDKFD_IOC_GET_VERSION` — return the KFD interface version.
        AMDKFD_IOC_GET_VERSION(0x01) {} => {
            major_version : u32,
            minor_version : u32,
        };

        /// `AMDKFD_IOC_CREATE_QUEUE` — create a compute, SDMA, or AQL queue.
        AMDKFD_IOC_CREATE_QUEUE(0x02) {
            ring_base_address : u64,
            write_pointer_address : u64,
            read_pointer_address : u64,
            ring_size : u32,
            gpu_id : u32,
            queue_type : KfdQueueType,
            queue_percentage : u32,
            queue_priority : u32,
            eop_buffer_address : u64,
            eop_buffer_size : u64,
            ctx_save_restore_address : u64,
            ctx_save_restore_size : u32,
            ctl_stack_size : u32,
            sdma_engine_id : u32,
        } => {
            doorbell_offset : u64,
            queue_id : u32,
        };

        /// `AMDKFD_IOC_DESTROY_QUEUE` — destroy a previously created queue.
        AMDKFD_IOC_DESTROY_QUEUE(0x03) {
            queue_id : u32,
        } => {};

        /// `AMDKFD_IOC_SET_MEMORY_POLICY` — set cache coherency policy.
        AMDKFD_IOC_SET_MEMORY_POLICY(0x04) {
            alternate_aperture_base : u64,
            alternate_aperture_size : u64,
            gpu_id : u32,
            default_policy : KfdCachePolicy,
            alternate_policy : KfdCachePolicy,
            misc_process_flag : u32,
        } => {};

        /// `AMDKFD_IOC_GET_CLOCK_COUNTERS` — GPU and CPU clock counters.
        AMDKFD_IOC_GET_CLOCK_COUNTERS(0x05) {
            gpu_id : u32,
        } => {
            gpu_clock_counter : u64,
            cpu_clock_counter : u64,
            system_clock_counter : u64,
            system_clock_freq : u64,
        };

        /// `AMDKFD_IOC_GET_PROCESS_APERTURES` — deprecated, limited to 7 GPUs.
        /// Use [`AMDKFD_IOC_GET_PROCESS_APERTURES_NEW`] instead.
        AMDKFD_IOC_GET_PROCESS_APERTURES(0x06) {} => {
            apertures : Vec<KfdProcessDeviceAperture>,
        };

        /// `AMDKFD_IOC_UPDATE_QUEUE` — modify an existing queue.
        AMDKFD_IOC_UPDATE_QUEUE(0x07) {
            ring_base_address : u64,
            queue_id : u32,
            ring_size : u32,
            queue_percentage : u32,
            queue_priority : u32,
        } => {};

        /// `AMDKFD_IOC_CREATE_EVENT` — create a GPU event.
        AMDKFD_IOC_CREATE_EVENT(0x08) {
            event_type : KfdEventType,
            auto_reset : u32,
            node_id : u32,
        } => {
            event_page_offset : u64,
            event_trigger_data : u32,
            event_id : u32,
            event_slot_index : u32,
        };

        /// `AMDKFD_IOC_DESTROY_EVENT` — destroy a GPU event.
        AMDKFD_IOC_DESTROY_EVENT(0x09) {
            event_id : u32,
        } => {};

        /// `AMDKFD_IOC_SET_EVENT` — signal an event.
        AMDKFD_IOC_SET_EVENT(0x0A) {
            event_id : u32,
        } => {};

        /// `AMDKFD_IOC_RESET_EVENT` — reset an event.
        AMDKFD_IOC_RESET_EVENT(0x0B) {
            event_id : u32,
        } => {};

        /// `AMDKFD_IOC_WAIT_EVENTS` — wait for one or more events.
        AMDKFD_IOC_WAIT_EVENTS(0x0C) {
            events : Vec<KfdEventData>,
            wait_for_all : bool,
            timeout : u32,
        } => {
            wait_result : u32,
            events : Vec<u8>,
        };

        /// `AMDKFD_IOC_DBG_REGISTER_DEPRECATED` — deprecated debugger register.
        AMDKFD_IOC_DBG_REGISTER_DEPRECATED(0x0D) {
            gpu_id : u32,
        } => {};

        /// `AMDKFD_IOC_DBG_UNREGISTER_DEPRECATED` — deprecated debugger unregister.
        AMDKFD_IOC_DBG_UNREGISTER_DEPRECATED(0x0E) {
            gpu_id : u32,
        } => {};

        /// `AMDKFD_IOC_DBG_ADDRESS_WATCH_DEPRECATED` — deprecated address watch.
        AMDKFD_IOC_DBG_ADDRESS_WATCH_DEPRECATED(0x0F) {
            gpu_id : u32,
            content : Vec<u8>,
        } => {};

        /// `AMDKFD_IOC_DBG_WAVE_CONTROL_DEPRECATED` — deprecated wave control.
        AMDKFD_IOC_DBG_WAVE_CONTROL_DEPRECATED(0x10) {
            gpu_id : u32,
            content : Vec<u8>,
        } => {};

        /// `AMDKFD_IOC_SET_SCRATCH_BACKING_VA` — set scratch backing VA.
        AMDKFD_IOC_SET_SCRATCH_BACKING_VA(0x11) {
            va_addr : u64,
            gpu_id : u32,
        } => {};

        /// `AMDKFD_IOC_GET_TILE_CONFIG` — query GPU tile-mode configuration.
        AMDKFD_IOC_GET_TILE_CONFIG(0x12) {
            gpu_id : u32,
            max_tile_configs : u32,
            max_macro_tile_configs : u32,
        } => {
            tile_config : Vec<u32>,
            macro_tile_config : Vec<u32>,
            gb_addr_config : u32,
            num_banks : u32,
            num_ranks : u32,
        };

        /// `AMDKFD_IOC_SET_TRAP_HANDLER` — set trap handler addresses.
        AMDKFD_IOC_SET_TRAP_HANDLER(0x13) {
            tba_addr : u64,
            tma_addr : u64,
            gpu_id : u32,
        } => {};

        /// `AMDKFD_IOC_GET_PROCESS_APERTURES_NEW` — GPUVM aperture info.
        AMDKFD_IOC_GET_PROCESS_APERTURES_NEW(0x14) {
            max_nodes : u32,
        } => {
            apertures : Vec<KfdProcessDeviceAperture>,
        };

        /// `AMDKFD_IOC_ACQUIRE_VM` — link a KFD device to a DRM fd.
        AMDKFD_IOC_ACQUIRE_VM(0x15) {
            drm_fd : u32,
            gpu_id : u32,
        } => {};

        /// `AMDKFD_IOC_ALLOC_MEMORY_OF_GPU` — allocate memory on/for a GPU.
        AMDKFD_IOC_ALLOC_MEMORY_OF_GPU(0x16) {
            va_addr : u64,
            size : u64,
            gpu_id : u32,
            flags : u32,
            mmap_offset : u64,
        } => {
            handle : u64,
            mmap_offset : u64,
            va_addr : u64,
        };

        /// `AMDKFD_IOC_FREE_MEMORY_OF_GPU` — free GPU memory.
        AMDKFD_IOC_FREE_MEMORY_OF_GPU(0x17) {
            handle : u64,
        } => {};

        /// `AMDKFD_IOC_MAP_MEMORY_TO_GPU` — map memory to one or more GPUs.
        AMDKFD_IOC_MAP_MEMORY_TO_GPU(0x18) {
            handle : u64,
            device_ids : Vec<u32>,
        } => {
            n_success : u32,
        };

        /// `AMDKFD_IOC_UNMAP_MEMORY_FROM_GPU` — unmap memory from GPUs.
        AMDKFD_IOC_UNMAP_MEMORY_FROM_GPU(0x19) {
            handle : u64,
            device_ids : Vec<u32>,
        } => {
            n_success : u32,
        };

        /// `AMDKFD_IOC_SET_CU_MASK` — set a compute-unit affinity mask.
        AMDKFD_IOC_SET_CU_MASK(0x1A) {
            queue_id : u32,
            num_cu_mask : u32,
            cu_mask : Vec<u8>,
        } => {};

        /// `AMDKFD_IOC_GET_QUEUE_WAVE_STATE` — query wave state.
        AMDKFD_IOC_GET_QUEUE_WAVE_STATE(0x1B) {
            ctl_stack_address : u64,
            queue_id : u32,
        } => {
            ctl_stack_used_size : u32,
            save_area_used_size : u32,
        };

        /// `AMDKFD_IOC_GET_DMABUF_INFO` — query DMA-BUF metadata.
        AMDKFD_IOC_GET_DMABUF_INFO(0x1C) {
            dmabuf_fd : u32,
            metadata_size : u32,
        } => {
            size : u64,
            metadata : Vec<u8>,
            gpu_id : u32,
            flags : u32,
        };

        /// `AMDKFD_IOC_IMPORT_DMABUF` — import a DMA-BUF into KFD.
        AMDKFD_IOC_IMPORT_DMABUF(0x1D) {
            va_addr : u64,
            gpu_id : u32,
            dmabuf_fd : u32,
        } => {
            handle : u64,
        };

        /// `AMDKFD_IOC_ALLOC_QUEUE_GWS` — allocate GWS resources for a queue.
        AMDKFD_IOC_ALLOC_QUEUE_GWS(0x1E) {
            queue_id : u32,
            num_gws : u32,
        } => {
            first_gws : u32,
        };

        /// `AMDKFD_IOC_SMI_EVENTS` — subscribe to SMI events.
        AMDKFD_IOC_SMI_EVENTS(0x1F) {
            gpu_id : u32,
        } => {
            anon_fd : u32,
        };

        /// `AMDKFD_IOC_SVM` — shared-virtual-memory attribute operations.
        AMDKFD_IOC_SVM(0x20) {
            start_addr : u64,
            size : u64,
            op : KfdSvmOp,
            attrs : Vec<KfdSvmAttribute>,
        } => {
            attrs : Vec<KfdSvmAttribute>,
        };

        /// `AMDKFD_IOC_SET_XNACK_MODE` — set or query XNACK mode.
        AMDKFD_IOC_SET_XNACK_MODE(0x21) {
            xnack_enabled : i32,
        } => {
            xnack_enabled : i32,
        };

        /// `AMDKFD_IOC_CRIU_OP` — checkpoint/restore interface.
        AMDKFD_IOC_CRIU_OP(0x22) {
            op : KfdCriuOp,
            pid : u32,
            num_devices : u32,
            num_bos : u32,
            num_objects : u32,
            devices : Vec<u8>,
            bos : Vec<u8>,
            priv_data : Vec<u8>,
        } => {
            num_devices : u32,
            num_bos : u32,
            num_objects : u32,
            priv_data_size : u64,
            pid : u32,
            devices : Vec<u8>,
            bos : Vec<u8>,
            priv_data : Vec<u8>,
        };

        /// `AMDKFD_IOC_AVAILABLE_MEMORY` — query available memory.
        AMDKFD_IOC_AVAILABLE_MEMORY(0x23) {
            gpu_id : u32,
        } => {
            available : u64,
        };

        /// `AMDKFD_IOC_EXPORT_DMABUF` — export a KFD allocation as DMA-BUF.
        AMDKFD_IOC_EXPORT_DMABUF(0x24) {
            handle : u64,
            flags : u32,
        } => {
            dmabuf_fd : u32,
        };

        /// `AMDKFD_IOC_RUNTIME_ENABLE` — enable the runtime.
        AMDKFD_IOC_RUNTIME_ENABLE(0x25) {
            flags : u64,
        } => {};

        /// `AMDKFD_IOC_DBG_TRAP` — debugger trap interface.
        AMDKFD_IOC_DBG_TRAP(0x26) {
            raw_args : Vec<u8>,
        } => {
            raw_args : Vec<u8>,
        };

        /// `AMDKFD_IOC_CREATE_PROCESS` — create secondary KFD context.
        AMDKFD_IOC_CREATE_PROCESS(0x27) {
            flags : u32,
        } => {};

        /// `AMDKFD_IOC_IPC_IMPORT_HANDLE` — import an IPC share handle.
        AMDKFD_IOC_IPC_IMPORT_HANDLE(0x80) {
            va_addr : u64,
            share_handle : Vec<u8>,
            gpu_id : u32,
        } => {
            handle : u64,
            mmap_offset : u64,
            flags : u32,
        };

        /// `AMDKFD_IOC_IPC_EXPORT_HANDLE` — export memory for IPC.
        AMDKFD_IOC_IPC_EXPORT_HANDLE(0x81) {
            handle : u64,
            gpu_id : u32,
            flags : u32,
        } => {
            share_handle : Vec<u8>,
        };

        /// `AMDKFD_IOC_CROSS_MEMORY_COPY` — copy between VM ranges of two processes.
        AMDKFD_IOC_CROSS_MEMORY_COPY(0x83) {
            pid : u32,
            flags : u32,
            src_mem_range_array : Vec<KfdMemoryRange>,
            dst_mem_range_array : Vec<KfdMemoryRange>,
        } => {
            bytes_copied : u64,
        };

        /// `AMDKFD_IOC_RLC_SPM` — RLC Streaming Performance Monitor.
        AMDKFD_IOC_RLC_SPM(0x84) {
            op : KfdSpmOp,
            gpu_id : u32,
            dest_buf : u64,
            buf_size : u32,
            timeout : u32,
        } => {
            timeout : u32,
            bytes_copied : u32,
            has_data_loss : u32,
        };

        /// `AMDKFD_IOC_PC_SAMPLE` — program counter sampling interface.
        AMDKFD_IOC_PC_SAMPLE(0x85) {
            raw_args : Vec<u8>,
        } => {
            raw_args : Vec<u8>,
        };

        /// `AMDKFD_IOC_PROFILER` — per-device profiler control.
        AMDKFD_IOC_PROFILER(0x86) {
            op : KfdProfilerOp,
            raw_args : Vec<u8>,
        } => {
            raw_args : Vec<u8>,
        };

        /// `AMDKFD_IOC_AIS_OP` — AMD Infinity Storage direct I/O.
        AMDKFD_IOC_AIS_OP(0x87) {
            op : KfdAisOp,
            fd : i32,
            handle : u64,
            handle_offset : u64,
            file_offset : i64,
            size : u64,
        } => {
            size_copied : u64,
            status : i32,
        };
    }

    drm {
        /// `DRM_AMDGPU_GEM_CREATE` — allocate a GPU buffer object.
        DRM_AMDGPU_GEM_CREATE(0x00) {
            bo_size : u64,
            alignment : u64,
            domains : u64,
            domain_flags : u64,
        } => {
            handle : u32,
        };

        /// `DRM_AMDGPU_GEM_MMAP` — get the mmap offset for a buffer object.
        DRM_AMDGPU_GEM_MMAP(0x01) {
            handle : u32,
        } => {
            addr_ptr : u64,
        };

        /// `DRM_AMDGPU_CTX` — allocate, free, or query a GPU context.
        DRM_AMDGPU_CTX(0x02) {
            op : CtxOp,
            flags : u32,
            ctx_id : u32,
            priority : i32,
        } => {
            ctx_id : u32,
            flags : u64,
            hangs : u32,
            reset_status : u32,
        };

        /// `DRM_AMDGPU_BO_LIST` — create, destroy, or update a BO list.
        DRM_AMDGPU_BO_LIST(0x03) {
            operation : BoListOp,
            list_handle : u32,
            bo_info : Vec<BoListEntry>,
        } => {
            list_handle : u32,
        };

        /// `DRM_AMDGPU_CS` — submit GPU work.
        DRM_AMDGPU_CS(0x04) {
            ctx_id : u32,
            bo_list_handle : u32,
            flags : u32,
            chunks : Vec<CsChunk>,
        } => {
            handle : u64,
        };

        /// `DRM_AMDGPU_INFO` — query device information.
        DRM_AMDGPU_INFO(0x05) {
            query : u32,
            return_size : u32,
            sub_query : u32,
            sub_query2 : u32,
            sub_query3 : u32,
            flags : u32,
        } => {
            raw_data : Vec<u8>,
        };

        /// `DRM_AMDGPU_GEM_METADATA` — get or set BO metadata.
        DRM_AMDGPU_GEM_METADATA(0x06) {
            handle : u32,
            op : GemMetadataOp,
            flags : u64,
            tiling_info : u64,
            data : Vec<u8>,
        } => {
            flags : u64,
            tiling_info : u64,
            data : Vec<u8>,
        };

        /// `DRM_AMDGPU_GEM_WAIT_IDLE` — wait for a BO to become idle.
        DRM_AMDGPU_GEM_WAIT_IDLE(0x07) {
            handle : u32,
            flags : u32,
            timeout : u64,
        } => {
            status : u32,
            domain : u32,
        };

        /// `DRM_AMDGPU_GEM_VA` — map or unmap GPU virtual-address ranges.
        DRM_AMDGPU_GEM_VA(0x08) {
            handle : u32,
            operation : VaOp,
            flags : u32,
            va_address : u64,
            offset_in_bo : u64,
            map_size : u64,
            vm_timeline_point : u64,
            vm_timeline_syncobj_out : u32,
            num_syncobj_handles : u32,
            input_fence_syncobj_handles : Vec<u32>,
        } => {};

        /// `DRM_AMDGPU_WAIT_CS` — wait for a command submission to complete.
        DRM_AMDGPU_WAIT_CS(0x09) {
            handle : u64,
            timeout : u64,
            ip_type : u32,
            ip_instance : u32,
            ring : u32,
            ctx_id : u32,
        } => {
            status : u64,
        };

        /// `DRM_AMDGPU_GEM_OP` — get or set BO properties.
        DRM_AMDGPU_GEM_OP(0x10) {
            handle : u32,
            op : GemOpType,
            value : u64,
            num_entries : u32,
        } => {
            value : u64,
            raw_data : Vec<u8>,
            num_entries : u32,
        };

        /// `DRM_AMDGPU_GEM_USERPTR` — register userspace memory for GPU access.
        DRM_AMDGPU_GEM_USERPTR(0x11) {
            addr : u64,
            size : u64,
            flags : u32,
        } => {
            handle : u32,
        };

        /// `DRM_AMDGPU_WAIT_FENCES` — wait for one or more fences.
        DRM_AMDGPU_WAIT_FENCES(0x12) {
            fences : Vec<Fence>,
            wait_all : bool,
            timeout_ns : u64,
        } => {
            status : u32,
            first_signaled : u32,
        };

        /// `DRM_AMDGPU_VM` — reserve or unreserve a VMID.
        DRM_AMDGPU_VM(0x13) {
            op : VmOp,
            flags : u32,
        } => {
            flags : u64,
        };

        /// `DRM_AMDGPU_FENCE_TO_HANDLE` — convert a fence to a handle or fd.
        DRM_AMDGPU_FENCE_TO_HANDLE(0x14) {
            fence : Fence,
            what : FenceToHandleType,
        } => {
            handle : u32,
        };

        /// `DRM_AMDGPU_SCHED` — set scheduler priority.
        DRM_AMDGPU_SCHED(0x15) {
            op : SchedOp,
            fd : u32,
            priority : i32,
            ctx_id : u32,
        } => {};

        /// `DRM_AMDGPU_USERQ` — create or free a user-mode queue.
        DRM_AMDGPU_USERQ(0x16) {
            op : UserqOp,
            queue_id : u32,
            ip_type : u32,
            doorbell_handle : u32,
            doorbell_offset : u32,
            flags : u32,
            queue_va : u64,
            queue_size : u64,
            rptr_va : u64,
            wptr_va : u64,
            mqd : Vec<u8>,
        } => {
            queue_id : u32,
        };

        /// `DRM_AMDGPU_USERQ_SIGNAL` — signal sync objects from a user queue.
        DRM_AMDGPU_USERQ_SIGNAL(0x17) {
            queue_id : u32,
            syncobj_handles : Vec<u32>,
            bo_read_handles : Vec<u32>,
            bo_write_handles : Vec<u32>,
        } => {};

        /// `DRM_AMDGPU_USERQ_WAIT` — wait on sync objects from a user queue.
        DRM_AMDGPU_USERQ_WAIT(0x18) {
            waitq_id : u32,
            syncobj_handles : Vec<u32>,
            syncobj_timeline_handles : Vec<u32>,
            syncobj_timeline_points : Vec<u64>,
            bo_read_handles : Vec<u32>,
            bo_write_handles : Vec<u32>,
            max_fences : u16,
        } => {
            fences : Vec<UserqFenceInfo>,
        };

        /// `DRM_AMDGPU_GEM_LIST_HANDLES` — list process GEM handles.
        DRM_AMDGPU_GEM_LIST_HANDLES(0x19) {
            max_entries : u32,
        } => {
            entries : Vec<GemListHandlesEntry>,
            num_entries : u32,
        };

        /// `DRM_AMDGPU_SEM` — semaphore operations.
        DRM_AMDGPU_SEM(0x5B) {
            op : SemOp,
            handle : u32,
            ctx_id : u32,
            ip_type : u32,
            ip_instance : u32,
            ring : u32,
            seq : u64,
        } => {
            handle_or_fd : i32,
        };

        /// `DRM_AMDGPU_GEM_DGMA` — direct GMA operations.
        DRM_AMDGPU_GEM_DGMA(0x5C) {
            addr : u64,
            size : u64,
            op : DgmaOp,
            handle : u32,
        } => {
            addr : u64,
            handle : u32,
        };
    }
}
