use std::mem::size_of;

use mirage_schema::amdgpu::{
    CtxOp, DrmAmdgpuBoListRequest, DrmAmdgpuBoListResponse, DrmAmdgpuCsRequest,
    DrmAmdgpuCsResponse, DrmAmdgpuCtxRequest, DrmAmdgpuCtxResponse, DrmAmdgpuFenceToHandleRequest,
    DrmAmdgpuFenceToHandleResponse, DrmAmdgpuGemCreateRequest, DrmAmdgpuGemCreateResponse,
    DrmAmdgpuGemDgmaRequest, DrmAmdgpuGemDgmaResponse, DrmAmdgpuGemListHandlesRequest,
    DrmAmdgpuGemListHandlesResponse, DrmAmdgpuGemMetadataRequest, DrmAmdgpuGemMetadataResponse,
    DrmAmdgpuGemMmapRequest, DrmAmdgpuGemMmapResponse, DrmAmdgpuGemOpRequest,
    DrmAmdgpuGemOpResponse, DrmAmdgpuGemUserptrRequest, DrmAmdgpuGemUserptrResponse,
    DrmAmdgpuGemVaRequest, DrmAmdgpuGemVaResponse, DrmAmdgpuGemWaitIdleRequest,
    DrmAmdgpuGemWaitIdleResponse, DrmAmdgpuInfoRequest, DrmAmdgpuInfoResponse,
    DrmAmdgpuSchedRequest, DrmAmdgpuSchedResponse, DrmAmdgpuSemRequest, DrmAmdgpuSemResponse,
    DrmAmdgpuUserqRequest, DrmAmdgpuUserqResponse, DrmAmdgpuUserqSignalRequest,
    DrmAmdgpuUserqSignalResponse, DrmAmdgpuUserqWaitRequest, DrmAmdgpuUserqWaitResponse,
    DrmAmdgpuVmRequest, DrmAmdgpuVmResponse, DrmAmdgpuWaitCsRequest, DrmAmdgpuWaitCsResponse,
    DrmAmdgpuWaitFencesRequest, DrmAmdgpuWaitFencesResponse, HandleAnyDrmIoctl, HandleDrmIoctl,
    IoctlCtx, SemOp,
};
use mirage_schema::amdgpu_error::AmdgpuResult;
use mirage_uapi::drm;
use mirage_uapi::drm_marshal::{DrmBoListEntry, DrmInfoOwned, build_cs_chunks};
use mirage_uapi::ioctl::{drm_iow, drm_iowr, maybe_mut_ptr, maybe_ptr};
use mirage_uapi::{FromC, FromCWith, ToBytes, ToC};

use crate::RealEmulator;
use crate::ioctl::{
    DrmBoListEntry, DrmCsChunk, DrmCsChunkDep, drm_iow, drm_iowr, maybe_mut_ptr, maybe_ptr,
};

impl HandleDrmIoctl for RealEmulator {
    fn drm_amdgpu_gem_create(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemCreateRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemCreateResponse> {
        let mut args = drm::drm_amdgpu_gem_create::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_gem_create_in {
                bo_size: request.bo_size,
                alignment: request.alignment,
                domains: request.domains,
                domain_flags: request.domain_flags,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_gem_create>(drm::DRM_AMDGPU_GEM_CREATE),
                &mut args,
            )?;
            Ok(DrmAmdgpuGemCreateResponse {
                handle: args.out.handle,
            })
        }
    }

    fn drm_amdgpu_gem_mmap(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemMmapRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemMmapResponse> {
        let mut args = drm::drm_amdgpu_gem_mmap::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_gem_mmap_in {
                handle: request.handle,
                _pad: 0,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_gem_mmap>(drm::DRM_AMDGPU_GEM_MMAP),
                &mut args,
            )?;
            Ok(DrmAmdgpuGemMmapResponse {
                addr_ptr: args.out.addr_ptr,
            })
        }
    }

    fn drm_amdgpu_ctx(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuCtxRequest,
    ) -> AmdgpuResult<DrmAmdgpuCtxResponse> {
        let mut args = drm::drm_amdgpu_ctx::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_ctx_in {
                op: request.op as u32,
                flags: request.flags,
                ctx_id: request.ctx_id,
                priority: request.priority,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_ctx>(drm::DRM_AMDGPU_CTX),
                &mut args,
            )?;
            let (ctx_id, flags, hangs, reset_status) = match request.op {
                CtxOp::AllocCtx => (args.out.alloc.ctx_id, 0, 0, 0),
                CtxOp::QueryState | CtxOp::QueryState2 => {
                    let state = args.out.state;
                    (request.ctx_id, state.flags, state.hangs, state.reset_status)
                }
                CtxOp::GetStablePstate | CtxOp::SetStablePstate => {
                    let pstate = args.out.pstate;
                    (request.ctx_id, pstate.flags as u64, 0, 0)
                }
                _ => (request.ctx_id, 0, 0, 0),
            };
            Ok(DrmAmdgpuCtxResponse {
                ctx_id,
                flags,
                hangs,
                reset_status,
            })
        }
    }

    fn drm_amdgpu_bo_list(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuBoListRequest,
    ) -> AmdgpuResult<DrmAmdgpuBoListResponse> {
        let bo_info: Vec<DrmBoListEntry> = request
            .bo_info
            .iter()
            .map(|entry| entry.to_c(&mut ()))
            .collect();
        let mut args = drm::drm_amdgpu_bo_list::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_bo_list_in {
                operation: request.operation as u32,
                list_handle: request.list_handle,
                bo_number: bo_info.len() as u32,
                bo_info_size: size_of::<DrmBoListEntry>() as u32,
                bo_info_ptr: maybe_ptr(&bo_info),
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_bo_list>(drm::DRM_AMDGPU_BO_LIST),
                &mut args,
            )?;
            Ok(DrmAmdgpuBoListResponse {
                list_handle: args.out.list_handle,
            })
        }
    }

    fn drm_amdgpu_cs(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuCsRequest,
    ) -> AmdgpuResult<DrmAmdgpuCsResponse> {
        let (_payloads, raw_chunks) = build_cs_chunks(&request.chunks);
        let mut args = drm::drm_amdgpu_cs::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_cs_in {
                ctx_id: request.ctx_id,
                bo_list_handle: request.bo_list_handle,
                num_chunks: raw_chunks.len() as u32,
                flags: request.flags,
                chunks: maybe_ptr(&raw_chunks),
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_cs>(drm::DRM_AMDGPU_CS),
                &mut args,
            )?;
            Ok(DrmAmdgpuCsResponse {
                handle: args.out.handle,
            })
        }
    }

    fn drm_amdgpu_info(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuInfoRequest,
    ) -> AmdgpuResult<DrmAmdgpuInfoResponse> {
        let mut owned = DrmInfoOwned::default();
        let mut args = request.to_c(&mut owned);
        self.drm_ioctl(
            drm_iow::<drm::drm_amdgpu_info>(drm::DRM_AMDGPU_INFO),
            &mut args,
        )?;
        Ok(DrmAmdgpuInfoResponse::from_c(args, owned))
    }

    fn drm_amdgpu_gem_metadata(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemMetadataRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemMetadataResponse> {
        let mut data = [0u32; 64];
        let copy_len = request.data.len().min(data.len());
        data[..copy_len].copy_from_slice(&request.data[..copy_len]);
        let mut args = drm::drm_amdgpu_gem_metadata {
            handle: request.handle,
            op: request.op as u32,
            data: drm::drm_amdgpu_gem_metadata__bindgen_ty_1 {
                flags: request.flags,
                tiling_info: request.tiling_info,
                data_size_bytes: request.data_size_bytes,
                data,
            },
        };
        self.drm_ioctl(
            drm_iowr::<drm::drm_amdgpu_gem_metadata>(drm::DRM_AMDGPU_GEM_METADATA),
            &mut args,
        )?;
        let out_len =
            ((args.data.data_size_bytes as usize) / size_of::<u32>()).min(args.data.data.len());
        Ok(DrmAmdgpuGemMetadataResponse {
            flags: args.data.flags,
            tiling_info: args.data.tiling_info,
            data_size_bytes: args.data.data_size_bytes,
            data: args.data.data[..out_len].to_vec(),
        })
    }

    fn drm_amdgpu_gem_wait_idle(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemWaitIdleRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemWaitIdleResponse> {
        let mut args = drm::drm_amdgpu_gem_wait_idle::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_gem_wait_idle_in {
                handle: request.handle,
                flags: request.flags,
                timeout: request.timeout,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_gem_wait_idle>(drm::DRM_AMDGPU_GEM_WAIT_IDLE),
                &mut args,
            )?;
            Ok(DrmAmdgpuGemWaitIdleResponse {
                status: args.out.status,
                domain: args.out.domain,
            })
        }
    }

    fn drm_amdgpu_gem_va(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemVaRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemVaResponse> {
        let handles = request.input_fence_syncobj_handles;
        let mut args = drm::drm_amdgpu_gem_va {
            handle: request.handle,
            _pad: 0,
            operation: request.operation as u32,
            flags: request.flags,
            va_address: request.va_address,
            offset_in_bo: request.offset_in_bo,
            map_size: request.map_size,
            vm_timeline_point: request.vm_timeline_point,
            vm_timeline_syncobj_out: request.vm_timeline_syncobj_out,
            num_syncobj_handles: request.num_syncobj_handles,
            input_fence_syncobj_handles: maybe_ptr(&handles),
        };
        self.drm_ioctl(
            drm_iow::<drm::drm_amdgpu_gem_va>(drm::DRM_AMDGPU_GEM_VA),
            &mut args,
        )?;
        Ok(DrmAmdgpuGemVaResponse {})
    }

    fn drm_amdgpu_wait_cs(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuWaitCsRequest,
    ) -> AmdgpuResult<DrmAmdgpuWaitCsResponse> {
        let mut args = drm::drm_amdgpu_wait_cs::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_wait_cs_in {
                handle: request.handle,
                timeout: request.timeout,
                ip_type: request.ip_type,
                ip_instance: request.ip_instance,
                ring: request.ring,
                ctx_id: request.ctx_id,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_wait_cs>(drm::DRM_AMDGPU_WAIT_CS),
                &mut args,
            )?;
            Ok(DrmAmdgpuWaitCsResponse {
                status: args.out.status,
            })
        }
    }

    fn drm_amdgpu_gem_op(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemOpRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemOpResponse> {
        let mut raw_data = vec![0u8; request.num_entries as usize * size_of::<u64>()];
        let mut args = drm::drm_amdgpu_gem_op {
            handle: request.handle,
            op: request.op as u32,
            value: if raw_data.is_empty() {
                request.value
            } else {
                maybe_mut_ptr(&mut raw_data)
            },
        };
        self.drm_ioctl(
            drm_iowr::<drm::drm_amdgpu_gem_op>(drm::DRM_AMDGPU_GEM_OP),
            &mut args,
        )?;
        Ok(DrmAmdgpuGemOpResponse {
            value: args.value,
            raw_data,
            num_entries: request.num_entries,
        })
    }

    fn drm_amdgpu_gem_userptr(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemUserptrRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemUserptrResponse> {
        let mut args = drm::drm_amdgpu_gem_userptr {
            addr: request.addr,
            size: request.size,
            flags: request.flags,
            handle: 0,
        };
        self.drm_ioctl(
            drm_iowr::<drm::drm_amdgpu_gem_userptr>(drm::DRM_AMDGPU_GEM_USERPTR),
            &mut args,
        )?;
        Ok(DrmAmdgpuGemUserptrResponse {
            handle: args.handle,
        })
    }

    fn drm_amdgpu_wait_fences(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuWaitFencesRequest,
    ) -> AmdgpuResult<DrmAmdgpuWaitFencesResponse> {
        let fences: Vec<drm::drm_amdgpu_fence> = request
            .fences
            .iter()
            .map(|fence| fence.to_c(&mut ()))
            .collect();
        let mut args = drm::drm_amdgpu_wait_fences::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_wait_fences_in {
                fences: maybe_ptr(&fences),
                fence_count: fences.len() as u32,
                wait_all: request.wait_all as u32,
                timeout_ns: request.timeout_ns,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_wait_fences>(drm::DRM_AMDGPU_WAIT_FENCES),
                &mut args,
            )?;
            Ok(DrmAmdgpuWaitFencesResponse {
                status: args.out.status,
                first_signaled: args.out.first_signaled,
            })
        }
    }

    fn drm_amdgpu_vm(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuVmRequest,
    ) -> AmdgpuResult<DrmAmdgpuVmResponse> {
        let mut args = drm::drm_amdgpu_vm::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_vm_in {
                op: request.op as u32,
                flags: request.flags,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_vm>(drm::DRM_AMDGPU_VM),
                &mut args,
            )?;
            Ok(DrmAmdgpuVmResponse {
                flags: args.out.flags,
            })
        }
    }

    fn drm_amdgpu_fence_to_handle(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuFenceToHandleRequest,
    ) -> AmdgpuResult<DrmAmdgpuFenceToHandleResponse> {
        let mut args = drm::drm_amdgpu_fence_to_handle::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_fence_to_handle__bindgen_ty_1 {
                fence: request.fence.to_c(&mut ()),
                what: request.what as u32,
                pad: 0,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_fence_to_handle>(drm::DRM_AMDGPU_FENCE_TO_HANDLE),
                &mut args,
            )?;
            Ok(DrmAmdgpuFenceToHandleResponse {
                handle: args.out.handle,
            })
        }
    }

    fn drm_amdgpu_sched(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuSchedRequest,
    ) -> AmdgpuResult<DrmAmdgpuSchedResponse> {
        let mut args = drm::drm_amdgpu_sched::default();
        args.in_ = drm::drm_amdgpu_sched_in {
            op: request.op as u32,
            fd: request.fd,
            priority: request.priority,
            ctx_id: request.ctx_id,
        };
        self.drm_ioctl(
            drm_iow::<drm::drm_amdgpu_sched>(drm::DRM_AMDGPU_SCHED),
            &mut args,
        )?;
        Ok(DrmAmdgpuSchedResponse {})
    }

    fn drm_amdgpu_userq(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuUserqRequest,
    ) -> AmdgpuResult<DrmAmdgpuUserqResponse> {
        let mqd = request.mqd.to_bytes();
        let mut args = drm::drm_amdgpu_userq::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_userq_in {
                op: request.op as u32,
                queue_id: request.queue_id,
                ip_type: request.ip_type,
                doorbell_handle: request.doorbell_handle,
                doorbell_offset: request.doorbell_offset,
                flags: request.flags,
                queue_va: request.queue_va,
                queue_size: request.queue_size,
                rptr_va: request.rptr_va,
                wptr_va: request.wptr_va,
                mqd: maybe_ptr(&mqd),
                mqd_size: request.mqd_size,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_userq>(drm::DRM_AMDGPU_USERQ),
                &mut args,
            )?;
            Ok(DrmAmdgpuUserqResponse {
                queue_id: args.out.queue_id,
            })
        }
    }

    fn drm_amdgpu_userq_signal(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuUserqSignalRequest,
    ) -> AmdgpuResult<DrmAmdgpuUserqSignalResponse> {
        let syncobj_handles = request.syncobj_handles;
        let bo_read_handles = request.bo_read_handles;
        let bo_write_handles = request.bo_write_handles;
        let mut args = drm::drm_amdgpu_userq_signal {
            queue_id: request.queue_id,
            pad: 0,
            syncobj_handles: maybe_ptr(&syncobj_handles),
            num_syncobj_handles: syncobj_handles.len() as u64,
            bo_read_handles: maybe_ptr(&bo_read_handles),
            bo_write_handles: maybe_ptr(&bo_write_handles),
            num_bo_read_handles: bo_read_handles.len() as u32,
            num_bo_write_handles: bo_write_handles.len() as u32,
        };
        self.drm_ioctl(
            drm_iowr::<drm::drm_amdgpu_userq_signal>(drm::DRM_AMDGPU_USERQ_SIGNAL),
            &mut args,
        )?;
        Ok(DrmAmdgpuUserqSignalResponse {})
    }

    fn drm_amdgpu_userq_wait(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuUserqWaitRequest,
    ) -> AmdgpuResult<DrmAmdgpuUserqWaitResponse> {
        let syncobj_handles = request.syncobj_handles;
        let syncobj_timeline_handles = request.syncobj_timeline_handles;
        let syncobj_timeline_points = request.syncobj_timeline_points;
        let bo_read_handles = request.bo_read_handles;
        let bo_write_handles = request.bo_write_handles;
        let mut fences =
            vec![drm::drm_amdgpu_userq_fence_info::default(); request.max_fences as usize];
        let mut args = drm::drm_amdgpu_userq_wait {
            waitq_id: request.waitq_id,
            pad: 0,
            syncobj_handles: maybe_ptr(&syncobj_handles),
            syncobj_timeline_handles: maybe_ptr(&syncobj_timeline_handles),
            syncobj_timeline_points: maybe_ptr(&syncobj_timeline_points),
            bo_read_handles: maybe_ptr(&bo_read_handles),
            bo_write_handles: maybe_ptr(&bo_write_handles),
            num_syncobj_timeline_handles: syncobj_timeline_handles.len() as u16,
            num_fences: fences.len() as u16,
            num_syncobj_handles: syncobj_handles.len() as u32,
            num_bo_read_handles: bo_read_handles.len() as u32,
            num_bo_write_handles: bo_write_handles.len() as u32,
            out_fences: maybe_mut_ptr(&mut fences),
        };
        self.drm_ioctl(
            drm_iowr::<drm::drm_amdgpu_userq_wait>(drm::DRM_AMDGPU_USERQ_WAIT),
            &mut args,
        )?;
        fences.truncate(args.num_fences as usize);
        Ok(DrmAmdgpuUserqWaitResponse {
            fences: fences
                .into_iter()
                .map(|fence| FromC::from_c(fence))
                .collect(),
        })
    }

    fn drm_amdgpu_gem_list_handles(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemListHandlesRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemListHandlesResponse> {
        let mut entries =
            vec![drm::drm_amdgpu_gem_list_handles_entry::default(); request.max_entries as usize];
        let mut args = drm::drm_amdgpu_gem_list_handles {
            entries: maybe_mut_ptr(&mut entries),
            num_entries: entries.len() as u32,
            padding: 0,
        };
        self.drm_ioctl(
            drm_iowr::<drm::drm_amdgpu_gem_list_handles>(drm::DRM_AMDGPU_GEM_LIST_HANDLES),
            &mut args,
        )?;
        entries.truncate(args.num_entries as usize);
        Ok(DrmAmdgpuGemListHandlesResponse {
            entries: entries
                .into_iter()
                .map(|entry| FromC::from_c(entry))
                .collect(),
            num_entries: args.num_entries,
        })
    }

    fn drm_amdgpu_sem(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuSemRequest,
    ) -> AmdgpuResult<DrmAmdgpuSemResponse> {
        let mut args = drm::drm_amdgpu_sem::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_sem_in {
                op: request.op as u32,
                handle: request.handle,
                ctx_id: request.ctx_id,
                ip_type: request.ip_type,
                ip_instance: request.ip_instance,
                ring: request.ring,
                seq: request.seq,
            };
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_sem>(drm::DRM_AMDGPU_SEM),
                &mut args,
            )?;
            Ok(DrmAmdgpuSemResponse {
                handle_or_fd: if matches!(request.op, SemOp::ExportSem) {
                    args.out.fd
                } else {
                    args.out.handle as i32
                },
            })
        }
    }

    fn drm_amdgpu_gem_dgma(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemDgmaRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemDgmaResponse> {
        let mut args = drm::drm_amdgpu_gem_dgma {
            addr: request.addr,
            size: request.size,
            op: request.op as u32,
            handle: request.handle,
        };
        self.drm_ioctl(
            drm_iowr::<drm::drm_amdgpu_gem_dgma>(drm::DRM_AMDGPU_GEM_DGMA),
            &mut args,
        )?;
        Ok(DrmAmdgpuGemDgmaResponse {
            addr: args.addr,
            handle: args.handle,
        })
    }
}

impl HandleAnyDrmIoctl for RealEmulator {}
