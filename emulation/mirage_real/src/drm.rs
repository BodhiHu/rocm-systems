use mirage_schema::amdgpu::{HandleAnyDrmIoctl, HandleDrmIoctl};

use crate::RealEmulator;

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
