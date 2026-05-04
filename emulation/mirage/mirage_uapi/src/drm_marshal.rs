use crate::amdgpu::{
    BoListEntry, ChunkId, CsChunk, CsChunkCpGfxShadow, CsChunkDep, CsChunkFence, CsChunkIb,
    CsChunkSyncobj, DrmAmdgpuInfoRequest, DrmAmdgpuInfoResponse, Fence, GemListHandlesEntry,
    HwIpType, UserqFenceInfo, UserqMqd,
};

use crate::drm;
use crate::ioctl::{bytes_of_slice, bytes_of_value, maybe_mut_ptr, maybe_ptr};
use crate::{FromC, FromCWith, ToBytes, ToC};

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct DrmBoListEntry {
    pub bo_handle: u32,
    pub bo_priority: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct DrmCsChunk {
    pub chunk_id: u32,
    pub length_dw: u32,
    pub chunk_data: u64,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct DrmCsChunkDep {
    pub ip_type: u32,
    pub ip_instance: u32,
    pub ring: u32,
    pub ctx_id: u32,
    pub handle: u64,
}

#[derive(Debug, Default, Clone)]
pub struct DrmInfoOwned {
    pub raw_data: bytes::BytesMut,
}

impl ToC<DrmBoListEntry> for BoListEntry {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> DrmBoListEntry {
        DrmBoListEntry {
            bo_handle: self.bo_handle,
            bo_priority: self.bo_priority,
        }
    }
}

impl ToC<drm::drm_amdgpu_fence> for Fence {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> drm::drm_amdgpu_fence {
        drm::drm_amdgpu_fence {
            ctx_id: self.ctx_id,
            ip_type: self.ip_type,
            ip_instance: self.ip_instance,
            ring: self.ring,
            seq_no: self.seq_no,
        }
    }
}

impl ToC<DrmCsChunkDep> for CsChunkDep {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> DrmCsChunkDep {
        DrmCsChunkDep {
            ip_type: self.ip_type,
            ip_instance: self.ip_instance,
            ring: self.ring,
            ctx_id: self.ctx_id,
            handle: self.handle,
        }
    }
}

impl ToC<drm::drm_amdgpu_cs_chunk_syncobj> for CsChunkSyncobj {
    type Owned = ();

    fn to_c(&self, _owned: &mut Self::Owned) -> drm::drm_amdgpu_cs_chunk_syncobj {
        drm::drm_amdgpu_cs_chunk_syncobj {
            handle: self.handle,
            flags: self.flags,
            point: self.point,
        }
    }
}

impl FromC<drm::drm_amdgpu_userq_fence_info> for UserqFenceInfo {
    fn from_c(raw: drm::drm_amdgpu_userq_fence_info) -> Self {
        Self {
            gpu_va: raw.va,
            value: raw.value,
        }
    }
}

impl FromC<drm::drm_amdgpu_gem_list_handles_entry> for GemListHandlesEntry {
    fn from_c(raw: drm::drm_amdgpu_gem_list_handles_entry) -> Self {
        Self {
            gem_handle: raw.gem_handle,
            flags: raw.flags,
            size: raw.size,
            preferred_domains: raw.preferred_domains,
            alloc_flags: raw.alloc_flags,
            alignment: raw.alignment,
        }
    }
}

impl ToBytes for CsChunkIb {
    fn to_bytes(&self) -> Vec<u8> {
        bytes_of_value(&drm::drm_amdgpu_cs_chunk_ib {
            _pad: 0,
            flags: self.flags,
            va_start: self.va_address,
            ib_bytes: self.ib_bytes,
            ip_type: self.ip_type as u32,
            ip_instance: self.ip_instance,
            ring: self.ring,
        })
    }
}

impl ToBytes for CsChunkFence {
    fn to_bytes(&self) -> Vec<u8> {
        bytes_of_value(&drm::drm_amdgpu_cs_chunk_fence {
            handle: self.handle,
            offset: self.offset,
        })
    }
}

impl ToBytes for CsChunkCpGfxShadow {
    fn to_bytes(&self) -> Vec<u8> {
        bytes_of_value(&drm::drm_amdgpu_cs_chunk_cp_gfx_shadow {
            shadow_va: self.shadow_va,
            csa_va: self.csa_va,
            gds_va: self.gds_va,
            flags: self.flags,
        })
    }
}

impl ToBytes for UserqMqd {
    fn to_bytes(&self) -> Vec<u8> {
        if let Some(gfx11) = self.gfx11 {
            bytes_of_value(&drm::drm_amdgpu_userq_mqd_gfx11 {
                shadow_va: gfx11.shadow_va,
                csa_va: gfx11.csa_va,
            })
        } else if let Some(sdma) = self.sdma_gfx11 {
            bytes_of_value(&drm::drm_amdgpu_userq_mqd_sdma_gfx11 {
                csa_va: sdma.csa_va,
            })
        } else if let Some(compute) = self.compute_gfx11 {
            bytes_of_value(&drm::drm_amdgpu_userq_mqd_compute_gfx11 {
                eop_va: compute.eop_va,
            })
        } else {
            self.raw_data.clone()
        }
    }
}

impl ToC<DrmCsChunk> for CsChunk {
    type Owned = Vec<u8>;

    fn to_c(&self, payload: &mut Self::Owned) -> DrmCsChunk {
        *payload = match self.chunk_id {
            ChunkId::Ib => self
                .ib
                .unwrap_or(CsChunkIb {
                    flags: 0,
                    va_address: 0,
                    ib_bytes: 0,
                    ip_type: HwIpType::Gfx,
                    ip_instance: 0,
                    ring: 0,
                })
                .to_bytes(),
            ChunkId::Fence => self
                .fence
                .unwrap_or(CsChunkFence {
                    handle: 0,
                    offset: 0,
                })
                .to_bytes(),
            ChunkId::Dependencies | ChunkId::ScheduledDependencies => bytes_of_slice(
                &self
                    .dependencies
                    .iter()
                    .map(|dep| dep.to_c(&mut ()))
                    .collect::<Vec<DrmCsChunkDep>>(),
            ),
            ChunkId::SyncobjIn
            | ChunkId::SyncobjOut
            | ChunkId::SyncobjTimelineWait
            | ChunkId::SyncobjTimelineSignal => bytes_of_slice(
                &self
                    .syncobjs
                    .iter()
                    .map(|syncobj| syncobj.to_c(&mut ()))
                    .collect::<Vec<drm::drm_amdgpu_cs_chunk_syncobj>>(),
            ),
            ChunkId::BoHandles => bytes_of_slice(&self.bo_handles),
            ChunkId::CpGfxShadow => self
                .cp_gfx_shadow
                .unwrap_or(CsChunkCpGfxShadow {
                    shadow_va: 0,
                    csa_va: 0,
                    gds_va: 0,
                    flags: 0,
                })
                .to_bytes(),
            _ => self.raw_data.clone(),
        };
        DrmCsChunk {
            chunk_id: self.chunk_id as u32,
            length_dw: payload.len().div_ceil(4) as u32,
            chunk_data: maybe_ptr(payload),
        }
    }
}

impl ToC<drm::drm_amdgpu_info> for DrmAmdgpuInfoRequest {
    type Owned = DrmInfoOwned;

    fn to_c(&self, owned: &mut Self::Owned) -> drm::drm_amdgpu_info {
        owned.raw_data = bytes::BytesMut::with_capacity(self.return_size as usize);
        owned.raw_data.resize(self.return_size as usize, 0);
        let mut args = drm::drm_amdgpu_info {
            return_pointer: maybe_mut_ptr(owned.raw_data.as_mut()),
            return_size: self.return_size,
            query: self.query,
            __bindgen_anon_1: Default::default(),
        };
        unsafe {
            let words = (&mut args.__bindgen_anon_1 as *mut drm::drm_amdgpu_info__bindgen_ty_1)
                .cast::<u32>();
            *words.add(0) = self.sub_query;
            *words.add(1) = self.sub_query2;
            *words.add(2) = self.sub_query3;
            *words.add(3) = self.flags;
        }
        args
    }
}

impl FromCWith<drm::drm_amdgpu_info, DrmInfoOwned> for DrmAmdgpuInfoResponse {
    fn from_c(_raw: drm::drm_amdgpu_info, owned: DrmInfoOwned) -> Self {
        Self {
            raw_data: owned.raw_data.to_vec(),
        }
    }
}

pub fn build_cs_chunks(chunks: &[CsChunk]) -> (Vec<Vec<u8>>, Vec<DrmCsChunk>) {
    let mut payloads = vec![Vec::new(); chunks.len()];
    let raw_chunks = chunks
        .iter()
        .zip(payloads.iter_mut())
        .map(|(chunk, payload)| chunk.to_c(payload))
        .collect();
    (payloads, raw_chunks)
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use crate::amdgpu::{ChunkId, CsChunk, CsChunkIb, HwIpType, UserqMqd, UserqMqdGfx11};

    use super::build_cs_chunks;
    use crate::ToBytes;
    use crate::drm;
    use crate::ioctl::bytes_of_slice;

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

        let (payloads, raw_chunks) = build_cs_chunks(&[chunk]);
        assert_eq!(raw_chunks[0].chunk_id, ChunkId::BoHandles as u32);
        assert_eq!(raw_chunks[0].length_dw, 2);
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

        let (_payloads, raw_chunks) = build_cs_chunks(&[chunk]);
        assert_eq!(raw_chunks[0].chunk_id, ChunkId::Ib as u32);
        assert_eq!(
            raw_chunks[0].length_dw as usize * 4,
            size_of::<drm::drm_amdgpu_cs_chunk_ib>()
        );
    }

    #[test]
    fn userq_mqd_prefers_typed_gfx11_layout() {
        let bytes = UserqMqd {
            gfx11: Some(UserqMqdGfx11 {
                shadow_va: 0x10,
                csa_va: 0x20,
            }),
            sdma_gfx11: None,
            compute_gfx11: None,
            raw_data: vec![1, 2, 3, 4],
        }
        .to_bytes();

        assert_eq!(bytes.len(), size_of::<drm::drm_amdgpu_userq_mqd_gfx11>());
        assert_ne!(bytes, vec![1, 2, 3, 4]);
    }
}
