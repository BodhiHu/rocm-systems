//! AMD GPU ioctl request/response definitions for `/dev/kfd` and the DRM
//! render nodes, mirroring `schema/kfd.fbs` and `schema/drm.fbs`.
//!
//! For every ioctl inside, the [`ioctl_dsl!`] macro emits:
//!
//! * `pub const $NAME: u32` — the ioctl number.
//! * `pub struct $NameRequest` / `$NameResponse` — request / response
//!   payloads.
//! * a `Handle{Subsys}Ioctl` trait with one method per ioctl.
//! 

use serde::{Deserialize, Serialize};

use crate::ioctl_dsl;

/// KFD queue types (see `enum KfdQueueType` in `schema/kfd.fbs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum KfdQueueType {
    Compute = 0,
    Sdma = 1,
    ComputeAql = 2,
    SdmaXgmi = 3,
    SdmaByEngId = 4,
}

ioctl_dsl! {
    kfd {
        /// `AMDKFD_IOC_GET_VERSION` — return the KFD interface version so
        /// userspace can check compatibility.
        AMDKFD_IOC_GET_VERSION(0x01) {} => {
            /// KFD major version (currently 1).
            major_version : u32,
            /// KFD minor version (currently 18).
            minor_version : u32,
        };

        /// `AMDKFD_IOC_CREATE_QUEUE` — create a compute, SDMA, or AQL queue
        /// on a specific GPU. The queue is backed by a ring buffer in
        /// GPU-accessible memory.
        AMDKFD_IOC_CREATE_QUEUE(0x02) {
            /// GPU virtual address of the ring buffer.
            ring_base_address : u64,
            /// GPU VA of the write pointer (doorbell target).
            write_pointer_address : u64,
            /// GPU VA of the read pointer.
            read_pointer_address : u64,
            /// Ring buffer size in bytes (minimum 1024, power of 2).
            ring_size : u32,
            /// Target GPU device ID.
            gpu_id : u32,
            /// Queue type: `COMPUTE`, `SDMA`, `COMPUTE_AQL`, `SDMA_XGMI`,
            /// or `SDMA_BY_ENG_ID`.
            queue_type : KfdQueueType,
            /// Queue scheduling percentage (0–100).
            queue_percentage : u32,
            /// Queue priority (0–15, higher = more priority).
            queue_priority : u32,
            /// GPU VA of the End-of-Pipe buffer for completion tracking.
            eop_buffer_address : u64,
            /// Size of the EoP buffer.
            eop_buffer_size : u64,
            /// GPU VA of the context save/restore area (for preemption).
            ctx_save_restore_address : u64,
            /// Size of the context save/restore area.
            ctx_save_restore_size : u32,
            /// Size of the control stack.
            ctl_stack_size : u32,
            /// Specific SDMA engine ID (for `SDMA_BY_ENG_ID` type).
            sdma_engine_id : u32,
        } => {
            /// Kernel-assigned doorbell offset for this queue.
            doorbell_offset : u64,
            /// Unique queue ID assigned by the kernel.
            queue_id : u32,
        };
    }

    // TODO: drm ioctls
}
