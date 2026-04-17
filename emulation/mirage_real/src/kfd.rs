use mirage_schema::amdgpu::{HandleAnyKfdIoctl, HandleKfdIoctl, IoctlCtx};
use mirage_schema::amdgpu_error::AmdgpuResult;

use crate::RealEmulator;

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
