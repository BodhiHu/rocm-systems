//! KFD sysfs topology types.
//!
//! This module provides two representations of the KFD sysfs topology tree
//! rooted at `/sys/class/kfd/kfd/topology/`:
//!
//! * [`Topology`] — a flat `BTreeMap<path, raw_bytes>` snapshot used on the
//!   wire.  A single [`ProvideTopology::get_topology`] call replaces the old
//!   per-file `SyscallSysfsRead` round-trips so the interceptor can cache the
//!   entire topology at start-up.
//!
//! * [`TypedTopology`] — a fully-typed, structured view of the same data.
//!   Every sysfs file is represented by a dedicated Rust struct.  Each struct
//!   implements [`Into<Vec<u8>>`] so it can be serialised back to the exact
//!   byte sequence the KFD driver would produce.
//!
//! # Converting between representations
//!
//! ```rust
//! use mirage_schema::topology::{Topology, TypedTopology};
//! use std::collections::BTreeMap;
//!
//! // Build a minimal raw topology (just system_properties + generation_id).
//! let mut files = BTreeMap::new();
//! files.insert("system_properties".into(), b"platform_oem 1\nplatform_id 2\nplatform_rev 3\n".to_vec());
//! files.insert("generation_id".into(), b"1\n".to_vec());
//! let raw = Topology { files };
//!
//! let typed = TypedTopology::from_topology(&raw).unwrap();
//! assert_eq!(typed.system_properties.platform_oem, 1);
//!
//! // Round-trip back to the wire format.
//! let raw2: Topology = typed.into_topology();
//! assert_eq!(raw2.files["system_properties"], raw.files["system_properties"]);
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── wire format ──────────────────────────────────────────────────────────────

/// Snapshot of the KFD sysfs topology tree.
///
/// Keys are relative paths under `/sys/class/kfd/kfd/topology/`
/// (e.g. `"nodes/0/properties"`, `"system_properties"`).
/// Values are the raw file contents as the KFD driver would write them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
    /// Relative-path → raw-bytes map for every file in the topology tree.
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Trait for emulators that can provide a KFD sysfs topology.
pub trait ProvideTopology: Send + Sync {
    /// Return a snapshot of the full KFD sysfs topology tree.
    fn get_topology(&self) -> crate::amdgpu_error::AmdgpuResult<Topology>;
}

// ── typed representation ─────────────────────────────────────────────────────

/// Structured view of the entire `/sys/class/kfd/kfd/topology/` tree.
///
/// Convert to/from the wire [`Topology`] with [`TypedTopology::from_topology`]
/// and [`TypedTopology::into_topology`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedTopology {
    /// Contents of `generation_id` — monotonically incremented by the driver
    /// each time the topology changes.
    pub generation_id: u32,
    /// Contents of `system_properties`.
    pub system_properties: SystemProperties,
    /// Per-node data, keyed by the numeric directory index under `nodes/`.
    pub nodes: BTreeMap<u32, TopologyNode>,
}

/// Contents of `/sys/class/kfd/kfd/topology/system_properties`.
///
/// The file contains three space-separated key-value pairs, one per line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemProperties {
    /// OEM platform identifier (`platform_oem`).
    pub platform_oem: u64,
    /// Platform identifier (`platform_id`).
    pub platform_id: u64,
    /// Platform revision (`platform_rev`).
    pub platform_rev: u32,
}

impl From<SystemProperties> for Vec<u8> {
    fn from(s: SystemProperties) -> Vec<u8> {
        format!(
            "platform_oem {}\nplatform_id {}\nplatform_rev {}\n",
            s.platform_oem, s.platform_id, s.platform_rev,
        )
        .into_bytes()
    }
}

/// All data associated with a single topology node directory
/// (`/sys/class/kfd/kfd/topology/nodes/<N>/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyNode {
    /// Contents of `nodes/<N>/properties`.
    pub properties: NodeProperties,
    /// Contents of `nodes/<N>/name`.  Empty for CPU nodes.
    pub name: String,
    /// Contents of `nodes/<N>/gpu_id`.  `0` for CPU nodes.
    pub gpu_id: u32,
    /// Memory banks, keyed by their directory index under `mem_banks/`.
    pub mem_banks: BTreeMap<u32, MemBank>,
    /// IO links, keyed by their directory index under `io_links/`.
    pub io_links: BTreeMap<u32, LinkProperties>,
    /// Peer-to-peer links, keyed by their directory index under `p2p_links/`.
    pub p2p_links: BTreeMap<u32, LinkProperties>,
    /// L1/L2/L3 cache descriptors, keyed by their directory index under
    /// `caches/`.
    pub caches: BTreeMap<u32, CacheProperties>,
}

/// Contents of `/sys/class/kfd/kfd/topology/nodes/<N>/properties`.
///
/// Fields that only the KFD GPU driver emits (i.e. are absent for CPU nodes)
/// are represented as [`Option`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeProperties {
    /// Number of CPU cores on this node (`cpu_cores_count`).
    /// Non-zero only for CPU nodes.
    pub cpu_cores_count: u32,
    /// Total SIMD unit count (`simd_count`).  Non-zero only for GPU nodes.
    pub simd_count: u32,
    /// Number of memory banks (`mem_banks_count`).
    pub mem_banks_count: u32,
    /// Number of cache descriptors (`caches_count`).
    pub caches_count: u32,
    /// Number of IO link descriptors (`io_links_count`).
    pub io_links_count: u32,
    /// Number of P2P link descriptors (`p2p_links_count`).
    pub p2p_links_count: u32,
    /// Base CPU core logical index (`cpu_core_id_base`).
    pub cpu_core_id_base: u32,
    /// Base SIMD logical index (`simd_id_base`).
    pub simd_id_base: u32,
    /// Maximum waves per SIMD (`max_waves_per_simd`).
    pub max_waves_per_simd: u32,
    /// Local Data Share size in KiB (`lds_size_in_kb`).
    pub lds_size_in_kb: u32,
    /// Global Data Share size in KiB (`gds_size_in_kb`).
    pub gds_size_in_kb: u32,
    /// Number of Global Wave Synchroniser slots (`num_gws`).
    pub num_gws: u32,
    /// Wavefront width in lanes (`wave_front_size`).
    pub wave_front_size: u32,
    /// Number of shader arrays (`array_count`).
    pub array_count: u32,
    /// SIMD arrays per shader engine (`simd_arrays_per_engine`).
    pub simd_arrays_per_engine: u32,
    /// Compute units per SIMD array (`cu_per_simd_array`).
    pub cu_per_simd_array: u32,
    /// SIMD units per compute unit (`simd_per_cu`).
    pub simd_per_cu: u32,
    /// Maximum scratch memory slots per CU (`max_slots_scratch_cu`).
    pub max_slots_scratch_cu: u32,
    /// GFX IP target version, e.g. `90402` for gfx904 (`gfx_target_version`).
    pub gfx_target_version: u32,
    /// PCI vendor ID (`vendor_id`).  `0` for CPU nodes.
    pub vendor_id: u32,
    /// PCI device ID (`device_id`).  `0` for CPU nodes.
    pub device_id: u32,
    /// PCI location ID (`location_id`).
    pub location_id: u32,
    /// PCI domain (`domain`).
    pub domain: u32,
    /// DRM render minor number (`drm_render_minor`).
    pub drm_render_minor: u32,
    /// XGMI hive identifier (`hive_id`).  `0` when not part of a hive.
    pub hive_id: u64,
    /// Number of SDMA engines (`num_sdma_engines`).
    pub num_sdma_engines: u32,
    /// Number of XGMI-dedicated SDMA engines (`num_sdma_xgmi_engines`).
    pub num_sdma_xgmi_engines: u32,
    /// Number of SDMA queues per engine (`num_sdma_queues_per_engine`).
    pub num_sdma_queues_per_engine: u32,
    /// Number of compute-pipe queues (`num_cp_queues`).
    pub num_cp_queues: u32,
    /// Maximum fixed-function compute clock in MHz (`max_engine_clk_ccompute`).
    pub max_engine_clk_ccompute: u32,

    // ── GPU-only fields (absent on CPU nodes) ────────────────────────────
    /// Maximum shader clock in MHz (`max_engine_clk_fcompute`).
    /// `None` for CPU nodes.
    pub max_engine_clk_fcompute: Option<u32>,
    /// VRAM size in bytes as reported by the driver (`local_mem_size`).
    /// `None` for CPU nodes.
    pub local_mem_size: Option<u64>,
    /// Microcode firmware version (`fw_version`).
    /// `None` for CPU nodes.
    pub fw_version: Option<u32>,
    /// GPU capability bitmask (`capability`).
    /// `None` for CPU nodes.
    pub capability: Option<u32>,
    /// Extended GPU capability bitmask (`capability2`).
    /// `None` for CPU nodes.
    pub capability2: Option<u32>,
    /// Debug property bitmask (`debug_prop`).
    /// `None` for CPU nodes.
    pub debug_prop: Option<u32>,
    /// SDMA firmware version (`sdma_fw_version`).
    /// `None` for CPU nodes.
    pub sdma_fw_version: Option<u32>,
    /// Unique GPU identifier (`unique_id`).
    /// `None` for CPU nodes.
    pub unique_id: Option<u64>,
    /// Number of extended compute clusters (`num_xcc`).
    /// `None` for CPU nodes.
    pub num_xcc: Option<u32>,
}

impl From<NodeProperties> for Vec<u8> {
    fn from(p: NodeProperties) -> Vec<u8> {
        let mut out = String::new();
        macro_rules! w {
            ($key:expr, $val:expr) => {
                out.push_str(&format!("{} {}\n", $key, $val));
            };
        }
        w!("cpu_cores_count", p.cpu_cores_count);
        w!("simd_count", p.simd_count);
        w!("mem_banks_count", p.mem_banks_count);
        w!("caches_count", p.caches_count);
        w!("io_links_count", p.io_links_count);
        w!("p2p_links_count", p.p2p_links_count);
        w!("cpu_core_id_base", p.cpu_core_id_base);
        w!("simd_id_base", p.simd_id_base);
        w!("max_waves_per_simd", p.max_waves_per_simd);
        w!("lds_size_in_kb", p.lds_size_in_kb);
        w!("gds_size_in_kb", p.gds_size_in_kb);
        w!("num_gws", p.num_gws);
        w!("wave_front_size", p.wave_front_size);
        w!("array_count", p.array_count);
        w!("simd_arrays_per_engine", p.simd_arrays_per_engine);
        w!("cu_per_simd_array", p.cu_per_simd_array);
        w!("simd_per_cu", p.simd_per_cu);
        w!("max_slots_scratch_cu", p.max_slots_scratch_cu);
        w!("gfx_target_version", p.gfx_target_version);
        w!("vendor_id", p.vendor_id);
        w!("device_id", p.device_id);
        w!("location_id", p.location_id);
        w!("domain", p.domain);
        w!("drm_render_minor", p.drm_render_minor);
        w!("hive_id", p.hive_id);
        w!("num_sdma_engines", p.num_sdma_engines);
        w!("num_sdma_xgmi_engines", p.num_sdma_xgmi_engines);
        w!("num_sdma_queues_per_engine", p.num_sdma_queues_per_engine);
        w!("num_cp_queues", p.num_cp_queues);
        if let Some(v) = p.max_engine_clk_fcompute {
            w!("max_engine_clk_fcompute", v);
        }
        if let Some(v) = p.local_mem_size {
            w!("local_mem_size", v);
        }
        if let Some(v) = p.fw_version {
            w!("fw_version", v);
        }
        if let Some(v) = p.capability {
            w!("capability", v);
        }
        if let Some(v) = p.capability2 {
            w!("capability2", v);
        }
        if let Some(v) = p.debug_prop {
            w!("debug_prop", v);
        }
        if let Some(v) = p.sdma_fw_version {
            w!("sdma_fw_version", v);
        }
        if let Some(v) = p.unique_id {
            w!("unique_id", v);
        }
        if let Some(v) = p.num_xcc {
            w!("num_xcc", v);
        }
        w!("max_engine_clk_ccompute", p.max_engine_clk_ccompute);
        out.into_bytes()
    }
}

/// Memory bank entry under `nodes/<N>/mem_banks/<M>/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemBank {
    /// Contents of `mem_banks/<M>/properties`.
    pub properties: MemBankProperties,
    /// Contents of `mem_banks/<M>/used_memory`, if present.
    /// CPU nodes typically do not expose this file.
    pub used_memory: Option<u64>,
}

/// Contents of `/sys/class/kfd/kfd/topology/nodes/<N>/mem_banks/<M>/properties`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemBankProperties {
    /// Heap type identifier (`heap_type`).
    /// `0` = system RAM, `1` = public VRAM, `2` = private VRAM.
    pub heap_type: u32,
    /// Total size of the memory bank in bytes (`size_in_bytes`).
    pub size_in_bytes: u64,
    /// Capability flags for this bank (`flags`).
    pub flags: u32,
    /// Memory bus width in bits (`width`).  `0` for system RAM.
    pub width: u32,
    /// Maximum memory clock in MHz (`mem_clk_max`).  `0` for system RAM.
    pub mem_clk_max: u32,
}

impl From<MemBankProperties> for Vec<u8> {
    fn from(m: MemBankProperties) -> Vec<u8> {
        format!(
            "heap_type {}\nsize_in_bytes {}\nflags {}\nwidth {}\nmem_clk_max {}\n",
            m.heap_type, m.size_in_bytes, m.flags, m.width, m.mem_clk_max,
        )
        .into_bytes()
    }
}

/// Contents of an `io_links/<M>/properties` or `p2p_links/<M>/properties` file.
///
/// Both link types share the same sysfs format; the field meanings and the set
/// of valid `type` values differ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkProperties {
    /// Link type code (`type`).
    /// `1` = HyperTransport, `2` = PCIe, `5` = XGMI, …
    pub link_type: u32,
    /// HSA link specification major version (`version_major`).
    pub version_major: u32,
    /// HSA link specification minor version (`version_minor`).
    pub version_minor: u32,
    /// Source node index (`node_from`).
    pub node_from: u32,
    /// Destination node index (`node_to`).
    pub node_to: u32,
    /// Relative link weight used by the NUMA distance metric (`weight`).
    pub weight: u32,
    /// Minimum transfer latency in ns (`min_latency`).
    pub min_latency: u32,
    /// Maximum transfer latency in ns (`max_latency`).
    pub max_latency: u32,
    /// Minimum sustained bandwidth in MB/s (`min_bandwidth`).
    pub min_bandwidth: u32,
    /// Maximum sustained bandwidth in MB/s (`max_bandwidth`).
    pub max_bandwidth: u32,
    /// Recommended single-transfer granularity in bytes
    /// (`recommended_transfer_size`).
    pub recommended_transfer_size: u32,
    /// Bitmask of SDMA engines recommended for this link
    /// (`recommended_sdma_engine_id_mask`).
    pub recommended_sdma_engine_id_mask: u32,
    /// Link capability flags (`flags`).
    pub flags: u32,
}

impl From<LinkProperties> for Vec<u8> {
    fn from(l: LinkProperties) -> Vec<u8> {
        format!(
            "type {}\nversion_major {}\nversion_minor {}\nnode_from {}\nnode_to {}\n\
             weight {}\nmin_latency {}\nmax_latency {}\nmin_bandwidth {}\nmax_bandwidth {}\n\
             recommended_transfer_size {}\nrecommended_sdma_engine_id_mask {}\nflags {}\n",
            l.link_type,
            l.version_major,
            l.version_minor,
            l.node_from,
            l.node_to,
            l.weight,
            l.min_latency,
            l.max_latency,
            l.min_bandwidth,
            l.max_bandwidth,
            l.recommended_transfer_size,
            l.recommended_sdma_engine_id_mask,
            l.flags,
        )
        .into_bytes()
    }
}

/// Contents of `/sys/class/kfd/kfd/topology/nodes/<N>/caches/<M>/properties`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheProperties {
    /// Lowest processor ID that shares this cache (`processor_id_low`).
    pub processor_id_low: u32,
    /// Cache level (`level`).  `1` = L1, `2` = L2, `3` = L3.
    pub level: u32,
    /// Cache size in KiB (`size`).
    pub size: u32,
    /// Cache line size in bytes (`cache_line_size`).
    pub cache_line_size: u32,
    /// Number of cache lines per tag (`cache_lines_per_tag`).
    pub cache_lines_per_tag: u32,
    /// Associativity — number of ways (`association`).
    pub association: u32,
    /// Cache access latency in cycles (`latency`).
    pub latency: u32,
    /// Cache type bitmask (`type`).
    /// Bit 0 = data, bit 1 = instruction, bit 2 = CPU, bit 3 = SIMD.
    pub cache_type: u32,
    /// Bitmap of sibling processors sharing this cache (`sibling_map`),
    /// stored as a list of 32-bit words in the sysfs file.
    pub sibling_map: Vec<u32>,
}

impl From<CacheProperties> for Vec<u8> {
    fn from(c: CacheProperties) -> Vec<u8> {
        let map = c
            .sibling_map
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "processor_id_low {}\nlevel {}\nsize {}\ncache_line_size {}\n\
             cache_lines_per_tag {}\nassociation {}\nlatency {}\ntype {}\nsibling_map {}\n",
            c.processor_id_low,
            c.level,
            c.size,
            c.cache_line_size,
            c.cache_lines_per_tag,
            c.association,
            c.latency,
            c.cache_type,
            map,
        )
        .into_bytes()
    }
}

// ── parsing helpers ───────────────────────────────────────────────────────────

/// Error type for topology parsing failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyParseError {
    /// Human-readable description of the failure.
    pub message: String,
}

impl std::fmt::Display for TopologyParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "topology parse error: {}", self.message)
    }
}

impl std::error::Error for TopologyParseError {}

impl TopologyParseError {
    fn new(msg: impl Into<String>) -> Self {
        Self { message: msg.into() }
    }
}

type ParseResult<T> = Result<T, TopologyParseError>;

/// Parse a sysfs key-value file (one `"key value\n"` pair per line) into a
/// `BTreeMap`.
fn parse_kv(bytes: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut map = BTreeMap::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once(' ') {
            map.insert(k.trim().to_owned(), v.trim().to_owned());
        }
    }
    map
}

/// Look up a required u32 field in a key-value map.
fn req_u32(map: &BTreeMap<String, String>, key: &str) -> ParseResult<u32> {
    map.get(key)
        .ok_or_else(|| TopologyParseError::new(format!("missing field `{key}`")))?
        .parse()
        .map_err(|_| TopologyParseError::new(format!("bad u32 for `{key}`")))
}

/// Look up a required u64 field in a key-value map.
fn req_u64(map: &BTreeMap<String, String>, key: &str) -> ParseResult<u64> {
    map.get(key)
        .ok_or_else(|| TopologyParseError::new(format!("missing field `{key}`")))?
        .parse()
        .map_err(|_| TopologyParseError::new(format!("bad u64 for `{key}`")))
}

/// Look up an optional u32 field in a key-value map (returns `None` if absent).
fn opt_u32(map: &BTreeMap<String, String>, key: &str) -> ParseResult<Option<u32>> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => v
            .parse()
            .map(Some)
            .map_err(|_| TopologyParseError::new(format!("bad u32 for `{key}`"))),
    }
}

/// Look up an optional u64 field in a key-value map (returns `None` if absent).
fn opt_u64(map: &BTreeMap<String, String>, key: &str) -> ParseResult<Option<u64>> {
    match map.get(key) {
        None => Ok(None),
        Some(v) => v
            .parse()
            .map(Some)
            .map_err(|_| TopologyParseError::new(format!("bad u64 for `{key}`"))),
    }
}

/// Parse a single integer from raw bytes (the format used by `gpu_id`,
/// `generation_id`, and `used_memory`).
fn parse_single_u32(bytes: &[u8]) -> ParseResult<u32> {
    String::from_utf8_lossy(bytes)
        .trim()
        .parse()
        .map_err(|_| TopologyParseError::new("expected single u32 value"))
}

fn parse_single_u64(bytes: &[u8]) -> ParseResult<u64> {
    String::from_utf8_lossy(bytes)
        .trim()
        .parse()
        .map_err(|_| TopologyParseError::new("expected single u64 value"))
}

// ── per-struct parsing ────────────────────────────────────────────────────────

impl SystemProperties {
    /// Parse from raw sysfs file bytes.
    pub fn from_bytes(bytes: &[u8]) -> ParseResult<Self> {
        let m = parse_kv(bytes);
        Ok(Self {
            platform_oem: req_u64(&m, "platform_oem")?,
            platform_id: req_u64(&m, "platform_id")?,
            platform_rev: req_u32(&m, "platform_rev")?,
        })
    }
}

impl NodeProperties {
    /// Parse from raw sysfs file bytes.
    pub fn from_bytes(bytes: &[u8]) -> ParseResult<Self> {
        let m = parse_kv(bytes);
        Ok(Self {
            cpu_cores_count: req_u32(&m, "cpu_cores_count")?,
            simd_count: req_u32(&m, "simd_count")?,
            mem_banks_count: req_u32(&m, "mem_banks_count")?,
            caches_count: req_u32(&m, "caches_count")?,
            io_links_count: req_u32(&m, "io_links_count")?,
            p2p_links_count: req_u32(&m, "p2p_links_count")?,
            cpu_core_id_base: req_u32(&m, "cpu_core_id_base")?,
            simd_id_base: req_u32(&m, "simd_id_base")?,
            max_waves_per_simd: req_u32(&m, "max_waves_per_simd")?,
            lds_size_in_kb: req_u32(&m, "lds_size_in_kb")?,
            gds_size_in_kb: req_u32(&m, "gds_size_in_kb")?,
            num_gws: req_u32(&m, "num_gws")?,
            wave_front_size: req_u32(&m, "wave_front_size")?,
            array_count: req_u32(&m, "array_count")?,
            simd_arrays_per_engine: req_u32(&m, "simd_arrays_per_engine")?,
            cu_per_simd_array: req_u32(&m, "cu_per_simd_array")?,
            simd_per_cu: req_u32(&m, "simd_per_cu")?,
            max_slots_scratch_cu: req_u32(&m, "max_slots_scratch_cu")?,
            gfx_target_version: req_u32(&m, "gfx_target_version")?,
            vendor_id: req_u32(&m, "vendor_id")?,
            device_id: req_u32(&m, "device_id")?,
            location_id: req_u32(&m, "location_id")?,
            domain: req_u32(&m, "domain")?,
            drm_render_minor: req_u32(&m, "drm_render_minor")?,
            hive_id: req_u64(&m, "hive_id")?,
            num_sdma_engines: req_u32(&m, "num_sdma_engines")?,
            num_sdma_xgmi_engines: req_u32(&m, "num_sdma_xgmi_engines")?,
            num_sdma_queues_per_engine: req_u32(&m, "num_sdma_queues_per_engine")?,
            num_cp_queues: req_u32(&m, "num_cp_queues")?,
            max_engine_clk_ccompute: req_u32(&m, "max_engine_clk_ccompute")?,
            max_engine_clk_fcompute: opt_u32(&m, "max_engine_clk_fcompute")?,
            local_mem_size: opt_u64(&m, "local_mem_size")?,
            fw_version: opt_u32(&m, "fw_version")?,
            capability: opt_u32(&m, "capability")?,
            capability2: opt_u32(&m, "capability2")?,
            debug_prop: opt_u32(&m, "debug_prop")?,
            sdma_fw_version: opt_u32(&m, "sdma_fw_version")?,
            unique_id: opt_u64(&m, "unique_id")?,
            num_xcc: opt_u32(&m, "num_xcc")?,
        })
    }
}

impl MemBankProperties {
    /// Parse from raw sysfs file bytes.
    pub fn from_bytes(bytes: &[u8]) -> ParseResult<Self> {
        let m = parse_kv(bytes);
        Ok(Self {
            heap_type: req_u32(&m, "heap_type")?,
            size_in_bytes: req_u64(&m, "size_in_bytes")?,
            flags: req_u32(&m, "flags")?,
            width: req_u32(&m, "width")?,
            mem_clk_max: req_u32(&m, "mem_clk_max")?,
        })
    }
}

impl LinkProperties {
    /// Parse from raw sysfs file bytes.
    pub fn from_bytes(bytes: &[u8]) -> ParseResult<Self> {
        let m = parse_kv(bytes);
        Ok(Self {
            link_type: req_u32(&m, "type")?,
            version_major: req_u32(&m, "version_major")?,
            version_minor: req_u32(&m, "version_minor")?,
            node_from: req_u32(&m, "node_from")?,
            node_to: req_u32(&m, "node_to")?,
            weight: req_u32(&m, "weight")?,
            min_latency: req_u32(&m, "min_latency")?,
            max_latency: req_u32(&m, "max_latency")?,
            min_bandwidth: req_u32(&m, "min_bandwidth")?,
            max_bandwidth: req_u32(&m, "max_bandwidth")?,
            recommended_transfer_size: req_u32(&m, "recommended_transfer_size")?,
            recommended_sdma_engine_id_mask: req_u32(&m, "recommended_sdma_engine_id_mask")?,
            flags: req_u32(&m, "flags")?,
        })
    }
}

impl CacheProperties {
    /// Parse from raw sysfs file bytes.
    pub fn from_bytes(bytes: &[u8]) -> ParseResult<Self> {
        let m = parse_kv(bytes);
        let sibling_map = m
            .get("sibling_map")
            .ok_or_else(|| TopologyParseError::new("missing field `sibling_map`"))?
            .split(',')
            .map(|s| {
                s.trim()
                    .parse::<u32>()
                    .map_err(|_| TopologyParseError::new("bad u32 in sibling_map"))
            })
            .collect::<ParseResult<Vec<u32>>>()?;
        Ok(Self {
            processor_id_low: req_u32(&m, "processor_id_low")?,
            level: req_u32(&m, "level")?,
            size: req_u32(&m, "size")?,
            cache_line_size: req_u32(&m, "cache_line_size")?,
            cache_lines_per_tag: req_u32(&m, "cache_lines_per_tag")?,
            association: req_u32(&m, "association")?,
            latency: req_u32(&m, "latency")?,
            cache_type: req_u32(&m, "type")?,
            sibling_map,
        })
    }
}

// ── TypedTopology conversion ──────────────────────────────────────────────────

impl TypedTopology {
    /// Parse a [`TypedTopology`] from the flat [`Topology`] wire format.
    ///
    /// Returns an error describing the first parse failure encountered.
    pub fn from_topology(topo: &Topology) -> ParseResult<Self> {
        let generation_id = topo
            .files
            .get("generation_id")
            .map(|b| parse_single_u32(b))
            .transpose()?
            .unwrap_or(0);

        let system_properties = topo
            .files
            .get("system_properties")
            .ok_or_else(|| TopologyParseError::new("missing `system_properties`"))
            .and_then(|b| SystemProperties::from_bytes(b))?;

        // Collect all node indices present in the file map.
        let mut node_indices = std::collections::BTreeSet::new();
        for key in topo.files.keys() {
            if let Some(rest) = key.strip_prefix("nodes/") {
                if let Some(idx_str) = rest.split('/').next() {
                    if let Ok(idx) = idx_str.parse::<u32>() {
                        node_indices.insert(idx);
                    }
                }
            }
        }

        let mut nodes = BTreeMap::new();
        for idx in node_indices {
            let prefix = format!("nodes/{idx}/");

            let properties = topo
                .files
                .get(&format!("{prefix}properties"))
                .ok_or_else(|| {
                    TopologyParseError::new(format!("missing `{prefix}properties`"))
                })
                .and_then(|b| NodeProperties::from_bytes(b))?;

            let name = topo
                .files
                .get(&format!("{prefix}name"))
                .map(|b| String::from_utf8_lossy(b).trim().to_owned())
                .unwrap_or_default();

            let gpu_id = topo
                .files
                .get(&format!("{prefix}gpu_id"))
                .map(|b| parse_single_u32(b))
                .transpose()?
                .unwrap_or(0);

            // ── mem_banks ──────────────────────────────────────────────────
            let mut mem_banks = BTreeMap::new();
            let mb_prefix = format!("{prefix}mem_banks/");
            let mut mb_indices = std::collections::BTreeSet::new();
            for key in topo.files.keys().filter(|k| k.starts_with(&mb_prefix)) {
                if let Some(rest) = key.strip_prefix(&mb_prefix) {
                    if let Some(i_str) = rest.split('/').next() {
                        if let Ok(i) = i_str.parse::<u32>() {
                            mb_indices.insert(i);
                        }
                    }
                }
            }
            for mi in mb_indices {
                let props = topo
                    .files
                    .get(&format!("{mb_prefix}{mi}/properties"))
                    .ok_or_else(|| {
                        TopologyParseError::new(format!(
                            "missing `{mb_prefix}{mi}/properties`"
                        ))
                    })
                    .and_then(|b| MemBankProperties::from_bytes(b))?;
                let used_memory = topo
                    .files
                    .get(&format!("{mb_prefix}{mi}/used_memory"))
                    .map(|b| parse_single_u64(b))
                    .transpose()?;
                mem_banks.insert(mi, MemBank { properties: props, used_memory });
            }

            // ── io_links ───────────────────────────────────────────────────
            let io_links =
                parse_link_map(&format!("{prefix}io_links/"), &topo.files)?;

            // ── p2p_links ──────────────────────────────────────────────────
            let p2p_links =
                parse_link_map(&format!("{prefix}p2p_links/"), &topo.files)?;

            // ── caches ─────────────────────────────────────────────────────
            let cache_prefix = format!("{prefix}caches/");
            let mut cache_indices = std::collections::BTreeSet::new();
            for key in topo.files.keys().filter(|k| k.starts_with(&cache_prefix)) {
                if let Some(rest) = key.strip_prefix(&cache_prefix) {
                    if let Some(i_str) = rest.split('/').next() {
                        if let Ok(i) = i_str.parse::<u32>() {
                            cache_indices.insert(i);
                        }
                    }
                }
            }
            let mut caches = BTreeMap::new();
            for ci in cache_indices {
                let cp = topo
                    .files
                    .get(&format!("{cache_prefix}{ci}/properties"))
                    .ok_or_else(|| {
                        TopologyParseError::new(format!(
                            "missing `{cache_prefix}{ci}/properties`"
                        ))
                    })
                    .and_then(|b| CacheProperties::from_bytes(b))?;
                caches.insert(ci, cp);
            }

            nodes.insert(
                idx,
                TopologyNode { properties, name, gpu_id, mem_banks, io_links, p2p_links, caches },
            );
        }

        Ok(TypedTopology { generation_id, system_properties, nodes })
    }

    /// Serialise back to the flat [`Topology`] wire format.
    ///
    /// The resulting [`Topology`] is semantically equivalent to the one that
    /// was originally parsed; individual files may differ in trailing
    /// whitespace from live sysfs but will parse to the same values.
    pub fn into_topology(self) -> Topology {
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();

        files.insert(
            "generation_id".into(),
            format!("{}\n", self.generation_id).into_bytes(),
        );
        files.insert(
            "system_properties".into(),
            self.system_properties.into(),
        );

        for (idx, node) in self.nodes {
            let prefix = format!("nodes/{idx}/");
            files.insert(format!("{prefix}properties"), node.properties.into());
            files.insert(
                format!("{prefix}name"),
                format!("{}\n", node.name).into_bytes(),
            );
            files.insert(
                format!("{prefix}gpu_id"),
                format!("{}\n", node.gpu_id).into_bytes(),
            );

            for (mi, bank) in node.mem_banks {
                files.insert(
                    format!("{prefix}mem_banks/{mi}/properties"),
                    bank.properties.into(),
                );
                if let Some(used) = bank.used_memory {
                    files.insert(
                        format!("{prefix}mem_banks/{mi}/used_memory"),
                        format!("{used}\n").into_bytes(),
                    );
                }
            }

            for (li, link) in node.io_links {
                files.insert(
                    format!("{prefix}io_links/{li}/properties"),
                    link.into(),
                );
            }

            for (li, link) in node.p2p_links {
                files.insert(
                    format!("{prefix}p2p_links/{li}/properties"),
                    link.into(),
                );
            }

            for (ci, cache) in node.caches {
                files.insert(
                    format!("{prefix}caches/{ci}/properties"),
                    cache.into(),
                );
            }
        }

        Topology { files }
    }
}

/// Parse a `BTreeMap<index, LinkProperties>` from all `<prefix><N>/properties`
/// entries in `files`.
fn parse_link_map(
    prefix: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> ParseResult<BTreeMap<u32, LinkProperties>> {
    let mut indices = std::collections::BTreeSet::new();
    for key in files.keys().filter(|k| k.starts_with(prefix)) {
        if let Some(rest) = key.strip_prefix(prefix) {
            if let Some(i_str) = rest.split('/').next() {
                if let Ok(i) = i_str.parse::<u32>() {
                    indices.insert(i);
                }
            }
        }
    }
    let mut map = BTreeMap::new();
    for i in indices {
        let lp = files
            .get(&format!("{prefix}{i}/properties"))
            .ok_or_else(|| {
                TopologyParseError::new(format!("missing `{prefix}{i}/properties`"))
            })
            .and_then(|b| LinkProperties::from_bytes(b))?;
        map.insert(i, lp);
    }
    Ok(map)
}
