use mirage_schema::amdgpu::{
    AmdkfdIocAcquireVmRequest, AmdkfdIocAcquireVmResponse, AmdkfdIocAisOpRequest,
    AmdkfdIocAisOpResponse, AmdkfdIocAllocMemoryOfGpuRequest, AmdkfdIocAllocMemoryOfGpuResponse,
    AmdkfdIocAllocQueueGwsRequest, AmdkfdIocAllocQueueGwsResponse, AmdkfdIocAvailableMemoryRequest,
    AmdkfdIocAvailableMemoryResponse, AmdkfdIocCreateEventRequest, AmdkfdIocCreateEventResponse,
    AmdkfdIocCreateProcessRequest, AmdkfdIocCreateProcessResponse, AmdkfdIocCreateQueueRequest,
    AmdkfdIocCreateQueueResponse, AmdkfdIocCriuOpRequest, AmdkfdIocCriuOpResponse,
    AmdkfdIocCrossMemoryCopyRequest, AmdkfdIocCrossMemoryCopyResponse,
    AmdkfdIocDbgAddressWatchDeprecatedRequest, AmdkfdIocDbgAddressWatchDeprecatedResponse,
    AmdkfdIocDbgRegisterDeprecatedRequest, AmdkfdIocDbgRegisterDeprecatedResponse,
    AmdkfdIocDbgTrapRequest, AmdkfdIocDbgTrapResponse, AmdkfdIocDbgUnregisterDeprecatedRequest,
    AmdkfdIocDbgUnregisterDeprecatedResponse, AmdkfdIocDbgWaveControlDeprecatedRequest,
    AmdkfdIocDbgWaveControlDeprecatedResponse, AmdkfdIocDestroyEventRequest,
    AmdkfdIocDestroyEventResponse, AmdkfdIocDestroyQueueRequest, AmdkfdIocDestroyQueueResponse,
    AmdkfdIocExportDmabufRequest, AmdkfdIocExportDmabufResponse, AmdkfdIocFreeMemoryOfGpuRequest,
    AmdkfdIocFreeMemoryOfGpuResponse, AmdkfdIocGetClockCountersRequest,
    AmdkfdIocGetClockCountersResponse, AmdkfdIocGetDmabufInfoRequest,
    AmdkfdIocGetDmabufInfoResponse, AmdkfdIocGetProcessAperturesNewRequest,
    AmdkfdIocGetProcessAperturesNewResponse, AmdkfdIocGetProcessAperturesRequest,
    AmdkfdIocGetProcessAperturesResponse, AmdkfdIocGetQueueWaveStateRequest,
    AmdkfdIocGetQueueWaveStateResponse, AmdkfdIocGetTileConfigRequest,
    AmdkfdIocGetTileConfigResponse, AmdkfdIocGetVersionRequest, AmdkfdIocGetVersionResponse,
    AmdkfdIocImportDmabufRequest, AmdkfdIocImportDmabufResponse, AmdkfdIocIpcExportHandleRequest,
    AmdkfdIocIpcExportHandleResponse, AmdkfdIocIpcImportHandleRequest,
    AmdkfdIocIpcImportHandleResponse, AmdkfdIocMapMemoryToGpuRequest,
    AmdkfdIocMapMemoryToGpuResponse, AmdkfdIocPcSampleRequest, AmdkfdIocPcSampleResponse,
    AmdkfdIocProfilerRequest, AmdkfdIocProfilerResponse, AmdkfdIocResetEventRequest,
    AmdkfdIocResetEventResponse, AmdkfdIocRlcSpmRequest, AmdkfdIocRlcSpmResponse,
    AmdkfdIocRuntimeEnableRequest, AmdkfdIocRuntimeEnableResponse, AmdkfdIocSetCuMaskRequest,
    AmdkfdIocSetCuMaskResponse, AmdkfdIocSetEventRequest, AmdkfdIocSetEventResponse,
    AmdkfdIocSetMemoryPolicyRequest, AmdkfdIocSetMemoryPolicyResponse,
    AmdkfdIocSetScratchBackingVaRequest, AmdkfdIocSetScratchBackingVaResponse,
    AmdkfdIocSetTrapHandlerRequest, AmdkfdIocSetTrapHandlerResponse, AmdkfdIocSetXnackModeRequest,
    AmdkfdIocSetXnackModeResponse, AmdkfdIocSmiEventsRequest, AmdkfdIocSmiEventsResponse,
    AmdkfdIocSvmRequest, AmdkfdIocSvmResponse, AmdkfdIocUnmapMemoryFromGpuRequest,
    AmdkfdIocUnmapMemoryFromGpuResponse, AmdkfdIocUpdateQueueRequest, AmdkfdIocUpdateQueueResponse,
    AmdkfdIocWaitEventsRequest, AmdkfdIocWaitEventsResponse, HandleAnyKfdIoctl, HandleKfdIoctl,
    IoctlCtx, KfdCriuBoBucket, KfdCriuDeviceBucket, KfdEventData, KfdPcSampleArgs, KfdPcSampleInfo,
    KfdProcessDeviceAperture,
};
use mirage_schema::amdgpu_error::{AmdgpuError, AmdgpuResult};
use mirage_uapi::ioctl::{kfd_ior, kfd_iow, kfd_iowr, maybe_mut_ptr};
use mirage_uapi::kfd;
use mirage_uapi::kfd_marshal::{
    DbgTrapOwned, KfdCreateQueueArgsCompat, KfdCriuBoBucketRaw, KfdCriuDeviceBucketRaw,
    KfdEventDataRaw, KfdMemoryRangeRaw, KfdPcSampleInfoRaw, KfdProfilerOwned,
    KfdSetMemoryPolicyArgsCompat, KfdSvmOwned, ais_op_response_from_c, dbg_trap_from_c,
    dbg_trap_to_c, profiler_args_from_c, profiler_args_to_c,
};
use mirage_uapi::{FromC, ToC};

use crate::RealEmulator;
use crate::ioctl::{kfd_ior, kfd_iow, kfd_iowr, maybe_mut_ptr, maybe_ptr};

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
        self.kfd_ioctl(kfd_iowr::<KfdCreateQueueArgsCompat>(0x02), &mut args)?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(kfd_iow::<KfdSetMemoryPolicyArgsCompat>(0x04), &mut args)?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_update_queue_args>(0x07), &mut args)?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_event_args>(0x0A), &mut args)?;
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
        self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_reset_event_args>(0x0B), &mut args)?;
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
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_wait_events_args>(0x0C), &mut args)?;
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
        self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_dbg_register_args>(0x0D), &mut args)?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        // The client's drm_fd is a virtual fd — look up our own real
        // DRM render fd for this GPU.
        let real_drm_fd = self
            .find_render_fd_for_gpu(request.gpu_id)
            .unwrap_or_else(|| self.primary_render_fd().unwrap_or(-1));
        let mut args = kfd::kfd_ioctl_acquire_vm_args {
            drm_fd: real_drm_fd as u32,
            gpu_id: request.gpu_id,
        };
        self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_acquire_vm_args>(0x15), &mut args)?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(kfd_iow::<kfd::kfd_ioctl_set_cu_mask_args>(0x1A), &mut args)?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_smi_events_args>(0x1F), &mut args)?;
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
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_svm_args>(0x20), owned.raw_mut())?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_criu_args>(0x22), &mut args)?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        match self.kfd_ioctl(
            kfd_iowr::<kfd::kfd_ioctl_runtime_enable_args>(0x25),
            &mut args,
        ) {
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
        let mut args = dbg_trap_to_c(&request, &mut owned);
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_dbg_trap_args>(0x26), &mut args)?;
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(
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
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_spm_args>(0x84), &mut args)?;
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
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_pc_sample_args>(0x85), &mut args)?;
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
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_profiler_args>(0x86), &mut args)?;
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
        self.kfd_ioctl(kfd_iowr::<kfd::kfd_ioctl_ais_args>(0x87), &mut args)?;
        Ok(ais_op_response_from_c(&args))
    }
}

impl HandleAnyKfdIoctl for RealEmulator {}
