//! KFD sysfs topology types.
//!
//! The [`Topology`] struct holds every file that would normally live
//! under `/sys/class/kfd/kfd/topology/` as a flat map of relative
//! paths to their raw contents.  A single [`ProvideTopology::get_topology`]
//! call replaces the old per-file `SyscallSysfsRead` round-trips,
//! allowing the interceptor to cache the entire topology at startup.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Snapshot of the KFD sysfs topology tree.
///
/// Keys are relative paths under `/sys/class/kfd/kfd/topology/`
/// (e.g. `"nodes/0/properties"`, `"system_properties"`).
/// Values are the raw file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Trait for emulators that can provide a KFD sysfs topology.
pub trait ProvideTopology: Send + Sync {
    fn get_topology(&self) -> crate::amdgpu_error::AmdgpuResult<Topology>;
}
