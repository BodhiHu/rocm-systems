# GPU Component Specification

This document describes the simulated GPU components available in **rocjitsu**, their
sockets (input/output ports), and their configuration options. All component types
are used in the declarative topology section of a simulation config JSON file (see
`schemas/simulation_config.fbs` for the full FlatBuffers schema).

---

## Table of Contents

1. [SOC](#soc)
2. [GPU Memory (VRAM)](#gpu-memory)
3. [I/O Die (IOD)](#io-die)
   - [Memory-Side Cache (MSC)](#memory-side-cache)
   - [HBM Controller](#hbm-controller)
4. [Accelerator Complex Die (XCD)](#accelerator-complex-die)
5. [L2 Cache](#l2-cache)
6. [Command Processor](#command-processor)
7. [Shader Engine](#shader-engine)
8. [Compute Unit](#compute-unit)
9. [Memory Types (Mtype)](#memory-types)
10. [Port Naming Conventions](#port-naming-conventions)

---

## SOC

**Type string:** `soc`  
**Implementation:** `SoC` (`lib/rocjitsu/src/rocjitsu/vm/soc.h`)  
**Role:** Top-level topology container. Constructs the full GPU hierarchy and wires
all inter-component connections.

### Sockets

The SOC has no external sockets of its own. It is a **composite** structural root
that wires its children together.

### Config Options

| Key | Type | Description |
|-----|------|-------------|
| `arch` | string | ISA architecture identifier: `"cdna3"`, `"cdna4"`, `"rv64i"`. |

> The SOC infers `num_xcds`, `num_iods`, and `exec_mode` from the top-level simulation
> config and the declarative topology.

---

## GPU Memory

**Type string:** `gpu_memory`  
**Implementation:** `GpuMemory` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/gpu_memory.h`)  
**Role:** Backing VRAM store. Sparse memory — pages are allocated on first access.

### Sockets

| Port | Direction | Protocol | Description |
|------|-----------|----------|-------------|
| `cpl` | **in** | memory | Receives read/write requests (structural; used in clocked mode). |

### Config Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `size_mb` | uint32 | — | Memory size limit in MB (reserved for future use). |
| `memory_side_cache_mb` | uint32 | `0` | MSC size in MB (`0` = MSC disabled). |

---

## I/O Die

**Type string:** `iod`  
**Implementation:** `Iod` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/iod.h`)  
**Role:** Memory-side infrastructure die. Contains the memory-side cache (MSC), HBM
controllers, and fabric routing. Each IOD serves a subset of XCDs and owns a portion
of the HBM stacks.

### Sockets

| Port | Direction | Protocol | Description |
|------|-----------|----------|-------------|
| `msc.cpl_<n>` | **in** | memory | One per assigned XCD. Receives L2 cache-miss and eviction traffic. Created dynamically via `create_cpl_port()`. |
| `peer_cpl` | **in** | memory | Receives cross-IOD peer traffic from other IODs. |
| `peer_req` | **out** | memory | Sends cross-IOD peer requests to other IODs. |
| `req_<n>` | **out** | memory | One per HBM stack. Structural port indicating HBM stack assignment. |

### Config Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `num_hbm_stacks` | uint32 | (required) | Number of HBM stacks attached to this IOD. |

### Internal Wiring

- MSC `req` → HBM Controller `cpl` (automatic, latency inherited from link).
- IODs are typically interconnected in a full mesh via `peer_req`/`peer_cpl` links.

### Sub-components

#### Memory-Side Cache

**Implementation:** `MemorySideCache` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/memory_side_cache.h`)  
**Role:** Last-level cache between the L2 and HBM. Write-back policy, striped locking
for thread safety (256 stripes).

| Property | Value |
|----------|-------|
| Line size | 128 bytes |
| Sets | 65,536 |
| Associativity | 16-way |
| Total size | **128 MB** |

| Port | Direction | Protocol | Description |
|------|-----------|----------|-------------|
| `cpl_<name>` | **in** | memory | One per source (XCD). Receives L2 requests. |
| `req` | **out** | memory | Sends misses/writebacks to the HBM Controller. |

#### HBM Controller

**Implementation:** `HbmController` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/hbm_controller.h`)  
**Role:** Wraps the shared `GpuMemory` and services memory requests from the MSC (or
directly from L2 when MSC is disabled).

| Port | Direction | Protocol | Description |
|------|-----------|----------|-------------|
| `cpl` | **in** | memory | Default completer. Receives requests from the MSC. |
| `cpl_<name>` | **in** | memory | Additional named completers (created dynamically). |

---

## Accelerator Complex Die

**Type string:** `xcd`  
**Implementation:** `Xcd` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/xcd.h`)  
**Role:** Independent compute chiplet. Contains a command processor, shader engines
(each holding compute units), and a shared L2 cache.

### Sockets

| Port | Direction | Protocol | Description |
|------|-----------|----------|-------------|
| `l2.req` | **out** | memory | L2 requester port. Connects to an IOD's MSC completer (or directly to HBM if no IODs are modeled). |

> All other traffic enters the XCD through its children's ports (CP dispatch, CU
> memory requests).

### Config Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `num_shader_engines` | uint32 | (required) | Number of shader engines in this XCD. |

> Per-SE and per-CU config is inherited from child component definitions in the topology.

### Internal Wiring

| Source | Destination | Latency | Weight | Description |
|--------|-------------|---------|--------|-------------|
| `cp.req_<n>` | `se[j].cu[k].cpl` | 1 | 2 | CP dispatches work to each CU. |
| `se[j].cu[k].req` | `l2.cpl_<n>` | 1 | 10 | Each CU's memory requests go to L2. |
| `se[j].cu[k].adj_req` | `se[j].cu[k+1].adj_cpl` | 1 | 2 | Adjacent CU forward link (partitioning hint). |
| `se[j].cu[k+1].adj_req_r` | `se[j].cu[k].adj_cpl_r` | 1 | 2 | Adjacent CU reverse link (partitioning hint). |

---

## L2 Cache

**Type string:** `l2_cache`  
**Implementation:** `L2Cache` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/l2_cache.h`)  
**Role:** Shared per-XCD cache. Serves L1 scalar and vector cache misses from all CUs
in the XCD. Write-back to the backing store (MSC or HBM).

### Sockets

| Port | Direction | Protocol | Description |
|------|-----------|----------|-------------|
| `cpl_<n>` | **in** | memory | One per CU in the XCD. Receives L1 miss traffic and eviction writebacks. Created dynamically. |
| `req` | **out** | memory | Sends misses/writebacks to the backing store (MSC or HBM). |

### Config Options

The L2 cache has a **fixed geometry** with no user-configurable options:

| Property | Value |
|----------|-------|
| Line size | 128 bytes |
| Sets | 2,048 |
| Associativity | 16-way |
| Total size | **4 MB** |

### Mtype Behavior at L2

| Mtype | L2 Behavior |
|-------|-------------|
| **RW** | Allocate-on-miss, write-back |
| **CC** | Write-through to HBM (coherence) |
| **UC** | Bypass — request forwarded directly to backing store |
| **NT** | Cached at L2 (L1 is bypassed, but L2 stores the line) |

### Thread Safety

- `read()`, `write()`, `fetch_line()`, `writeback_line()` are **not thread-safe**.
  All CUs sharing an L2 must be in the same simulation partition.
- `atomic_rmw()` **is thread-safe** (striped locking, 8 stripes by line address).

---

## Command Processor

**Type string:** `command_processor`  
**Implementation:** `CommandProcessor` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.h`)  
**Role:** Dispatches wavefront work to compute units. Receives dispatch packets and
activates wavefront slots on CUs through its requester ports.

### Sockets

| Port | Direction | Protocol | Description |
|------|-----------|----------|-------------|
| `req_<n>` | **out** | dispatch | One per registered CU. Sends dispatch activation messages. Ports are named `req_0`, `req_1`, etc., created when CUs are added. |

> The CP does not have an external input port. Work is submitted programmatically
> via `enqueue()` (pre-simulation) or `submit()` (async, thread-safe). A doorbell
> event triggers `step()` to process queued packets.

### Config Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `max_queued_packets` | uint32 | `0` | Maximum dispatch packets that can be queued. `0` = unlimited. |

### Dispatch Packet Fields

Each dispatch packet submitted to the CP contains:

| Field | Type | Description |
|-------|------|-------------|
| `kernel_entry_pc` | uint64 | Byte address of the kernel entry point. |
| `workgroup_count` | uint32 | Number of workgroups to dispatch. |
| `wfs_per_workgroup` | uint32 | Wavefronts per workgroup. |
| `sgprs_per_wf` | uint32 | Scalar GPRs per wavefront (from code object). |
| `vgprs_per_wf` | uint32 | Vector GPRs per wavefront (from code object). |
| `kernarg_addr` | uint64 | Kernarg segment base address (written to `s[0:1]`). |
| `num_user_sgprs` | uint32 | User SGPRs count (determines `workgroup_id_x` offset). |
| `workgroup_id_offset` | uint32 | Starting workgroup ID offset (for multi-XCD dispatch). |

---

## Shader Engine

**Type string:** `shader_engine`  
**Implementation:** `ShaderEngine` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/shader_engine.h`)  
**Role:** Structural container grouping compute units within an XCD. Has no
computational logic of its own (weight = 0).

### Sockets

None. The shader engine is purely structural. CU ports are exposed directly.

### Config Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `num_compute_units` | uint32 | (required) | Number of compute units in this shader engine. |

> Per-CU config is inherited from `compute_unit` child definitions in the topology.

---

## Compute Unit

**Type string:** `compute_unit`  
**Implementation:** `ComputeUnitCore` (`lib/rocjitsu/src/rocjitsu/vm/amdgpu/compute_unit.h`)  
**Role:** Manages wavefront slots, instruction execution, scalar/vector register
files, and per-CU caches (L1 scalar, L1 vector, LDS).

### Sockets

| Port | Direction | Protocol | Description |
|------|-----------|----------|-------------|
| `cpl` | **in** | dispatch | Receives dispatch activation from the command processor. |
| `req` | **out** | memory | Sends L1 cache misses and memory requests to the L2. |
| `adj_req` | **out** | untyped | Forward adjacency link to the next CU in the same SE. Synthetic, for partitioner use. |
| `adj_cpl` | **in** | untyped | Forward adjacency completer from the previous CU. |
| `adj_req_r` | **out** | untyped | Reverse adjacency link to the previous CU. |
| `adj_cpl_r` | **in** | untyped | Reverse adjacency completer from the next CU. |

### Config Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `num_wf_slots` | uint32 | (required) | Number of hardware wavefront slots (contexts). Determines how many wavefronts can execute concurrently on this CU. |
| `sgprs_per_wf` | uint32 | (required) | Scalar general-purpose registers per wavefront. Defines allocation granularity in the physical SGPR file. |
| `vgprs_per_wf` | uint32 | (required) | Vector general-purpose registers per wavefront. Defines allocation granularity in the physical VGPR file. |
| `lds_size_kb` | uint32 | (required) | Local Data Share size in kilobytes. Shared scratchpad memory for workgroup communication. |
| `functional_quantum` | uint32 | `0` | Maximum instructions per `advance()` call in functional mode. `0` = unbounded (drain all active wavefronts in one call). A non-zero value guarantees forward progress for inter-CU synchronization (e.g., spin-locks on global memory). |

### Internal Resources

| Resource | Description |
|----------|-------------|
| **SGPR file** | Physical scalar register file (`num_wf_slots × sgprs_per_wf` entries). |
| **VGPR file** | Physical vector register file (`num_wf_slots × vgprs_per_wf × wave_size` entries). |
| **L1 Scalar Cache (K$)** | Per-CU scalar cache. Write-through to L2. |
| **L1 Vector Cache (V$)** | Per-CU vector cache. Write-through to L2. |
| **LDS** | Local Data Share — per-CU shared memory for workgroup communication. |

### Mtype Behavior at L1

| Mtype | L1 Behavior |
|-------|-------------|
| **RW** | Cached in L1, write-through to L2 |
| **CC** | Invalidate-on-read, write-through to L2 |
| **UC** | Bypass L1 entirely, go direct to backing store |
| **NT** | Bypass L1, request goes to L2 only |

---

## Memory Types

The AMD Memory Type (Mtype) is derived from instruction encoding bits (`sc1`, `sc0`,
`nt`). It controls caching behavior at each layer of the memory hierarchy.

See `lib/rocjitsu/src/rocjitsu/vm/amdgpu/mtype.h` for the enum definition.

| sc1 | sc0 | nt | Mtype | Description |
|-----|-----|----|-------|-------------|
| 0 | 0 | 0 | **RW** | L1 + L2 cached, write-back. Default for most accesses. |
| 0 | 1 | 0 | **CC** | Coherently cacheable. L1 write-through, L2 write-through to HBM. Visible to all agents. |
| 1 | 0 | 0 | **UC** | Uncacheable. Bypasses all caches, goes directly to memory. |
| 0 | 0 | 1 | **NT** | Non-temporal. Bypasses L1 but caches at L2. |

---

## Port Naming Conventions

| Pattern | Direction | Protocol | Usage |
|---------|-----------|----------|-------|
| `req` | out | memory | Send requests to the backing store (e.g., L2 → MSC). |
| `cpl` | in | memory/dispatch | Receive requests (generic completer). |
| `cpl_<name>` | in | memory | Named completer for multiple sources (e.g., `cpl_0`, `cpl_cu3`). |
| `req_<n>` | out | dispatch | Dispatch requester (CP → CU). |
| `adj_req` / `adj_cpl` | out / in | untyped | Forward adjacency link between neighboring CUs. |
| `adj_req_r` / `adj_cpl_r` | out / in | untyped | Reverse adjacency link between neighboring CUs. |
| `peer_req` / `peer_cpl` | out / in | memory | Inter-IOD cross-die traffic. |

### Link Properties

Links connecting ports have two key properties:

- **latency** — Simulated propagation delay in ticks.
- **weight** — Used by the graph partitioner to estimate communication cost when
  assigning components to simulation threads.
