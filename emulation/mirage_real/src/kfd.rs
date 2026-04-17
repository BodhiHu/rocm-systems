/*
use std::mem::size_of;

use mirage_schema::amdgpu::{
    AmdkfdIocAcquireVmRequest, AmdkfdIocAcquireVmResponse, AmdkfdIocAisOpRequest,
    AmdkfdIocAisOpResponse, AmdkfdIocAllocMemoryOfGpuRequest, AmdkfdIocAllocMemoryOfGpuResponse,
    AmdkfdIocAllocQueueGwsRequest, AmdkfdIocAllocQueueGwsResponse,
    AmdkfdIocAvailableMemoryRequest, AmdkfdIocAvailableMemoryResponse,
    AmdkfdIocCreateEventRequest, AmdkfdIocCreateEventResponse,
    AmdkfdIocCreateProcessRequest, AmdkfdIocCreateProcessResponse,
    AmdkfdIocCreateQueueRequest, AmdkfdIocCreateQueueResponse, AmdkfdIocCriuOpRequest,
    AmdkfdIocCriuOpResponse, AmdkfdIocCrossMemoryCopyRequest, AmdkfdIocCrossMemoryCopyResponse,
    AmdkfdIocDbgAddressWatchDeprecatedRequest, AmdkfdIocDbgAddressWatchDeprecatedResponse,
    AmdkfdIocDbgRegisterDeprecatedRequest, AmdkfdIocDbgRegisterDeprecatedResponse,
    AmdkfdIocDbgTrapRequest, AmdkfdIocDbgTrapResponse,
    AmdkfdIocDbgUnregisterDeprecatedRequest, AmdkfdIocDbgUnregisterDeprecatedResponse,
    AmdkfdIocDbgWaveControlDeprecatedRequest, AmdkfdIocDbgWaveControlDeprecatedResponse,
    AmdkfdIocDestroyEventRequest, AmdkfdIocDestroyEventResponse,
    AmdkfdIocDestroyQueueRequest, AmdkfdIocDestroyQueueResponse,
    AmdkfdIocExportDmabufRequest, AmdkfdIocExportDmabufResponse,
    AmdkfdIocFreeMemoryOfGpuRequest, AmdkfdIocFreeMemoryOfGpuResponse,
    AmdkfdIocGetClockCountersRequest, AmdkfdIocGetClockCountersResponse,
    AmdkfdIocGetDmabufInfoRequest, AmdkfdIocGetDmabufInfoResponse,
    AmdkfdIocGetProcessAperturesNewRequest, AmdkfdIocGetProcessAperturesNewResponse,
    AmdkfdIocGetProcessAperturesRequest, AmdkfdIocGetProcessAperturesResponse,
    AmdkfdIocGetQueueWaveStateRequest, AmdkfdIocGetQueueWaveStateResponse,
    AmdkfdIocGetVersionRequest, AmdkfdIocGetVersionResponse, AmdkfdIocGetTileConfigRequest,
    AmdkfdIocGetTileConfigResponse, AmdkfdIocImportDmabufRequest, AmdkfdIocImportDmabufResponse,
    AmdkfdIocIpcExportHandleRequest, AmdkfdIocIpcExportHandleResponse,
    AmdkfdIocIpcImportHandleRequest, AmdkfdIocIpcImportHandleResponse,
    AmdkfdIocMapMemoryToGpuRequest, AmdkfdIocMapMemoryToGpuResponse,
    AmdkfdIocPcSampleRequest, AmdkfdIocPcSampleResponse, AmdkfdIocProfilerRequest,
    AmdkfdIocProfilerResponse, AmdkfdIocResetEventRequest, AmdkfdIocResetEventResponse,
    AmdkfdIocRlcSpmRequest, AmdkfdIocRlcSpmResponse, AmdkfdIocRuntimeEnableRequest,
    AmdkfdIocRuntimeEnableResponse, AmdkfdIocSetCuMaskRequest, AmdkfdIocSetCuMaskResponse,
    AmdkfdIocSetEventRequest, AmdkfdIocSetEventResponse,
    AmdkfdIocSetMemoryPolicyRequest, AmdkfdIocSetMemoryPolicyResponse,
    AmdkfdIocSetScratchBackingVaRequest, AmdkfdIocSetScratchBackingVaResponse,
    AmdkfdIocSetTrapHandlerRequest, AmdkfdIocSetTrapHandlerResponse,
    AmdkfdIocSetXnackModeRequest, AmdkfdIocSetXnackModeResponse, AmdkfdIocSmiEventsRequest,
    AmdkfdIocSmiEventsResponse, AmdkfdIocSvmRequest, AmdkfdIocSvmResponse,
    AmdkfdIocUnmapMemoryFromGpuRequest, AmdkfdIocUnmapMemoryFromGpuResponse,
    AmdkfdIocUpdateQueueRequest, AmdkfdIocUpdateQueueResponse, AmdkfdIocWaitEventsRequest,
    AmdkfdIocWaitEventsResponse, HandleAnyKfdIoctl, HandleKfdIoctl, IoctlCtx,
    KfdAisOp, KfdCriuBoBucket, KfdCriuDeviceBucket, KfdDbgDeviceInfoEntry, KfdDbgTrapArgs,
    KfdDbgTrapOp, KfdEventData, KfdHwExceptionData, KfdMemoryExceptionData,
    KfdMemoryExceptionFailure, KfdMemoryRange, KfdPcSampleArgs, KfdPcSampleInfo,
    KfdPmcSettings, KfdProcessDeviceAperture, KfdProfilerArgs, KfdQueueSnapshotEntry,
    KfdSignalEventData, KfdSvmAttribute,
};
use mirage_schema::amdgpu_error::AmdgpuResult;
use mirage_uapi::kfd;

use crate::ioctl::{from_io_error, kfd_ior, kfd_iow, kfd_iowr, maybe_mut_ptr, maybe_ptr};
use crate::RealEmulator;

impl HandleKfdIoctl for RealEmulator {
    fn amdkfd_ioc_get_version(
        &self,
        _ctx: IoctlCtx,
        _request: AmdkfdIocGetVersionRequest,
    ) -> AmdgpuResult<AmdkfdIocGetVersionResponse> {
        self.kfd_get_version()
    }

    fn amdkfd_ioc_create_queue(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCreateQueueRequest,
    ) -> AmdgpuResult<AmdkfdIocCreateQueueResponse> {
        let mut args = kfd::kfd_ioctl_create_queue_args {
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_create_queue_args>(0x02), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_destroy_queue_args>(0x03), &mut args)? };
        Ok(AmdkfdIocDestroyQueueResponse {})
    }

    fn amdkfd_ioc_set_memory_policy(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetMemoryPolicyRequest,
    ) -> AmdgpuResult<AmdkfdIocSetMemoryPolicyResponse> {
        let mut args = kfd::kfd_ioctl_set_memory_policy_args {
            alternate_aperture_base: request.alternate_aperture_base,
            alternate_aperture_size: request.alternate_aperture_size,
            gpu_id: request.gpu_id,
            default_policy: request.default_policy as u32,
            alternate_policy: request.alternate_policy as u32,
            misc_process_flag: request.misc_process_flag,
        };
        unsafe {
            self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_memory_policy_args>(0x04), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_clock_counters_args>(0x05), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_ior::<kfd::kfd_ioctl_get_process_apertures_args>(0x06), &mut args)?
        };
        let count = args.num_of_nodes.min(args.process_apertures.len() as u32) as usize;
        Ok(AmdkfdIocGetProcessAperturesResponse {
            apertures: args.process_apertures[..count]
                .iter()
                .copied()
                .map(aperture_from_raw)
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_update_queue_args>(0x07), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_create_event_args>(0x08), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_destroy_event_args>(0x09), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_event_args>(0x0A), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_reset_event_args>(0x0B), &mut args)? };
        Ok(AmdkfdIocResetEventResponse {})
    }

    fn amdkfd_ioc_wait_events(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocWaitEventsRequest,
    ) -> AmdgpuResult<AmdkfdIocWaitEventsResponse> {
        let mut events: Vec<KfdEventDataRaw> = request.events.iter().map(event_to_raw).collect();
        let mut args = kfd::kfd_ioctl_wait_events_args {
            events_ptr: maybe_mut_ptr(&mut events),
            num_events: events.len() as u32,
            wait_for_all: request.wait_for_all as u32,
            timeout: request.timeout,
            ..Default::default()
        };
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_wait_events_args>(0x0C), &mut args)? };
        Ok(AmdkfdIocWaitEventsResponse {
            wait_result: args.wait_result,
            events: events.into_iter().map(event_from_raw).collect(),
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_register_args>(0x0D), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_unregister_args>(0x0E), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_address_watch_args>(0x0F), &mut args)?
        };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_wave_control_args>(0x10), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_set_scratch_backing_va_args>(0x11), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_tile_config_args>(0x12), &mut args)?
        };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_trap_handler_args>(0x13), &mut args)? };
        Ok(AmdkfdIocSetTrapHandlerResponse {})
    }

    fn amdkfd_ioc_get_process_apertures_new(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocGetProcessAperturesNewRequest,
    ) -> AmdgpuResult<AmdkfdIocGetProcessAperturesNewResponse> {
        let mut apertures = vec![kfd::kfd_process_device_apertures::default(); request.max_nodes as usize];
        let mut args = kfd::kfd_ioctl_get_process_apertures_new_args {
            kfd_process_device_apertures_ptr: maybe_mut_ptr(&mut apertures),
            num_of_nodes: apertures.len() as u32,
            ..Default::default()
        };
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_process_apertures_new_args>(0x14), &mut args)?
        };
        apertures.truncate(args.num_of_nodes as usize);
        Ok(AmdkfdIocGetProcessAperturesNewResponse {
            apertures: apertures.into_iter().map(aperture_from_raw).collect(),
        })
    }

    fn amdkfd_ioc_acquire_vm(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocAcquireVmRequest,
    ) -> AmdgpuResult<AmdkfdIocAcquireVmResponse> {
        let mut args = kfd::kfd_ioctl_acquire_vm_args {
            drm_fd: request.drm_fd,
            gpu_id: request.gpu_id,
        };
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_acquire_vm_args>(0x15), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_alloc_memory_of_gpu_args>(0x16), &mut args)?
        };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_free_memory_of_gpu_args>(0x17), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_map_memory_to_gpu_args>(0x18), &mut args)? };
        Ok(AmdkfdIocMapMemoryToGpuResponse { n_success: args.n_success })
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_unmap_memory_from_gpu_args>(0x19), &mut args)?
        };
        Ok(AmdkfdIocUnmapMemoryFromGpuResponse { n_success: args.n_success })
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_cu_mask_args>(0x1A), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_queue_wave_state_args>(0x1B), &mut args)?
        };
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_dmabuf_info_args>(0x1C), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_import_dmabuf_args>(0x1D), &mut args)? };
        Ok(AmdkfdIocImportDmabufResponse { handle: args.handle })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_alloc_queue_gws_args>(0x1E), &mut args)? };
        Ok(AmdkfdIocAllocQueueGwsResponse { first_gws: args.first_gws })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_smi_events_args>(0x1F), &mut args)? };
        Ok(AmdkfdIocSmiEventsResponse { anon_fd: args.anon_fd })
    }

    fn amdkfd_ioc_svm(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSvmRequest,
    ) -> AmdgpuResult<AmdkfdIocSvmResponse> {
        let mut attrs: Vec<kfd::kfd_ioctl_svm_attribute> = request
            .attrs
            .iter()
            .map(|attr| kfd::kfd_ioctl_svm_attribute {
                type_: attr.attr_type,
                value: attr.value,
            })
            .collect();
        let base_len = size_of::<kfd::kfd_ioctl_svm_args>();
        let total_len = base_len + attrs.len() * size_of::<kfd::kfd_ioctl_svm_attribute>();
        let mut storage = vec![0u8; total_len];
        let args = storage.as_mut_ptr() as *mut kfd::kfd_ioctl_svm_args;
        unsafe {
            (*args).start_addr = request.start_addr;
            (*args).size = request.size;
            (*args).op = request.op as u32;
            (*args).nattr = attrs.len() as u32;
            std::ptr::copy_nonoverlapping(
                attrs.as_ptr(),
                (*args).attrs.as_mut_ptr(),
                attrs.len(),
            );
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_svm_args>(0x20), &mut *args)?;
            attrs = (*args)
                .attrs
                .as_slice(attrs.len())
                .iter()
                .map(|attr| kfd::kfd_ioctl_svm_attribute {
                    type_: attr.type_,
                    value: attr.value,
                })
                .collect();
        }
        Ok(AmdkfdIocSvmResponse {
            attrs: attrs
                .into_iter()
                .map(|attr| KfdSvmAttribute {
                    attr_type: attr.type_,
                    value: attr.value,
                })
                .collect(),
        })
    }

    fn amdkfd_ioc_set_xnack_mode(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetXnackModeRequest,
    ) -> AmdgpuResult<AmdkfdIocSetXnackModeResponse> {
        let mut args = kfd::kfd_ioctl_set_xnack_mode_args {
            xnack_enabled: request.xnack_enabled,
        };
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_set_xnack_mode_args>(0x21), &mut args)? };
        Ok(AmdkfdIocSetXnackModeResponse { xnack_enabled: args.xnack_enabled })
    }

    fn amdkfd_ioc_criu_op(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCriuOpRequest,
    ) -> AmdgpuResult<AmdkfdIocCriuOpResponse> {
        let mut devices: Vec<KfdCriuDeviceBucketRaw> = request.devices.iter().copied().map(Into::into).collect();
        let mut bos: Vec<KfdCriuBoBucketRaw> = request.bos.iter().copied().map(Into::into).collect();
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_criu_args>(0x22), &mut args)? };
        devices.truncate(args.num_devices as usize);
        bos.truncate(args.num_bos as usize);
        priv_data.truncate(args.priv_data_size as usize);
        Ok(AmdkfdIocCriuOpResponse {
            num_devices: args.num_devices,
            num_bos: args.num_bos,
            num_objects: args.num_objects,
            priv_data_size: args.priv_data_size,
            pid: args.pid,
            devices: devices.into_iter().map(Into::into).collect(),
            bos: bos.into_iter().map(Into::into).collect(),
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_available_memory_args>(0x23), &mut args)?
        };
        Ok(AmdkfdIocAvailableMemoryResponse { available: args.available })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_export_dmabuf_args>(0x24), &mut args)? };
        Ok(AmdkfdIocExportDmabufResponse { dmabuf_fd: args.dmabuf_fd })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_runtime_enable_args>(0x25), &mut args)? };
        Ok(AmdkfdIocRuntimeEnableResponse {})
    }

    fn amdkfd_ioc_dbg_trap(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocDbgTrapRequest,
    ) -> AmdgpuResult<AmdkfdIocDbgTrapResponse> {
        let mut owned = DbgTrapOwned::default();
        let mut args = dbg_trap_to_raw(&request, &mut owned);
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_dbg_trap_args>(0x26), &mut args)? };
        Ok(AmdkfdIocDbgTrapResponse {
            args: dbg_trap_from_raw(request.op, &args, &owned),
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_create_process_args>(0x27), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_ipc_import_handle_args>(0x80), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_ipc_export_handle_args>(0x81), &mut args)?
        };
        Ok(AmdkfdIocIpcExportHandleResponse { share_handle: args.share_handle })
    }

    fn amdkfd_ioc_cross_memory_copy(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCrossMemoryCopyRequest,
    ) -> AmdgpuResult<AmdkfdIocCrossMemoryCopyResponse> {
        let mut src: Vec<KfdMemoryRangeRaw> = request.src_mem_range_array.iter().copied().map(Into::into).collect();
        let mut dst: Vec<KfdMemoryRangeRaw> = request.dst_mem_range_array.iter().copied().map(Into::into).collect();
        let mut args = kfd::kfd_ioctl_cross_memory_copy_args {
            pid: request.pid,
            flags: request.flags,
            src_mem_range_array: maybe_mut_ptr(&mut src),
            src_mem_array_size: src.len() as u64,
            dst_mem_range_array: maybe_mut_ptr(&mut dst),
            dst_mem_array_size: dst.len() as u64,
            ..Default::default()
        };
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_cross_memory_copy_args>(0x83), &mut args)?
        };
        Ok(AmdkfdIocCrossMemoryCopyResponse { bytes_copied: args.bytes_copied })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_spm_args>(0x84), &mut args)? };
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
            .copied()
            .map(Into::into)
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_pc_sample_args>(0x85), &mut args)? };
        sample_info.truncate(args.num_sample_info as usize);
        Ok(AmdkfdIocPcSampleResponse {
            args: KfdPcSampleArgs {
                sample_info: sample_info.into_iter().map(Into::into).collect(),
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
        let mut sample_info = Vec::new();
        let mut args = kfd::kfd_ioctl_profiler_args {
            op: request.op as u32,
            ..Default::default()
        };
        unsafe {
            args.__bindgen_anon_1 = profiler_args_to_raw(&request.args, &mut sample_info);
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_profiler_args>(0x86), &mut args)?;
        }
        Ok(AmdkfdIocProfilerResponse {
            args: profiler_args_from_raw(request.op, &args, &sample_info),
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_ais_args>(0x87), &mut args)? };
        let out = unsafe { args.__bindgen_anon_1.out };
        Ok(AmdkfdIocAisOpResponse {
            size_copied: out.size_copied,
            status: out.status,
        })
    }
}

impl HandleAnyKfdIoctl for RealEmulator {}

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
#[derive(Debug, Default, Copy, Clone)]
struct KfdSignalEventDataRaw {
    last_event_age: u64,
}

#[repr(C)]
union KfdEventUnionRaw {
    memory_exception_data: KfdMemoryExceptionDataRaw,
    hw_exception_data: KfdHwExceptionDataRaw,
    signal_event_data: KfdSignalEventDataRaw,
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
struct KfdEventDataRaw {
    payload: KfdEventUnionRaw,
    kfd_event_data_ext: u64,
    event_id: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdQueueSnapshotEntryRaw {
    exception_status: u64,
    ring_base_address: u64,
    write_pointer_address: u64,
    read_pointer_address: u64,
    ctx_save_restore_address: u64,
    queue_id: u32,
    gpu_id: u32,
    ring_size: u32,
    queue_type: u32,
    ctx_save_restore_area_size: u32,
    reserved: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdDbgDeviceInfoEntryRaw {
    exception_status: u64,
    lds_base: u64,
    lds_limit: u64,
    scratch_base: u64,
    scratch_limit: u64,
    gpuvm_base: u64,
    gpuvm_limit: u64,
    gpu_id: u32,
    location_id: u32,
    vendor_id: u32,
    device_id: u32,
    revision_id: u32,
    subsystem_vendor_id: u32,
    subsystem_device_id: u32,
    fw_version: u32,
    gfx_target_version: u32,
    simd_count: u32,
    max_waves_per_simd: u32,
    array_count: u32,
    simd_arrays_per_engine: u32,
    num_xcc: u32,
    capability: u32,
    debug_prop: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdCriuDeviceBucketRaw {
    user_gpu_id: u32,
    actual_gpu_id: u32,
    drm_fd: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdCriuBoBucketRaw {
    addr: u64,
    size: u64,
    offset: u64,
    restored_offset: u64,
    gpu_id: u32,
    alloc_flags: u32,
    dmabuf_fd: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdMemoryRangeRaw {
    va_addr: u64,
    size: u64,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdPcSampleInfoRaw {
    interval: u64,
    interval_min: u64,
    interval_max: u64,
    flags: u64,
    method: u32,
    type_: u32,
}

#[derive(Default)]
struct DbgTrapOwned {
    queue_ids: Vec<u32>,
    info: Vec<u8>,
    queue_snapshot: Vec<KfdQueueSnapshotEntryRaw>,
    device_snapshot: Vec<KfdDbgDeviceInfoEntryRaw>,
}

fn aperture_from_raw(raw: kfd::kfd_process_device_apertures) -> KfdProcessDeviceAperture {
    KfdProcessDeviceAperture {
        lds_base: raw.lds_base,
        lds_limit: raw.lds_limit,
        scratch_base: raw.scratch_base,
        scratch_limit: raw.scratch_limit,
        gpuvm_base: raw.gpuvm_base,
        gpuvm_limit: raw.gpuvm_limit,
        gpu_id: raw.gpu_id,
    }
}

fn event_to_raw(event: &KfdEventData) -> KfdEventDataRaw {
    let mut raw = KfdEventDataRaw {
        kfd_event_data_ext: event.kfd_event_data_ext,
        event_id: event.event_id,
        ..Default::default()
    };
    raw.payload = if let Some(signal) = event.signal_event_data {
        KfdEventUnionRaw {
            signal_event_data: KfdSignalEventDataRaw {
                last_event_age: signal.last_event_age,
            },
        }
    } else if let Some(hw) = event.hw_exception_data {
        KfdEventUnionRaw {
            hw_exception_data: KfdHwExceptionDataRaw {
                reset_type: hw.reset_type,
                reset_cause: hw.reset_cause,
                memory_lost: hw.memory_lost,
                gpu_id: hw.gpu_id,
            },
        }
    } else if let Some(memory) = event.memory_exception_data {
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

fn event_from_raw(raw: KfdEventDataRaw) -> KfdEventData {
    let signal = unsafe { raw.payload.signal_event_data };
    let hw = unsafe { raw.payload.hw_exception_data };
    let memory = unsafe { raw.payload.memory_exception_data };
    KfdEventData {
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
        hw_exception_data: (hw.reset_type != 0 || hw.reset_cause != 0 || hw.memory_lost != 0 || hw.gpu_id != 0)
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

impl From<KfdCriuDeviceBucket> for KfdCriuDeviceBucketRaw {
    fn from(value: KfdCriuDeviceBucket) -> Self {
        Self {
            user_gpu_id: value.user_gpu_id,
            actual_gpu_id: value.actual_gpu_id,
            drm_fd: value.drm_fd,
            pad: 0,
        }
    }
}

impl From<KfdCriuDeviceBucketRaw> for KfdCriuDeviceBucket {
    fn from(value: KfdCriuDeviceBucketRaw) -> Self {
        Self {
            user_gpu_id: value.user_gpu_id,
            actual_gpu_id: value.actual_gpu_id,
            drm_fd: value.drm_fd,
        }
    }
}

impl From<KfdCriuBoBucket> for KfdCriuBoBucketRaw {
    fn from(value: KfdCriuBoBucket) -> Self {
        Self {
            addr: value.addr,
            size: value.size,
            offset: value.offset,
            restored_offset: value.restored_offset,
            gpu_id: value.gpu_id,
            alloc_flags: value.alloc_flags,
            dmabuf_fd: value.dmabuf_fd,
            pad: 0,
        }
    }
}

impl From<KfdCriuBoBucketRaw> for KfdCriuBoBucket {
    fn from(value: KfdCriuBoBucketRaw) -> Self {
        Self {
            addr: value.addr,
            size: value.size,
            offset: value.offset,
            restored_offset: value.restored_offset,
            gpu_id: value.gpu_id,
            alloc_flags: value.alloc_flags,
            dmabuf_fd: value.dmabuf_fd,
        }
    }
}

impl From<KfdMemoryRange> for KfdMemoryRangeRaw {
    fn from(value: KfdMemoryRange) -> Self {
        Self {
            va_addr: value.va_addr,
            size: value.size,
        }
    }
}

impl From<KfdPcSampleInfo> for KfdPcSampleInfoRaw {
    fn from(value: KfdPcSampleInfo) -> Self {
        Self {
            interval: value.interval,
            interval_min: value.interval_min,
            interval_max: value.interval_max,
            flags: value.flags,
            method: value.method as u32,
            type_: value.sample_type as u32,
        }
    }
}

impl From<KfdPcSampleInfoRaw> for KfdPcSampleInfo {
    fn from(value: KfdPcSampleInfoRaw) -> Self {
        Self {
            interval: value.interval,
            interval_min: value.interval_min,
            interval_max: value.interval_max,
            flags: value.flags,
            method: unsafe { std::mem::transmute(value.method) },
            sample_type: unsafe { std::mem::transmute(value.type_) },
        }
    }
}

fn dbg_trap_to_raw(request: &AmdkfdIocDbgTrapRequest, owned: &mut DbgTrapOwned) -> kfd::kfd_ioctl_dbg_trap_args {
    let mut raw = kfd::kfd_ioctl_dbg_trap_args {
        pid: request.pid,
        op: request.op as u32,
        ..Default::default()
    };
    unsafe {
        raw.__bindgen_anon_1 = match request.op {
            KfdDbgTrapOp::Enable => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                enable: {
                    let enable = request.args.enable.unwrap_or_default();
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
                    let args = request.args.send_runtime_event.unwrap_or_default();
                    kfd::kfd_ioctl_dbg_trap_send_runtime_event_args {
                        exception_mask: args.exception_mask,
                        gpu_id: args.gpu_id,
                        queue_id: args.queue_id,
                    }
                },
            },
            KfdDbgTrapOp::SetExceptionsEnabled => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                set_exceptions_enabled: {
                    let args = request.args.set_exceptions_enabled.unwrap_or_default();
                    kfd::kfd_ioctl_dbg_trap_set_exceptions_enabled_args {
                        exception_mask: args.exception_mask,
                    }
                },
            },
            KfdDbgTrapOp::SetWaveLaunchOverride => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                launch_override: {
                    let args = request.args.launch_override.unwrap_or_default();
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
                    let args = request.args.launch_mode.unwrap_or_default();
                    kfd::kfd_ioctl_dbg_trap_set_wave_launch_mode_args {
                        launch_mode: args.launch_mode as u32,
                        pad: 0,
                    }
                },
            },
            KfdDbgTrapOp::SuspendQueues => {
                let args = request.args.suspend_queues.clone().unwrap_or_default();
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
                let args = request.args.resume_queues.clone().unwrap_or_default();
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
                    let args = request.args.set_node_address_watch.unwrap_or_default();
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
                    let args = request.args.clear_node_address_watch.unwrap_or_default();
                    kfd::kfd_ioctl_dbg_trap_clear_node_address_watch_args {
                        gpu_id: args.gpu_id,
                        id: args.id,
                    }
                },
            },
            KfdDbgTrapOp::SetFlags => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                set_flags: {
                    let args = request.args.set_flags.unwrap_or_default();
                    kfd::kfd_ioctl_dbg_trap_set_flags_args {
                        flags: args.flags,
                        pad: 0,
                    }
                },
            },
            KfdDbgTrapOp::QueryDebugEvent => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                query_debug_event: {
                    let args = request.args.query_debug_event.unwrap_or_default();
                    kfd::kfd_ioctl_dbg_trap_query_debug_event_args {
                        exception_mask: args.exception_mask,
                        gpu_id: args.gpu_id,
                        queue_id: args.queue_id,
                    }
                },
            },
            KfdDbgTrapOp::QueryExceptionInfo => {
                let args = request.args.query_exception_info.clone().unwrap_or_default();
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
                    mirage_schema::amdgpu::KfdDbgTrapQueueSnapshotArgs {
                        exception_mask: 0,
                        entries: Vec::new(),
                        num_queues: 0,
                        entry_size: 0,
                    },
                );
                owned.queue_snapshot = vec![KfdQueueSnapshotEntryRaw::default(); args.num_queues as usize];
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
                    mirage_schema::amdgpu::KfdDbgTrapDeviceSnapshotArgs {
                        exception_mask: 0,
                        entries: Vec::new(),
                        num_devices: 0,
                        entry_size: 0,
                    },
                );
                owned.device_snapshot = vec![KfdDbgDeviceInfoEntryRaw::default(); args.num_devices as usize];
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
    }
    raw
}

fn dbg_trap_from_raw(
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
                args.enable = Some(mirage_schema::amdgpu::KfdDbgTrapEnableArgs {
                    exception_mask: enable.exception_mask,
                    rinfo_size: enable.rinfo_size,
                    dbg_fd: enable.dbg_fd,
                });
            }
            KfdDbgTrapOp::SendRuntimeEvent => {
                let event = raw.__bindgen_anon_1.send_runtime_event;
                args.send_runtime_event = Some(mirage_schema::amdgpu::KfdDbgTrapSendRuntimeEventArgs {
                    exception_mask: event.exception_mask,
                    gpu_id: event.gpu_id,
                    queue_id: event.queue_id,
                });
            }
            KfdDbgTrapOp::SetExceptionsEnabled => {
                args.set_exceptions_enabled = Some(mirage_schema::amdgpu::KfdDbgTrapSetExceptionsEnabledArgs {
                    exception_mask: raw.__bindgen_anon_1.set_exceptions_enabled.exception_mask,
                });
            }
            KfdDbgTrapOp::SetWaveLaunchOverride => {
                let value = raw.__bindgen_anon_1.launch_override;
                args.launch_override = Some(mirage_schema::amdgpu::KfdDbgTrapSetWaveLaunchOverrideArgs {
                    override_mode: std::mem::transmute(value.override_mode),
                    enable_mask: value.enable_mask,
                    support_request_mask: value.support_request_mask,
                });
            }
            KfdDbgTrapOp::SetWaveLaunchMode => {
                args.launch_mode = Some(mirage_schema::amdgpu::KfdDbgTrapSetWaveLaunchModeArgs {
                    launch_mode: std::mem::transmute(raw.__bindgen_anon_1.launch_mode.launch_mode),
                });
            }
            KfdDbgTrapOp::SuspendQueues => {
                let value = raw.__bindgen_anon_1.suspend_queues;
                args.suspend_queues = Some(mirage_schema::amdgpu::KfdDbgTrapSuspendQueuesArgs {
                    exception_mask: value.exception_mask,
                    queue_ids: owned.queue_ids.clone(),
                    grace_period: value.grace_period,
                });
            }
            KfdDbgTrapOp::ResumeQueues => {
                args.resume_queues = Some(mirage_schema::amdgpu::KfdDbgTrapResumeQueuesArgs {
                    queue_ids: owned.queue_ids.clone(),
                });
            }
            KfdDbgTrapOp::SetNodeAddressWatch => {
                let value = raw.__bindgen_anon_1.set_node_address_watch;
                args.set_node_address_watch = Some(mirage_schema::amdgpu::KfdDbgTrapSetNodeAddressWatchArgs {
                    address: value.address,
                    mode: std::mem::transmute(value.mode),
                    mask: value.mask,
                    gpu_id: value.gpu_id,
                    id: value.id,
                });
            }
            KfdDbgTrapOp::ClearNodeAddressWatch => {
                let value = raw.__bindgen_anon_1.clear_node_address_watch;
                args.clear_node_address_watch = Some(mirage_schema::amdgpu::KfdDbgTrapClearNodeAddressWatchArgs {
                    gpu_id: value.gpu_id,
                    id: value.id,
                });
            }
            KfdDbgTrapOp::SetFlags => {
                args.set_flags = Some(mirage_schema::amdgpu::KfdDbgTrapSetFlagsArgs {
                    flags: raw.__bindgen_anon_1.set_flags.flags,
                });
            }
            KfdDbgTrapOp::QueryDebugEvent => {
                let value = raw.__bindgen_anon_1.query_debug_event;
                args.query_debug_event = Some(mirage_schema::amdgpu::KfdDbgTrapQueryDebugEventArgs {
                    exception_mask: value.exception_mask,
                    gpu_id: value.gpu_id,
                    queue_id: value.queue_id,
                });
            }
            KfdDbgTrapOp::QueryExceptionInfo => {
                let value = raw.__bindgen_anon_1.query_exception_info;
                args.query_exception_info = Some(mirage_schema::amdgpu::KfdDbgTrapQueryExceptionInfoArgs {
                    info: owned.info.clone(),
                    info_size: value.info_size,
                    source_id: value.source_id,
                    exception_code: value.exception_code,
                    clear_exception: value.clear_exception,
                });
            }
            KfdDbgTrapOp::GetQueueSnapshot => {
                let value = raw.__bindgen_anon_1.queue_snapshot;
                args.queue_snapshot = Some(mirage_schema::amdgpu::KfdDbgTrapQueueSnapshotArgs {
                    exception_mask: value.exception_mask,
                    entries: owned
                        .queue_snapshot
                        .iter()
                        .take(value.num_queues as usize)
                        .map(|entry| KfdQueueSnapshotEntry {
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
                        })
                        .collect(),
                    num_queues: value.num_queues,
                    entry_size: value.entry_size,
                });
            }
            KfdDbgTrapOp::GetDeviceSnapshot => {
                let value = raw.__bindgen_anon_1.device_snapshot;
                args.device_snapshot = Some(mirage_schema::amdgpu::KfdDbgTrapDeviceSnapshotArgs {
                    exception_mask: value.exception_mask,
                    entries: owned
                        .device_snapshot
                        .iter()
                        .take(value.num_devices as usize)
                        .map(|entry| KfdDbgDeviceInfoEntry {
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
                        })
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

unsafe fn profiler_args_to_raw(
    args: &KfdProfilerArgs,
    sample_info: &mut Vec<KfdPcSampleInfoRaw>,
) -> kfd::kfd_ioctl_profiler_args__bindgen_ty_1 {
    if let Some(pc_sample) = &args.pc_sample {
        *sample_info = pc_sample.sample_info.iter().copied().map(Into::into).collect();
        kfd::kfd_ioctl_profiler_args__bindgen_ty_1 {
            pc_sample: kfd::kfd_ioctl_pc_sample_args {
                sample_info_ptr: maybe_mut_ptr(sample_info),
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

fn profiler_args_from_raw(
    op: mirage_schema::amdgpu::KfdProfilerOp,
    args: &kfd::kfd_ioctl_profiler_args,
    sample_info: &[KfdPcSampleInfoRaw],
) -> KfdProfilerArgs {
    unsafe {
        match op {
            mirage_schema::amdgpu::KfdProfilerOp::Pmc => KfdProfilerArgs {
                pc_sample: None,
                pmc: Some(KfdPmcSettings {
                    gpu_id: args.__bindgen_anon_1.pmc.gpu_id,
                    lock: args.__bindgen_anon_1.pmc.lock,
                    perfcount_enable: args.__bindgen_anon_1.pmc.perfcount_enable,
                }),
                version: None,
            },
            mirage_schema::amdgpu::KfdProfilerOp::PcSample => KfdProfilerArgs {
                pc_sample: Some(KfdPcSampleArgs {
                    sample_info: sample_info.iter().copied().map(Into::into).collect(),
                    num_sample_info: args.__bindgen_anon_1.pc_sample.num_sample_info,
                    op: std::mem::transmute(args.__bindgen_anon_1.pc_sample.op),
                    gpu_id: args.__bindgen_anon_1.pc_sample.gpu_id,
                    trace_id: args.__bindgen_anon_1.pc_sample.trace_id,
                    flags: args.__bindgen_anon_1.pc_sample.flags,
                    reserved: args.__bindgen_anon_1.pc_sample.reserved,
                }),
                pmc: None,
                version: None,
            },
            mirage_schema::amdgpu::KfdProfilerOp::Version => KfdProfilerArgs {
                pc_sample: None,
                pmc: None,
                version: Some(args.__bindgen_anon_1.version),
            },
        }
    }
}
*/
use std::mem::size_of;

use mirage_schema::amdgpu::{
    AmdkfdIocAcquireVmRequest, AmdkfdIocAcquireVmResponse, AmdkfdIocAisOpRequest,
    AmdkfdIocAisOpResponse, AmdkfdIocAllocMemoryOfGpuRequest, AmdkfdIocAllocMemoryOfGpuResponse,
    AmdkfdIocAllocQueueGwsRequest, AmdkfdIocAllocQueueGwsResponse,
    AmdkfdIocAvailableMemoryRequest, AmdkfdIocAvailableMemoryResponse,
    AmdkfdIocCreateEventRequest, AmdkfdIocCreateEventResponse,
    AmdkfdIocCreateProcessRequest, AmdkfdIocCreateProcessResponse,
    AmdkfdIocCreateQueueRequest, AmdkfdIocCreateQueueResponse, AmdkfdIocCriuOpRequest,
    AmdkfdIocCriuOpResponse, AmdkfdIocCrossMemoryCopyRequest, AmdkfdIocCrossMemoryCopyResponse,
    AmdkfdIocDbgAddressWatchDeprecatedRequest, AmdkfdIocDbgAddressWatchDeprecatedResponse,
    AmdkfdIocDbgRegisterDeprecatedRequest, AmdkfdIocDbgRegisterDeprecatedResponse,
    AmdkfdIocDbgTrapRequest, AmdkfdIocDbgTrapResponse,
    AmdkfdIocDbgUnregisterDeprecatedRequest, AmdkfdIocDbgUnregisterDeprecatedResponse,
    AmdkfdIocDbgWaveControlDeprecatedRequest, AmdkfdIocDbgWaveControlDeprecatedResponse,
    AmdkfdIocDestroyEventRequest, AmdkfdIocDestroyEventResponse,
    AmdkfdIocDestroyQueueRequest, AmdkfdIocDestroyQueueResponse,
    AmdkfdIocExportDmabufRequest, AmdkfdIocExportDmabufResponse,
    AmdkfdIocFreeMemoryOfGpuRequest, AmdkfdIocFreeMemoryOfGpuResponse,
    AmdkfdIocGetClockCountersRequest, AmdkfdIocGetClockCountersResponse,
    AmdkfdIocGetDmabufInfoRequest, AmdkfdIocGetDmabufInfoResponse,
    AmdkfdIocGetProcessAperturesNewRequest, AmdkfdIocGetProcessAperturesNewResponse,
    AmdkfdIocGetProcessAperturesRequest, AmdkfdIocGetProcessAperturesResponse,
    AmdkfdIocGetQueueWaveStateRequest, AmdkfdIocGetQueueWaveStateResponse,
    AmdkfdIocGetVersionRequest, AmdkfdIocGetVersionResponse, AmdkfdIocGetTileConfigRequest,
    AmdkfdIocGetTileConfigResponse, AmdkfdIocImportDmabufRequest, AmdkfdIocImportDmabufResponse,
    AmdkfdIocIpcExportHandleRequest, AmdkfdIocIpcExportHandleResponse,
    AmdkfdIocIpcImportHandleRequest, AmdkfdIocIpcImportHandleResponse,
    AmdkfdIocMapMemoryToGpuRequest, AmdkfdIocMapMemoryToGpuResponse,
    AmdkfdIocPcSampleRequest, AmdkfdIocPcSampleResponse, AmdkfdIocProfilerRequest,
    AmdkfdIocProfilerResponse, AmdkfdIocResetEventRequest, AmdkfdIocResetEventResponse,
    AmdkfdIocRlcSpmRequest, AmdkfdIocRlcSpmResponse, AmdkfdIocRuntimeEnableRequest,
    AmdkfdIocRuntimeEnableResponse, AmdkfdIocSetCuMaskRequest, AmdkfdIocSetCuMaskResponse,
    AmdkfdIocSetEventRequest, AmdkfdIocSetEventResponse,
    AmdkfdIocSetMemoryPolicyRequest, AmdkfdIocSetMemoryPolicyResponse,
    AmdkfdIocSetScratchBackingVaRequest, AmdkfdIocSetScratchBackingVaResponse,
    AmdkfdIocSetTrapHandlerRequest, AmdkfdIocSetTrapHandlerResponse,
    AmdkfdIocSetXnackModeRequest, AmdkfdIocSetXnackModeResponse, AmdkfdIocSmiEventsRequest,
    AmdkfdIocSmiEventsResponse, AmdkfdIocSvmRequest, AmdkfdIocSvmResponse,
    AmdkfdIocUnmapMemoryFromGpuRequest, AmdkfdIocUnmapMemoryFromGpuResponse,
    AmdkfdIocUpdateQueueRequest, AmdkfdIocUpdateQueueResponse, AmdkfdIocWaitEventsRequest,
    AmdkfdIocWaitEventsResponse, HandleAnyKfdIoctl, HandleKfdIoctl, IoctlCtx,
    KfdCriuBoBucket, KfdCriuDeviceBucket, KfdDbgDeviceInfoEntry, KfdDbgTrapArgs,
    KfdDbgTrapOp, KfdEventData, KfdHwExceptionData, KfdMemoryExceptionData,
    KfdMemoryExceptionFailure, KfdMemoryRange, KfdPcSampleArgs, KfdPcSampleInfo,
    KfdPmcSettings, KfdProcessDeviceAperture, KfdProfilerArgs, KfdQueueSnapshotEntry,
    KfdSignalEventData, KfdSvmAttribute,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_uapi::kfd;

use crate::ioctl::{kfd_ior, kfd_iow, kfd_iowr, maybe_mut_ptr, maybe_ptr};
use crate::RealEmulator;

impl HandleKfdIoctl for RealEmulator {
    fn amdkfd_ioc_get_version(
        &self,
        _ctx: IoctlCtx,
        _request: AmdkfdIocGetVersionRequest,
    ) -> AmdgpuResult<AmdkfdIocGetVersionResponse> {
        self.kfd_get_version()
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
        unsafe { self.kfd_ioctl(kfd_iowr::<KfdCreateQueueArgsCompat>(0x02), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_destroy_queue_args>(0x03), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<KfdSetMemoryPolicyArgsCompat>(0x04), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_clock_counters_args>(0x05), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_ior::<kfd::kfd_ioctl_get_process_apertures_args>(0x06), &mut args)?
        };
        let count = args.num_of_nodes.min(args.process_apertures.len() as u32) as usize;
        Ok(AmdkfdIocGetProcessAperturesResponse {
            apertures: args.process_apertures[..count]
                .iter()
                .copied()
                .map(aperture_from_raw)
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_update_queue_args>(0x07), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_create_event_args>(0x08), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_destroy_event_args>(0x09), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_event_args>(0x0A), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_reset_event_args>(0x0B), &mut args)? };
        Ok(AmdkfdIocResetEventResponse {})
    }

    fn amdkfd_ioc_wait_events(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocWaitEventsRequest,
    ) -> AmdgpuResult<AmdkfdIocWaitEventsResponse> {
        let mut events: Vec<KfdEventDataRaw> = request.events.iter().map(event_to_raw).collect();
        let mut args = kfd::kfd_ioctl_wait_events_args {
            events_ptr: maybe_mut_ptr(&mut events),
            num_events: events.len() as u32,
            wait_for_all: request.wait_for_all as u32,
            timeout: request.timeout,
            ..Default::default()
        };
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_wait_events_args>(0x0C), &mut args)? };
        Ok(AmdkfdIocWaitEventsResponse {
            wait_result: args.wait_result,
            events: events.into_iter().map(event_from_raw).collect(),
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_register_args>(0x0D), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_unregister_args>(0x0E), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_address_watch_args>(0x0F), &mut args)?
        };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_wave_control_args>(0x10), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_set_scratch_backing_va_args>(0x11), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_tile_config_args>(0x12), &mut args)?
        };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_trap_handler_args>(0x13), &mut args)? };
        Ok(AmdkfdIocSetTrapHandlerResponse {})
    }

    fn amdkfd_ioc_get_process_apertures_new(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocGetProcessAperturesNewRequest,
    ) -> AmdgpuResult<AmdkfdIocGetProcessAperturesNewResponse> {
        let mut apertures = vec![kfd::kfd_process_device_apertures::default(); request.max_nodes as usize];
        let mut args = kfd::kfd_ioctl_get_process_apertures_new_args {
            kfd_process_device_apertures_ptr: maybe_mut_ptr(&mut apertures),
            num_of_nodes: apertures.len() as u32,
            ..Default::default()
        };
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_process_apertures_new_args>(0x14), &mut args)?
        };
        apertures.truncate(args.num_of_nodes as usize);
        Ok(AmdkfdIocGetProcessAperturesNewResponse {
            apertures: apertures.into_iter().map(aperture_from_raw).collect(),
        })
    }

    fn amdkfd_ioc_acquire_vm(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocAcquireVmRequest,
    ) -> AmdgpuResult<AmdkfdIocAcquireVmResponse> {
        // The client's drm_fd is a virtual fd — look up our own real
        // DRM render fd for this GPU.
        let real_drm_fd = self.find_render_fd_for_gpu(request.gpu_id)
            .unwrap_or_else(|| {
                self.primary_render_fd().unwrap_or(-1)
            });
        let mut args = kfd::kfd_ioctl_acquire_vm_args {
            drm_fd: real_drm_fd as u32,
            gpu_id: request.gpu_id,
        };
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_acquire_vm_args>(0x15), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_alloc_memory_of_gpu_args>(0x16), &mut args)?
        };
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_free_memory_of_gpu_args>(0x17), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_map_memory_to_gpu_args>(0x18), &mut args)? };
        Ok(AmdkfdIocMapMemoryToGpuResponse { n_success: args.n_success })
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_unmap_memory_from_gpu_args>(0x19), &mut args)?
        };
        Ok(AmdkfdIocUnmapMemoryFromGpuResponse { n_success: args.n_success })
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
        unsafe { self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_cu_mask_args>(0x1A), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_queue_wave_state_args>(0x1B), &mut args)?
        };
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_dmabuf_info_args>(0x1C), &mut args)? };
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_import_dmabuf_args>(0x1D), &mut args)? };
        Ok(AmdkfdIocImportDmabufResponse { handle: args.handle })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_alloc_queue_gws_args>(0x1E), &mut args)? };
        Ok(AmdkfdIocAllocQueueGwsResponse { first_gws: args.first_gws })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_smi_events_args>(0x1F), &mut args)? };
        Ok(AmdkfdIocSmiEventsResponse { anon_fd: args.anon_fd })
    }

    fn amdkfd_ioc_svm(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSvmRequest,
    ) -> AmdgpuResult<AmdkfdIocSvmResponse> {
        let mut attrs: Vec<kfd::kfd_ioctl_svm_attribute> = request
            .attrs
            .iter()
            .map(|attr| kfd::kfd_ioctl_svm_attribute {
                type_: attr.attr_type,
                value: attr.value,
            })
            .collect();
        let base_len = size_of::<kfd::kfd_ioctl_svm_args>();
        let total_len = base_len + attrs.len() * size_of::<kfd::kfd_ioctl_svm_attribute>();
        let mut storage = vec![0u8; total_len];
        let args = storage.as_mut_ptr() as *mut kfd::kfd_ioctl_svm_args;
        unsafe {
            (*args).start_addr = request.start_addr;
            (*args).size = request.size;
            (*args).op = request.op as u32;
            (*args).nattr = attrs.len() as u32;
            std::ptr::copy_nonoverlapping(
                attrs.as_ptr(),
                (*args).attrs.as_mut_ptr(),
                attrs.len(),
            );
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_svm_args>(0x20), &mut *args)?;
            attrs = (*args)
                .attrs
                .as_slice(attrs.len())
                .iter()
                .map(|attr| kfd::kfd_ioctl_svm_attribute {
                    type_: attr.type_,
                    value: attr.value,
                })
                .collect();
        }
        Ok(AmdkfdIocSvmResponse {
            attrs: attrs
                .into_iter()
                .map(|attr| KfdSvmAttribute {
                    attr_type: attr.type_,
                    value: attr.value,
                })
                .collect(),
        })
    }

    fn amdkfd_ioc_set_xnack_mode(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocSetXnackModeRequest,
    ) -> AmdgpuResult<AmdkfdIocSetXnackModeResponse> {
        let mut args = kfd::kfd_ioctl_set_xnack_mode_args {
            xnack_enabled: request.xnack_enabled,
        };
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_set_xnack_mode_args>(0x21), &mut args)? };
        Ok(AmdkfdIocSetXnackModeResponse { xnack_enabled: args.xnack_enabled })
    }

    fn amdkfd_ioc_criu_op(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCriuOpRequest,
    ) -> AmdgpuResult<AmdkfdIocCriuOpResponse> {
        let mut devices: Vec<KfdCriuDeviceBucketRaw> = request.devices.iter().copied().map(Into::into).collect();
        let mut bos: Vec<KfdCriuBoBucketRaw> = request.bos.iter().copied().map(Into::into).collect();
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_criu_args>(0x22), &mut args)? };
        devices.truncate(args.num_devices as usize);
        bos.truncate(args.num_bos as usize);
        priv_data.truncate(args.priv_data_size as usize);
        Ok(AmdkfdIocCriuOpResponse {
            num_devices: args.num_devices,
            num_bos: args.num_bos,
            num_objects: args.num_objects,
            priv_data_size: args.priv_data_size,
            pid: args.pid,
            devices: devices.into_iter().map(Into::into).collect(),
            bos: bos.into_iter().map(Into::into).collect(),
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_get_available_memory_args>(0x23), &mut args)?
        };
        Ok(AmdkfdIocAvailableMemoryResponse { available: args.available })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_export_dmabuf_args>(0x24), &mut args)? };
        Ok(AmdkfdIocExportDmabufResponse { dmabuf_fd: args.dmabuf_fd })
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
        match unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_runtime_enable_args>(0x25), &mut args) } {
            Ok(()) => Ok(AmdkfdIocRuntimeEnableResponse {}),
            // EBUSY means runtime is already enabled for this KFD process;
            // treat as success when proxying for a remote client that shares
            // the daemon's process context.
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
        let mut args = dbg_trap_to_raw(&request, &mut owned);
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_dbg_trap_args>(0x26), &mut args)? };
        Ok(AmdkfdIocDbgTrapResponse {
            args: dbg_trap_from_raw(request.op, &args, &owned),
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_create_process_args>(0x27), &mut args)? };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_ipc_import_handle_args>(0x80), &mut args)?
        };
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
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_ipc_export_handle_args>(0x81), &mut args)?
        };
        Ok(AmdkfdIocIpcExportHandleResponse { share_handle: args.share_handle })
    }

    fn amdkfd_ioc_cross_memory_copy(
        &self,
        _ctx: IoctlCtx,
        request: AmdkfdIocCrossMemoryCopyRequest,
    ) -> AmdgpuResult<AmdkfdIocCrossMemoryCopyResponse> {
        let mut src: Vec<KfdMemoryRangeRaw> = request.src_mem_range_array.iter().copied().map(Into::into).collect();
        let mut dst: Vec<KfdMemoryRangeRaw> = request.dst_mem_range_array.iter().copied().map(Into::into).collect();
        let mut args = kfd::kfd_ioctl_cross_memory_copy_args {
            pid: request.pid,
            flags: request.flags,
            src_mem_range_array: maybe_mut_ptr(&mut src),
            src_mem_array_size: src.len() as u64,
            dst_mem_range_array: maybe_mut_ptr(&mut dst),
            dst_mem_array_size: dst.len() as u64,
            ..Default::default()
        };
        unsafe {
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_cross_memory_copy_args>(0x83), &mut args)?
        };
        Ok(AmdkfdIocCrossMemoryCopyResponse { bytes_copied: args.bytes_copied })
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_spm_args>(0x84), &mut args)? };
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
            .copied()
            .map(Into::into)
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_pc_sample_args>(0x85), &mut args)? };
        sample_info.truncate(args.num_sample_info as usize);
        Ok(AmdkfdIocPcSampleResponse {
            args: KfdPcSampleArgs {
                sample_info: sample_info.into_iter().map(Into::into).collect(),
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
        let mut sample_info = Vec::new();
        let mut args = kfd::kfd_ioctl_profiler_args {
            op: request.op as u32,
            ..Default::default()
        };
        unsafe {
            args.__bindgen_anon_1 = profiler_args_to_raw(&request.args, &mut sample_info);
            self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_profiler_args>(0x86), &mut args)?;
        }
        Ok(AmdkfdIocProfilerResponse {
            args: profiler_args_from_raw(request.op, &args, &sample_info),
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
        unsafe { self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_ais_args>(0x87), &mut args)? };
        let out = unsafe { args.__bindgen_anon_1.out };
        Ok(AmdkfdIocAisOpResponse {
            size_copied: out.size_copied,
            status: out.status,
        })
    }
}

impl HandleAnyKfdIoctl for RealEmulator {}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdCreateQueueArgsCompat {
    ring_base_address: u64,
    write_pointer_address: u64,
    read_pointer_address: u64,
    doorbell_offset: u64,
    ring_size: u32,
    gpu_id: u32,
    queue_type: u32,
    queue_percentage: u32,
    queue_priority: u32,
    queue_id: u32,
    eop_buffer_address: u64,
    eop_buffer_size: u64,
    ctx_save_restore_address: u64,
    ctx_save_restore_size: u32,
    ctl_stack_size: u32,
    sdma_engine_id: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdSetMemoryPolicyArgsCompat {
    alternate_aperture_base: u64,
    alternate_aperture_size: u64,
    gpu_id: u32,
    default_policy: u32,
    alternate_policy: u32,
    misc_process_flag: u32,
    pad: u32,
}

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
#[derive(Debug, Default, Copy, Clone)]
struct KfdSignalEventDataRaw {
    last_event_age: u64,
}

#[repr(C)]
union KfdEventUnionRaw {
    memory_exception_data: KfdMemoryExceptionDataRaw,
    hw_exception_data: KfdHwExceptionDataRaw,
    signal_event_data: KfdSignalEventDataRaw,
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
struct KfdEventDataRaw {
    payload: KfdEventUnionRaw,
    kfd_event_data_ext: u64,
    event_id: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdQueueSnapshotEntryRaw {
    exception_status: u64,
    ring_base_address: u64,
    write_pointer_address: u64,
    read_pointer_address: u64,
    ctx_save_restore_address: u64,
    queue_id: u32,
    gpu_id: u32,
    ring_size: u32,
    queue_type: u32,
    ctx_save_restore_area_size: u32,
    reserved: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdDbgDeviceInfoEntryRaw {
    exception_status: u64,
    lds_base: u64,
    lds_limit: u64,
    scratch_base: u64,
    scratch_limit: u64,
    gpuvm_base: u64,
    gpuvm_limit: u64,
    gpu_id: u32,
    location_id: u32,
    vendor_id: u32,
    device_id: u32,
    revision_id: u32,
    subsystem_vendor_id: u32,
    subsystem_device_id: u32,
    fw_version: u32,
    gfx_target_version: u32,
    simd_count: u32,
    max_waves_per_simd: u32,
    array_count: u32,
    simd_arrays_per_engine: u32,
    num_xcc: u32,
    capability: u32,
    debug_prop: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdCriuDeviceBucketRaw {
    user_gpu_id: u32,
    actual_gpu_id: u32,
    drm_fd: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdCriuBoBucketRaw {
    addr: u64,
    size: u64,
    offset: u64,
    restored_offset: u64,
    gpu_id: u32,
    alloc_flags: u32,
    dmabuf_fd: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdMemoryRangeRaw {
    va_addr: u64,
    size: u64,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
struct KfdPcSampleInfoRaw {
    interval: u64,
    interval_min: u64,
    interval_max: u64,
    flags: u64,
    method: u32,
    type_: u32,
}

#[derive(Default)]
struct DbgTrapOwned {
    queue_ids: Vec<u32>,
    info: Vec<u8>,
    queue_snapshot: Vec<KfdQueueSnapshotEntryRaw>,
    device_snapshot: Vec<KfdDbgDeviceInfoEntryRaw>,
}

fn aperture_from_raw(raw: kfd::kfd_process_device_apertures) -> KfdProcessDeviceAperture {
    KfdProcessDeviceAperture {
        lds_base: raw.lds_base,
        lds_limit: raw.lds_limit,
        scratch_base: raw.scratch_base,
        scratch_limit: raw.scratch_limit,
        gpuvm_base: raw.gpuvm_base,
        gpuvm_limit: raw.gpuvm_limit,
        gpu_id: raw.gpu_id,
    }
}

fn event_to_raw(event: &KfdEventData) -> KfdEventDataRaw {
    let mut raw = KfdEventDataRaw {
        kfd_event_data_ext: event.kfd_event_data_ext,
        event_id: event.event_id,
        ..Default::default()
    };
    raw.payload = if let Some(signal) = event.signal_event_data {
        KfdEventUnionRaw {
            signal_event_data: KfdSignalEventDataRaw {
                last_event_age: signal.last_event_age,
            },
        }
    } else if let Some(hw) = event.hw_exception_data {
        KfdEventUnionRaw {
            hw_exception_data: KfdHwExceptionDataRaw {
                reset_type: hw.reset_type,
                reset_cause: hw.reset_cause,
                memory_lost: hw.memory_lost,
                gpu_id: hw.gpu_id,
            },
        }
    } else if let Some(memory) = event.memory_exception_data {
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

fn event_from_raw(raw: KfdEventDataRaw) -> KfdEventData {
    let signal = unsafe { raw.payload.signal_event_data };
    let hw = unsafe { raw.payload.hw_exception_data };
    let memory = unsafe { raw.payload.memory_exception_data };
    KfdEventData {
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
        hw_exception_data: (hw.reset_type != 0 || hw.reset_cause != 0 || hw.memory_lost != 0 || hw.gpu_id != 0)
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

impl From<KfdCriuDeviceBucket> for KfdCriuDeviceBucketRaw {
    fn from(value: KfdCriuDeviceBucket) -> Self {
        Self {
            user_gpu_id: value.user_gpu_id,
            actual_gpu_id: value.actual_gpu_id,
            drm_fd: value.drm_fd,
            pad: 0,
        }
    }
}

impl From<KfdCriuDeviceBucketRaw> for KfdCriuDeviceBucket {
    fn from(value: KfdCriuDeviceBucketRaw) -> Self {
        Self {
            user_gpu_id: value.user_gpu_id,
            actual_gpu_id: value.actual_gpu_id,
            drm_fd: value.drm_fd,
        }
    }
}

impl From<KfdCriuBoBucket> for KfdCriuBoBucketRaw {
    fn from(value: KfdCriuBoBucket) -> Self {
        Self {
            addr: value.addr,
            size: value.size,
            offset: value.offset,
            restored_offset: value.restored_offset,
            gpu_id: value.gpu_id,
            alloc_flags: value.alloc_flags,
            dmabuf_fd: value.dmabuf_fd,
            pad: 0,
        }
    }
}

impl From<KfdCriuBoBucketRaw> for KfdCriuBoBucket {
    fn from(value: KfdCriuBoBucketRaw) -> Self {
        Self {
            addr: value.addr,
            size: value.size,
            offset: value.offset,
            restored_offset: value.restored_offset,
            gpu_id: value.gpu_id,
            alloc_flags: value.alloc_flags,
            dmabuf_fd: value.dmabuf_fd,
        }
    }
}

impl From<KfdMemoryRange> for KfdMemoryRangeRaw {
    fn from(value: KfdMemoryRange) -> Self {
        Self {
            va_addr: value.va_addr,
            size: value.size,
        }
    }
}

impl From<KfdPcSampleInfo> for KfdPcSampleInfoRaw {
    fn from(value: KfdPcSampleInfo) -> Self {
        Self {
            interval: value.interval,
            interval_min: value.interval_min,
            interval_max: value.interval_max,
            flags: value.flags,
            method: value.method as u32,
            type_: value.sample_type as u32,
        }
    }
}

impl From<KfdPcSampleInfoRaw> for KfdPcSampleInfo {
    fn from(value: KfdPcSampleInfoRaw) -> Self {
        Self {
            interval: value.interval,
            interval_min: value.interval_min,
            interval_max: value.interval_max,
            flags: value.flags,
            method: unsafe { std::mem::transmute(value.method) },
            sample_type: unsafe { std::mem::transmute(value.type_) },
        }
    }
}

fn dbg_trap_to_raw(request: &AmdkfdIocDbgTrapRequest, owned: &mut DbgTrapOwned) -> kfd::kfd_ioctl_dbg_trap_args {
    let mut raw = kfd::kfd_ioctl_dbg_trap_args {
        pid: request.pid,
        op: request.op as u32,
        ..Default::default()
    };
    raw.__bindgen_anon_1 = match request.op {
            KfdDbgTrapOp::Enable => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                enable: {
                    let enable = request.args.enable.unwrap_or(mirage_schema::amdgpu::KfdDbgTrapEnableArgs {
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
                        mirage_schema::amdgpu::KfdDbgTrapSendRuntimeEventArgs {
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
                        mirage_schema::amdgpu::KfdDbgTrapSetExceptionsEnabledArgs {
                            exception_mask: 0,
                        },
                    );
                    kfd::kfd_ioctl_dbg_trap_set_exceptions_enabled_args {
                        exception_mask: args.exception_mask,
                    }
                },
            },
            KfdDbgTrapOp::SetWaveLaunchOverride => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                launch_override: {
                    let args = request.args.launch_override.unwrap_or(
                        mirage_schema::amdgpu::KfdDbgTrapSetWaveLaunchOverrideArgs {
                            override_mode: mirage_schema::amdgpu::KfdDbgTrapOverrideMode::Or,
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
                        mirage_schema::amdgpu::KfdDbgTrapSetWaveLaunchModeArgs {
                            launch_mode: mirage_schema::amdgpu::KfdDbgTrapWaveLaunchMode::Normal,
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
                    mirage_schema::amdgpu::KfdDbgTrapSuspendQueuesArgs {
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
                    mirage_schema::amdgpu::KfdDbgTrapResumeQueuesArgs { queue_ids: Vec::new() },
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
                        mirage_schema::amdgpu::KfdDbgTrapSetNodeAddressWatchArgs {
                            address: 0,
                            mode: mirage_schema::amdgpu::KfdDbgTrapAddressWatchMode::Read,
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
                        mirage_schema::amdgpu::KfdDbgTrapClearNodeAddressWatchArgs { gpu_id: 0, id: 0 },
                    );
                    kfd::kfd_ioctl_dbg_trap_clear_node_address_watch_args {
                        gpu_id: args.gpu_id,
                        id: args.id,
                    }
                },
            },
            KfdDbgTrapOp::SetFlags => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                set_flags: {
                    let args = request.args.set_flags.unwrap_or(
                        mirage_schema::amdgpu::KfdDbgTrapSetFlagsArgs { flags: 0 },
                    );
                    kfd::kfd_ioctl_dbg_trap_set_flags_args {
                        flags: args.flags,
                        pad: 0,
                    }
                },
            },
            KfdDbgTrapOp::QueryDebugEvent => kfd::kfd_ioctl_dbg_trap_args__bindgen_ty_1 {
                query_debug_event: {
                    let args = request.args.query_debug_event.unwrap_or(
                        mirage_schema::amdgpu::KfdDbgTrapQueryDebugEventArgs {
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
                    mirage_schema::amdgpu::KfdDbgTrapQueryExceptionInfoArgs {
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
                    mirage_schema::amdgpu::KfdDbgTrapQueueSnapshotArgs {
                        exception_mask: 0,
                        entries: Vec::new(),
                        num_queues: 0,
                        entry_size: 0,
                    },
                );
                owned.queue_snapshot = vec![KfdQueueSnapshotEntryRaw::default(); args.num_queues as usize];
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
                    mirage_schema::amdgpu::KfdDbgTrapDeviceSnapshotArgs {
                        exception_mask: 0,
                        entries: Vec::new(),
                        num_devices: 0,
                        entry_size: 0,
                    },
                );
                owned.device_snapshot = vec![KfdDbgDeviceInfoEntryRaw::default(); args.num_devices as usize];
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

fn dbg_trap_from_raw(
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
                args.enable = Some(mirage_schema::amdgpu::KfdDbgTrapEnableArgs {
                    exception_mask: enable.exception_mask,
                    rinfo_size: enable.rinfo_size,
                    dbg_fd: enable.dbg_fd,
                });
            }
            KfdDbgTrapOp::SendRuntimeEvent => {
                let event = raw.__bindgen_anon_1.send_runtime_event;
                args.send_runtime_event = Some(mirage_schema::amdgpu::KfdDbgTrapSendRuntimeEventArgs {
                    exception_mask: event.exception_mask,
                    gpu_id: event.gpu_id,
                    queue_id: event.queue_id,
                });
            }
            KfdDbgTrapOp::SetExceptionsEnabled => {
                args.set_exceptions_enabled = Some(mirage_schema::amdgpu::KfdDbgTrapSetExceptionsEnabledArgs {
                    exception_mask: raw.__bindgen_anon_1.set_exceptions_enabled.exception_mask,
                });
            }
            KfdDbgTrapOp::SetWaveLaunchOverride => {
                let value = raw.__bindgen_anon_1.launch_override;
                args.launch_override = Some(mirage_schema::amdgpu::KfdDbgTrapSetWaveLaunchOverrideArgs {
                    override_mode: std::mem::transmute(value.override_mode),
                    enable_mask: value.enable_mask,
                    support_request_mask: value.support_request_mask,
                });
            }
            KfdDbgTrapOp::SetWaveLaunchMode => {
                args.launch_mode = Some(mirage_schema::amdgpu::KfdDbgTrapSetWaveLaunchModeArgs {
                    launch_mode: std::mem::transmute(raw.__bindgen_anon_1.launch_mode.launch_mode),
                });
            }
            KfdDbgTrapOp::SuspendQueues => {
                let value = raw.__bindgen_anon_1.suspend_queues;
                args.suspend_queues = Some(mirage_schema::amdgpu::KfdDbgTrapSuspendQueuesArgs {
                    exception_mask: value.exception_mask,
                    queue_ids: owned.queue_ids.clone(),
                    grace_period: value.grace_period,
                });
            }
            KfdDbgTrapOp::ResumeQueues => {
                args.resume_queues = Some(mirage_schema::amdgpu::KfdDbgTrapResumeQueuesArgs {
                    queue_ids: owned.queue_ids.clone(),
                });
            }
            KfdDbgTrapOp::SetNodeAddressWatch => {
                let value = raw.__bindgen_anon_1.set_node_address_watch;
                args.set_node_address_watch = Some(mirage_schema::amdgpu::KfdDbgTrapSetNodeAddressWatchArgs {
                    address: value.address,
                    mode: std::mem::transmute(value.mode),
                    mask: value.mask,
                    gpu_id: value.gpu_id,
                    id: value.id,
                });
            }
            KfdDbgTrapOp::ClearNodeAddressWatch => {
                let value = raw.__bindgen_anon_1.clear_node_address_watch;
                args.clear_node_address_watch = Some(mirage_schema::amdgpu::KfdDbgTrapClearNodeAddressWatchArgs {
                    gpu_id: value.gpu_id,
                    id: value.id,
                });
            }
            KfdDbgTrapOp::SetFlags => {
                args.set_flags = Some(mirage_schema::amdgpu::KfdDbgTrapSetFlagsArgs {
                    flags: raw.__bindgen_anon_1.set_flags.flags,
                });
            }
            KfdDbgTrapOp::QueryDebugEvent => {
                let value = raw.__bindgen_anon_1.query_debug_event;
                args.query_debug_event = Some(mirage_schema::amdgpu::KfdDbgTrapQueryDebugEventArgs {
                    exception_mask: value.exception_mask,
                    gpu_id: value.gpu_id,
                    queue_id: value.queue_id,
                });
            }
            KfdDbgTrapOp::QueryExceptionInfo => {
                let value = raw.__bindgen_anon_1.query_exception_info;
                args.query_exception_info = Some(mirage_schema::amdgpu::KfdDbgTrapQueryExceptionInfoArgs {
                    info: owned.info.clone(),
                    info_size: value.info_size,
                    source_id: value.source_id,
                    exception_code: value.exception_code,
                    clear_exception: value.clear_exception,
                });
            }
            KfdDbgTrapOp::GetQueueSnapshot => {
                let value = raw.__bindgen_anon_1.queue_snapshot;
                args.queue_snapshot = Some(mirage_schema::amdgpu::KfdDbgTrapQueueSnapshotArgs {
                    exception_mask: value.exception_mask,
                    entries: owned
                        .queue_snapshot
                        .iter()
                        .take(value.num_queues as usize)
                        .map(|entry| KfdQueueSnapshotEntry {
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
                        })
                        .collect(),
                    num_queues: value.num_queues,
                    entry_size: value.entry_size,
                });
            }
            KfdDbgTrapOp::GetDeviceSnapshot => {
                let value = raw.__bindgen_anon_1.device_snapshot;
                args.device_snapshot = Some(mirage_schema::amdgpu::KfdDbgTrapDeviceSnapshotArgs {
                    exception_mask: value.exception_mask,
                    entries: owned
                        .device_snapshot
                        .iter()
                        .take(value.num_devices as usize)
                        .map(|entry| KfdDbgDeviceInfoEntry {
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
                        })
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

unsafe fn profiler_args_to_raw(
    args: &KfdProfilerArgs,
    sample_info: &mut Vec<KfdPcSampleInfoRaw>,
) -> kfd::kfd_ioctl_profiler_args__bindgen_ty_1 {
    if let Some(pc_sample) = &args.pc_sample {
        *sample_info = pc_sample.sample_info.iter().copied().map(Into::into).collect();
        kfd::kfd_ioctl_profiler_args__bindgen_ty_1 {
            pc_sample: kfd::kfd_ioctl_pc_sample_args {
                sample_info_ptr: maybe_mut_ptr(sample_info),
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

fn profiler_args_from_raw(
    op: mirage_schema::amdgpu::KfdProfilerOp,
    args: &kfd::kfd_ioctl_profiler_args,
    sample_info: &[KfdPcSampleInfoRaw],
) -> KfdProfilerArgs {
    unsafe {
        match op {
            mirage_schema::amdgpu::KfdProfilerOp::Pmc => KfdProfilerArgs {
                pc_sample: None,
                pmc: Some(KfdPmcSettings {
                    gpu_id: args.__bindgen_anon_1.pmc.gpu_id,
                    lock: args.__bindgen_anon_1.pmc.lock,
                    perfcount_enable: args.__bindgen_anon_1.pmc.perfcount_enable,
                }),
                version: None,
            },
            mirage_schema::amdgpu::KfdProfilerOp::PcSample => KfdProfilerArgs {
                pc_sample: Some(KfdPcSampleArgs {
                    sample_info: sample_info.iter().copied().map(Into::into).collect(),
                    num_sample_info: args.__bindgen_anon_1.pc_sample.num_sample_info,
                    op: std::mem::transmute(args.__bindgen_anon_1.pc_sample.op),
                    gpu_id: args.__bindgen_anon_1.pc_sample.gpu_id,
                    trace_id: args.__bindgen_anon_1.pc_sample.trace_id,
                    flags: args.__bindgen_anon_1.pc_sample.flags,
                    reserved: args.__bindgen_anon_1.pc_sample.reserved,
                }),
                pmc: None,
                version: None,
            },
            mirage_schema::amdgpu::KfdProfilerOp::Version => KfdProfilerArgs {
                pc_sample: None,
                pmc: None,
                version: Some(args.__bindgen_anon_1.version),
            },
        }
    }
}