use std::mem::size_of;
use std::slice;

use mirage_schema::amdgpu::{
    BoListEntry, ChunkId, CsChunk, CsChunkCpGfxShadow, CsChunkDep, CsChunkFence, CsChunkIb,
    CsChunkSyncobj, CtxOp, DrmAmdgpuBoListRequest, DrmAmdgpuBoListResponse,
    DrmAmdgpuCsRequest, DrmAmdgpuCsResponse, DrmAmdgpuCtxRequest, DrmAmdgpuCtxResponse,
    DrmAmdgpuFenceToHandleRequest, DrmAmdgpuFenceToHandleResponse,
    DrmAmdgpuGemCreateRequest, DrmAmdgpuGemCreateResponse,
    DrmAmdgpuGemDgmaRequest, DrmAmdgpuGemDgmaResponse,
    DrmAmdgpuGemListHandlesRequest, DrmAmdgpuGemListHandlesResponse,
    DrmAmdgpuGemMetadataRequest, DrmAmdgpuGemMetadataResponse,
    DrmAmdgpuGemMmapRequest, DrmAmdgpuGemMmapResponse, DrmAmdgpuGemOpRequest,
    DrmAmdgpuGemOpResponse, DrmAmdgpuGemUserptrRequest, DrmAmdgpuGemUserptrResponse,
    DrmAmdgpuGemVaRequest, DrmAmdgpuGemVaResponse, DrmAmdgpuGemWaitIdleRequest,
    DrmAmdgpuGemWaitIdleResponse, DrmAmdgpuInfoRequest, DrmAmdgpuInfoResponse,
    DrmAmdgpuSchedRequest, DrmAmdgpuSchedResponse, DrmAmdgpuSemRequest,
    DrmAmdgpuSemResponse, DrmAmdgpuUserqRequest, DrmAmdgpuUserqResponse,
    DrmAmdgpuUserqSignalRequest, DrmAmdgpuUserqSignalResponse,
    DrmAmdgpuUserqWaitRequest, DrmAmdgpuUserqWaitResponse, DrmAmdgpuVmRequest,
    DrmAmdgpuVmResponse, DrmAmdgpuWaitCsRequest, DrmAmdgpuWaitCsResponse,
    DrmAmdgpuWaitFencesRequest, DrmAmdgpuWaitFencesResponse, Fence, GemListHandlesEntry,
    HandleAnyDrmIoctl, HandleDrmIoctl, IoctlCtx, SemOp, UserqFenceInfo, UserqMqd,
};
use mirage_schema::amdgpu_error::AmdgpuResult;
use mirage_uapi::drm;

use crate::ioctl::{
    drm_iow, drm_iowr, maybe_mut_ptr, maybe_ptr, DrmBoListEntry, DrmCsChunk, DrmCsChunkDep,
};
use crate::RealEmulator;

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
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_gem_create>(drm::DRM_AMDGPU_GEM_CREATE), &mut args)?;
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
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_gem_mmap>(drm::DRM_AMDGPU_GEM_MMAP), &mut args)?;
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
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_ctx>(drm::DRM_AMDGPU_CTX), &mut args)?;
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
        let bo_info: Vec<DrmBoListEntry> = request.bo_info.iter().copied().map(bo_list_entry_to_raw).collect();
        let mut args = drm::drm_amdgpu_bo_list::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_bo_list_in {
                operation: request.operation as u32,
                list_handle: request.list_handle,
                bo_number: bo_info.len() as u32,
                bo_info_size: size_of::<DrmBoListEntry>() as u32,
                bo_info_ptr: maybe_ptr(&bo_info),
            };
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_bo_list>(drm::DRM_AMDGPU_BO_LIST), &mut args)?;
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
        let mut payloads = Vec::with_capacity(request.chunks.len());
        let raw_chunks: Vec<DrmCsChunk> = request
            .chunks
            .iter()
            .map(|chunk| chunk_to_raw(chunk, &mut payloads))
            .collect();
        let mut args = drm::drm_amdgpu_cs::default();
        unsafe {
            args.in_ = drm::drm_amdgpu_cs_in {
                ctx_id: request.ctx_id,
                bo_list_handle: request.bo_list_handle,
                num_chunks: raw_chunks.len() as u32,
                flags: request.flags,
                chunks: maybe_ptr(&raw_chunks),
            };
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_cs>(drm::DRM_AMDGPU_CS), &mut args)?;
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
        let mut raw_data = vec![0u8; request.return_size as usize];
        let mut args = drm::drm_amdgpu_info {
            return_pointer: maybe_mut_ptr(&mut raw_data),
            return_size: request.return_size,
            query: request.query,
            __bindgen_anon_1: Default::default(),
        };
        unsafe {
            let words = (&mut args.__bindgen_anon_1 as *mut drm::drm_amdgpu_info__bindgen_ty_1).cast::<u32>();
            *words.add(0) = request.sub_query;
            *words.add(1) = request.sub_query2;
            *words.add(2) = request.sub_query3;
            *words.add(3) = request.flags;
            self.drm_ioctl(drm_iow::<drm::drm_amdgpu_info>(drm::DRM_AMDGPU_INFO), &mut args)?;
        }
        Ok(DrmAmdgpuInfoResponse { raw_data })
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
        unsafe {
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_gem_metadata>(drm::DRM_AMDGPU_GEM_METADATA),
                &mut args,
            )?;
        }
        let out_len = ((args.data.data_size_bytes as usize) / size_of::<u32>()).min(args.data.data.len());
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
        unsafe {
            self.drm_ioctl(drm_iow::<drm::drm_amdgpu_gem_va>(drm::DRM_AMDGPU_GEM_VA), &mut args)?;
        }
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
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_wait_cs>(drm::DRM_AMDGPU_WAIT_CS), &mut args)?;
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
            value: if raw_data.is_empty() { request.value } else { maybe_mut_ptr(&mut raw_data) },
        };
        unsafe {
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_gem_op>(drm::DRM_AMDGPU_GEM_OP), &mut args)?;
        }
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
        unsafe {
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_gem_userptr>(drm::DRM_AMDGPU_GEM_USERPTR),
                &mut args,
            )?;
        }
        Ok(DrmAmdgpuGemUserptrResponse {
            handle: args.handle,
        })
    }

    fn drm_amdgpu_wait_fences(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuWaitFencesRequest,
    ) -> AmdgpuResult<DrmAmdgpuWaitFencesResponse> {
        let fences: Vec<drm::drm_amdgpu_fence> = request.fences.iter().copied().map(fence_to_raw).collect();
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
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_vm>(drm::DRM_AMDGPU_VM), &mut args)?;
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
                fence: fence_to_raw(request.fence),
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
        unsafe {
            args.in_ = drm::drm_amdgpu_sched_in {
                op: request.op as u32,
                fd: request.fd,
                priority: request.priority,
                ctx_id: request.ctx_id,
            };
            self.drm_ioctl(drm_iow::<drm::drm_amdgpu_sched>(drm::DRM_AMDGPU_SCHED), &mut args)?;
        }
        Ok(DrmAmdgpuSchedResponse {})
    }

    fn drm_amdgpu_userq(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuUserqRequest,
    ) -> AmdgpuResult<DrmAmdgpuUserqResponse> {
        let mqd = userq_mqd_to_bytes(&request.mqd);
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
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_userq>(drm::DRM_AMDGPU_USERQ), &mut args)?;
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
        unsafe {
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_userq_signal>(drm::DRM_AMDGPU_USERQ_SIGNAL),
                &mut args,
            )?;
        }
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
        let mut fences = vec![drm::drm_amdgpu_userq_fence_info::default(); request.max_fences as usize];
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
        unsafe {
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_userq_wait>(drm::DRM_AMDGPU_USERQ_WAIT),
                &mut args,
            )?;
        }
        fences.truncate(args.num_fences as usize);
        Ok(DrmAmdgpuUserqWaitResponse {
            fences: fences.into_iter().map(userq_fence_from_raw).collect(),
        })
    }

    fn drm_amdgpu_gem_list_handles(
        &self,
        _ctx: IoctlCtx,
        request: DrmAmdgpuGemListHandlesRequest,
    ) -> AmdgpuResult<DrmAmdgpuGemListHandlesResponse> {
        let mut entries = vec![drm::drm_amdgpu_gem_list_handles_entry::default(); request.max_entries as usize];
        let mut args = drm::drm_amdgpu_gem_list_handles {
            entries: maybe_mut_ptr(&mut entries),
            num_entries: entries.len() as u32,
            padding: 0,
        };
        unsafe {
            self.drm_ioctl(
                drm_iowr::<drm::drm_amdgpu_gem_list_handles>(drm::DRM_AMDGPU_GEM_LIST_HANDLES),
                &mut args,
            )?;
        }
        entries.truncate(args.num_entries as usize);
        Ok(DrmAmdgpuGemListHandlesResponse {
            entries: entries.into_iter().map(gem_list_handles_entry_from_raw).collect(),
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
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_sem>(drm::DRM_AMDGPU_SEM), &mut args)?;
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
        unsafe {
            self.drm_ioctl(drm_iowr::<drm::drm_amdgpu_gem_dgma>(drm::DRM_AMDGPU_GEM_DGMA), &mut args)?;
        }
        Ok(DrmAmdgpuGemDgmaResponse {
            addr: args.addr,
            handle: args.handle,
        })
    }
}

impl HandleAnyDrmIoctl for RealEmulator {}

fn bo_list_entry_to_raw(entry: BoListEntry) -> DrmBoListEntry {
    DrmBoListEntry {
        bo_handle: entry.bo_handle,
        bo_priority: entry.bo_priority,
    }
}

fn fence_to_raw(fence: Fence) -> drm::drm_amdgpu_fence {
    drm::drm_amdgpu_fence {
        ctx_id: fence.ctx_id,
        ip_type: fence.ip_type,
        ip_instance: fence.ip_instance,
        ring: fence.ring,
        seq_no: fence.seq_no,
    }
}

fn chunk_to_raw(chunk: &CsChunk, payloads: &mut Vec<Vec<u8>>) -> DrmCsChunk {
    let payload = match chunk.chunk_id {
        ChunkId::Ib => bytes_of_ib(chunk.ib.unwrap_or(CsChunkIb {
            flags: 0,
            va_address: 0,
            ib_bytes: 0,
            ip_type: mirage_schema::amdgpu::HwIpType::Gfx,
            ip_instance: 0,
            ring: 0,
        })),
        ChunkId::Fence => bytes_of_fence(chunk.fence.unwrap_or(CsChunkFence { handle: 0, offset: 0 })),
        ChunkId::Dependencies | ChunkId::ScheduledDependencies => bytes_of_slice(
            &chunk.dependencies.iter().copied().map(dep_to_raw).collect::<Vec<_>>(),
        ),
        ChunkId::SyncobjIn
        | ChunkId::SyncobjOut
        | ChunkId::SyncobjTimelineWait
        | ChunkId::SyncobjTimelineSignal => bytes_of_slice(
            &chunk.syncobjs.iter().copied().map(syncobj_to_raw).collect::<Vec<_>>(),
        ),
        ChunkId::BoHandles => bytes_of_slice(&chunk.bo_handles),
        ChunkId::CpGfxShadow => bytes_of_cp_gfx_shadow(chunk.cp_gfx_shadow.unwrap_or(CsChunkCpGfxShadow {
            shadow_va: 0,
            csa_va: 0,
            gds_va: 0,
            flags: 0,
        })),
        _ => chunk.raw_data.clone(),
    };
    let length_dw = payload.len().div_ceil(4) as u32;
    payloads.push(payload);
    let payload = payloads.last().unwrap();
    DrmCsChunk {
        chunk_id: chunk.chunk_id as u32,
        length_dw,
        chunk_data: maybe_ptr(payload),
    }
}

fn dep_to_raw(dep: CsChunkDep) -> DrmCsChunkDep {
    DrmCsChunkDep {
        ip_type: dep.ip_type,
        ip_instance: dep.ip_instance,
        ring: dep.ring,
        ctx_id: dep.ctx_id,
        handle: dep.handle,
    }
}

fn syncobj_to_raw(syncobj: CsChunkSyncobj) -> drm::drm_amdgpu_cs_chunk_syncobj {
    drm::drm_amdgpu_cs_chunk_syncobj {
        handle: syncobj.handle,
        flags: syncobj.flags,
        point: syncobj.point,
    }
}

fn userq_mqd_to_bytes(mqd: &UserqMqd) -> Vec<u8> {
    if let Some(gfx11) = mqd.gfx11 {
        bytes_of_value(&drm::drm_amdgpu_userq_mqd_gfx11 {
            shadow_va: gfx11.shadow_va,
            csa_va: gfx11.csa_va,
        })
    } else if let Some(sdma) = mqd.sdma_gfx11 {
        bytes_of_value(&drm::drm_amdgpu_userq_mqd_sdma_gfx11 {
            csa_va: sdma.csa_va,
        })
    } else if let Some(compute) = mqd.compute_gfx11 {
        bytes_of_value(&drm::drm_amdgpu_userq_mqd_compute_gfx11 {
            eop_va: compute.eop_va,
        })
    } else {
        mqd.raw_data.clone()
    }
}

fn userq_fence_from_raw(fence: drm::drm_amdgpu_userq_fence_info) -> UserqFenceInfo {
    UserqFenceInfo {
        gpu_va: fence.va,
        value: fence.value,
    }
}

fn gem_list_handles_entry_from_raw(entry: drm::drm_amdgpu_gem_list_handles_entry) -> GemListHandlesEntry {
    GemListHandlesEntry {
        gem_handle: entry.gem_handle,
        flags: entry.flags,
        size: entry.size,
        preferred_domains: entry.preferred_domains,
        alloc_flags: entry.alloc_flags,
        alignment: entry.alignment,
    }
}

fn bytes_of_ib(ib: CsChunkIb) -> Vec<u8> {
    bytes_of_value(&drm::drm_amdgpu_cs_chunk_ib {
        _pad: 0,
        flags: ib.flags,
        va_start: ib.va_address,
        ib_bytes: ib.ib_bytes,
        ip_type: ib.ip_type as u32,
        ip_instance: ib.ip_instance,
        ring: ib.ring,
    })
}

fn bytes_of_fence(fence: CsChunkFence) -> Vec<u8> {
    bytes_of_value(&drm::drm_amdgpu_cs_chunk_fence {
        handle: fence.handle,
        offset: fence.offset,
    })
}

fn bytes_of_cp_gfx_shadow(shadow: CsChunkCpGfxShadow) -> Vec<u8> {
    bytes_of_value(&drm::drm_amdgpu_cs_chunk_cp_gfx_shadow {
        shadow_va: shadow.shadow_va,
        csa_va: shadow.csa_va,
        gds_va: shadow.gds_va,
        flags: shadow.flags,
    })
}

fn bytes_of_value<T>(value: &T) -> Vec<u8> {
    unsafe { slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }.to_vec()
}

fn bytes_of_slice<T>(values: &[T]) -> Vec<u8> {
    unsafe {
        slice::from_raw_parts(
            values.as_ptr().cast::<u8>(),
            std::mem::size_of_val(values),
        )
    }
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirage_schema::amdgpu::{HwIpType, UserqMqdGfx11};

    #[test]
    fn bo_handle_chunk_marshals_as_u32_words() {
        let chunk = CsChunk {
            chunk_id: ChunkId::BoHandles,
            ib: None,
            fence: None,
            dependencies: Vec::new(),
            syncobjs: Vec::new(),
            bo_handles: vec![7, 9],
            cp_gfx_shadow: None,
            raw_data: Vec::new(),
        };
        let mut payloads = Vec::new();
        let raw = chunk_to_raw(&chunk, &mut payloads);

        assert_eq!(raw.chunk_id, ChunkId::BoHandles as u32);
        assert_eq!(raw.length_dw, 2);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0], bytes_of_slice(&[7u32, 9u32]));
    }

    #[test]
    fn ib_chunk_marshals_fixed_struct_layout() {
        let chunk = CsChunk {
            chunk_id: ChunkId::Ib,
            ib: Some(CsChunkIb {
                flags: 3,
                va_address: 0x1234,
                ib_bytes: 64,
                ip_type: HwIpType::Compute,
                ip_instance: 2,
                ring: 1,
            }),
            fence: None,
            dependencies: Vec::new(),
            syncobjs: Vec::new(),
            bo_handles: Vec::new(),
            cp_gfx_shadow: None,
            raw_data: Vec::new(),
        };
        let mut payloads = Vec::new();
        let raw = chunk_to_raw(&chunk, &mut payloads);

        assert_eq!(raw.chunk_id, ChunkId::Ib as u32);
        assert_eq!(raw.length_dw as usize * 4, size_of::<drm::drm_amdgpu_cs_chunk_ib>());
    }

    #[test]
    fn userq_mqd_prefers_typed_gfx11_layout() {
        let bytes = userq_mqd_to_bytes(&UserqMqd {
            gfx11: Some(UserqMqdGfx11 {
                shadow_va: 0x10,
                csa_va: 0x20,
            }),
            sdma_gfx11: None,
            compute_gfx11: None,
            raw_data: vec![1, 2, 3, 4],
        });

        assert_eq!(bytes.len(), size_of::<drm::drm_amdgpu_userq_mqd_gfx11>());
        assert_ne!(bytes, vec![1, 2, 3, 4]);
    }
}