//! End-to-end round-trip test for the KFD sysfs topology types.
//!
//! The fixture `fixtures/mi300x_8gpu.json` is a snapshot of the full
//! `/sys/class/kfd/kfd/topology/` tree captured from the build machine
//! (8× MI300X, 2 CPU nodes).
//!
//! The test:
//! 1. Loads the fixture into the flat [`Topology`] wire format.
//! 2. Parses it into a [`TypedTopology`].
//! 3. Asserts a handful of known values for this specific machine.
//! 4. Converts the [`TypedTopology`] back to [`Topology`].
//! 5. Re-parses the round-tripped [`Topology`] and asserts the two
//!    [`TypedTopology`] values are identical.

use std::collections::BTreeMap;

use mirage_schema::topology::{Topology, TypedTopology};

/// Load the fixture JSON (a `Map<String, String>` of relative path → text
/// content) and convert it into a [`Topology`].
fn load_fixture() -> Topology {
    let json_str = include_str!("fixtures/mi300x_8gpu.json");
    let text_map: BTreeMap<String, String> =
        serde_json::from_str(json_str).expect("fixture JSON is valid");
    let files = text_map
        .into_iter()
        .map(|(k, v)| (k, v.into_bytes()))
        .collect();
    Topology { files }
}

#[test]
fn parse_fixture_succeeds() {
    let raw = load_fixture();
    TypedTopology::from_topology(&raw).expect("topology parses without error");
}

#[test]
fn fixture_system_properties() {
    let raw = load_fixture();
    let typed = TypedTopology::from_topology(&raw).unwrap();

    // Values captured from the MI300X build machine.
    assert_eq!(typed.generation_id, 9);
    assert_eq!(typed.system_properties.platform_oem, 4_777_524_452_235_890_003);
    assert_eq!(typed.system_properties.platform_id, 85_015_745_549_651);
    assert_eq!(typed.system_properties.platform_rev, 2);
}

#[test]
fn fixture_node_count() {
    let raw = load_fixture();
    let typed = TypedTopology::from_topology(&raw).unwrap();

    // 2 CPU nodes (indices 0, 1) + 8 GPU nodes (indices 2–9).
    assert_eq!(typed.nodes.len(), 10);
}

#[test]
fn fixture_cpu_nodes() {
    let raw = load_fixture();
    let typed = TypedTopology::from_topology(&raw).unwrap();

    for idx in [0u32, 1u32] {
        let node = typed.nodes.get(&idx).unwrap_or_else(|| panic!("node {idx} missing"));
        let p = &node.properties;

        // CPU nodes have cores and no SIMDs.
        assert!(p.cpu_cores_count > 0, "node {idx} should have CPU cores");
        assert_eq!(p.simd_count, 0, "node {idx} should have no SIMDs");
        assert_eq!(node.gpu_id, 0, "node {idx} gpu_id should be 0");

        // GPU-only optional fields must be absent.
        assert!(p.max_engine_clk_fcompute.is_none(), "node {idx} should not have fcompute clock");
        assert!(p.fw_version.is_none(), "node {idx} should not have fw_version");
        assert!(p.capability.is_none(), "node {idx} should not have capability");
        assert!(p.unique_id.is_none(), "node {idx} should not have unique_id");
        assert!(p.num_xcc.is_none(), "node {idx} should not have num_xcc");
    }
}

#[test]
fn fixture_gpu_node_2() {
    let raw = load_fixture();
    let typed = TypedTopology::from_topology(&raw).unwrap();

    let node = typed.nodes.get(&2).expect("node 2 missing");
    let p = &node.properties;

    assert_eq!(node.gpu_id, 22_683);
    assert_eq!(node.name, "ip discovery");

    // GFX IP target version for MI300X (gfx904 → 90402).
    assert_eq!(p.gfx_target_version, 90402);

    // GPU node has SIMDs and no CPU cores.
    assert!(p.simd_count > 0);
    assert_eq!(p.cpu_cores_count, 0);

    // GPU-only fields must be present.
    assert!(p.max_engine_clk_fcompute.is_some());
    assert!(p.fw_version.is_some());
    assert!(p.capability.is_some());
    assert!(p.unique_id.is_some());
    assert!(p.num_xcc.is_some());
    assert_eq!(p.num_xcc, Some(8));

    // VRAM bank.
    assert!(!node.mem_banks.is_empty());
    let bank0 = node.mem_banks.get(&0).expect("mem_bank 0 missing");
    assert_eq!(bank0.properties.heap_type, 1); // public VRAM
    assert!(bank0.properties.size_in_bytes > 0);

    // Cache descriptors (MI300X has hundreds per GPU node).
    assert!(node.caches.len() > 100, "expected many cache entries");

    // IO links must be present.
    assert!(!node.io_links.is_empty());
}

#[test]
fn round_trip_typed_topology() {
    let raw = load_fixture();
    let typed = TypedTopology::from_topology(&raw).unwrap();

    // Serialise back to the flat wire format.
    let raw2 = typed.clone().into_topology();

    // Re-parse and check structural equality.
    let typed2 = TypedTopology::from_topology(&raw2).unwrap();
    assert_eq!(typed, typed2, "round-trip produced a different TypedTopology");
}

#[test]
fn into_vec_u8_system_properties_roundtrips() {
    use mirage_schema::topology::SystemProperties;

    let sp = SystemProperties {
        platform_oem: 4_777_524_452_235_890_003,
        platform_id: 85_015_745_549_651,
        platform_rev: 2,
    };
    let bytes: Vec<u8> = sp.clone().into();
    let sp2 = SystemProperties::from_bytes(&bytes).unwrap();
    assert_eq!(sp, sp2);
}

#[test]
fn into_vec_u8_node_properties_roundtrips() {
    use mirage_schema::topology::NodeProperties;

    // GPU node: all optional fields present.
    let p = NodeProperties {
        cpu_cores_count: 0,
        simd_count: 1216,
        mem_banks_count: 1,
        caches_count: 626,
        io_links_count: 8,
        p2p_links_count: 1,
        cpu_core_id_base: 0,
        simd_id_base: 2_147_487_744,
        max_waves_per_simd: 8,
        lds_size_in_kb: 64,
        gds_size_in_kb: 0,
        num_gws: 64,
        wave_front_size: 64,
        array_count: 32,
        simd_arrays_per_engine: 1,
        cu_per_simd_array: 10,
        simd_per_cu: 4,
        max_slots_scratch_cu: 32,
        gfx_target_version: 90402,
        vendor_id: 4098,
        device_id: 29857,
        location_id: 25856,
        domain: 0,
        drm_render_minor: 128,
        hive_id: 10_639_499_081_066_680_890,
        num_sdma_engines: 2,
        num_sdma_xgmi_engines: 14,
        num_sdma_queues_per_engine: 8,
        num_cp_queues: 24,
        max_engine_clk_ccompute: 2750,
        max_engine_clk_fcompute: Some(2100),
        local_mem_size: Some(0),
        fw_version: Some(192),
        capability: Some(2_893_521_536),
        capability2: Some(0),
        debug_prop: Some(1511),
        sdma_fw_version: Some(25),
        unique_id: Some(1_773_477_448_619_324_155),
        num_xcc: Some(8),
    };
    let bytes: Vec<u8> = p.clone().into();
    let p2 = NodeProperties::from_bytes(&bytes).unwrap();
    assert_eq!(p, p2);
}

#[test]
fn into_vec_u8_cache_properties_roundtrips() {
    use mirage_schema::topology::CacheProperties;

    let c = CacheProperties {
        processor_id_low: 2_147_487_951,
        level: 1,
        size: 32,
        cache_line_size: 128,
        cache_lines_per_tag: 0,
        association: 0,
        latency: 0,
        cache_type: 9,
        sibling_map: vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                          0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    };
    let bytes: Vec<u8> = c.clone().into();
    let c2 = CacheProperties::from_bytes(&bytes).unwrap();
    assert_eq!(c, c2);
}

#[test]
fn into_vec_u8_link_properties_roundtrips() {
    use mirage_schema::topology::LinkProperties;

    let l = LinkProperties {
        link_type: 2,
        version_major: 0,
        version_minor: 0,
        node_from: 2,
        node_to: 0,
        weight: 20,
        min_latency: 0,
        max_latency: 0,
        min_bandwidth: 0,
        max_bandwidth: 64_000,
        recommended_transfer_size: 0,
        recommended_sdma_engine_id_mask: 3,
        flags: 1,
    };
    let bytes: Vec<u8> = l.clone().into();
    let l2 = LinkProperties::from_bytes(&bytes).unwrap();
    assert_eq!(l, l2);
}

#[test]
fn into_vec_u8_mem_bank_properties_roundtrips() {
    use mirage_schema::topology::MemBankProperties;

    let m = MemBankProperties {
        heap_type: 1,
        size_in_bytes: 206_141_652_992,
        flags: 0,
        width: 8192,
        mem_clk_max: 1300,
    };
    let bytes: Vec<u8> = m.clone().into();
    let m2 = MemBankProperties::from_bytes(&bytes).unwrap();
    assert_eq!(m, m2);
}

#[test]
fn parse_error_on_missing_field() {
    // Omit `platform_oem` — must produce a TopologyParseError.
    let bad = b"platform_id 1\nplatform_rev 2\n";
    let err = mirage_schema::topology::SystemProperties::from_bytes(bad)
        .unwrap_err();
    assert!(
        err.message.contains("platform_oem"),
        "error should mention the missing field: {err}"
    );
}
