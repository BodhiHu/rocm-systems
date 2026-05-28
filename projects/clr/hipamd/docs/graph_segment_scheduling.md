# HIP Graph Segment Scheduling — Internals

## Overview

The segment scheduler is the "packet engine" path for `hipGraphLaunch`. Instead of
re-issuing `CreateCommand`/`EnqueueCommands` on every launch (the classic path), it
captures AQL packets once at instantiation time and replays them as flat buffers on
each launch, patching only the HW-event signals that change between runs.

This document covers the full lifecycle: segmentation → packet capture → sync plan →
flat-buffer dispatch.

---

## 1. Enabling Conditions

```
DEBUG_HIP_GRAPH_SEGMENT_SCHEDULING
  0 = disabled (always classic path)
  1 = enabled  (default) — falls back to classic if ≥16 segments AND avg length < 8
  2 = forced   (always segment path)

AMD_DIRECT_DISPATCH = 1  (required for AQL flat-buffer dispatch)
```

Entry point: `GraphExec::Init()` → calls `ScheduleNodesIntoBatches()` → if that
succeeds, calls `CaptureAQLPackets()`.

---

## 2. Graph → Segments (FindExecutionPathsHierarchical)

The graph's nodes are partitioned into **segments** — linear chains with no internal
branching. `FindPathsDFS` walks the graph and splits at fork/join nodes.

```
GRAPH NODES (example from test):

  K1 (spin+write d_a) ──┐   roots: no deps
  K2 (spin+write d_b) ──┤   both level 0
                         │
                    JOIN ▼
               DtoH d_a→h_a   ← first node of consumer segment
               accumulate      ← captured kernel
               DtoH d_sum→h_sum

SPLIT RULE (FindPathsDFS):
  ┌─ fork node  (edges > 1) → save current path, push each branch
  └─ join node  (deps  > 1) → save path WITHOUT join node, restart from join

RESULTING SEGMENTS:
  seg=0  { K2 }                          ← root, stream_id=0
  seg=2  { K1 }                          ← root, stream_id=1
  seg=1  { DtoH, accumulate, DtoH }      ← level 1, stream_id=0
           ^join node starts new segment
```

### 2.1 Stream Assignment (PrecomputeStreamAssignment)

`max_streams_dev_` = max concurrent segments at any level on a device.
With 2 segments at level 0 → pool size = 2 → two HW queues.

```
Level 0: seg=0 → stream_id=0 (launch stream)
         seg=2 → stream_id=1 (parallel HW queue)

Level 1: seg=1 → stream_id=0 % 2 = 0 (launch stream)

Cross-dep: seg=2 (stream 1) → seg=1 (stream 0)  ← CROSS-STREAM, needs barrier
Same-dep:  seg=0 (stream 0) → seg=1 (stream 0)  ← SAME-STREAM, no barrier needed
```

---

## 3. CaptureAndFormPacketsForGraph — Batch Layout

This function runs **once at instantiation** and decides:
1. How many `PacketBatch` objects to create per segment
2. Which nodes get their AQL packets captured vs executed live

### 3.1 Decision per node

```
node.GraphCaptureEnabled() == true?
  ├─ YES: captured → AQL packets extracted, stored in a PacketBatch
  └─ NO:  uncaptured → no packets; runs via CreateCommand/EnqueueCommands at launch
           Examples of uncaptured nodes:
             - hipGraphAddHostNode          (always uncaptured)
             - hipMemcpyDeviceToHost with non-registered host ptr (hipReadBuffer path)
             - Cooperative kernels
```

### 3.2 Batch creation rules

```
┌──────────────────────────────────────────────────────────────────────────┐
│  CaptureAndFormPacketsForGraph  per-segment logic                        │
│                                                                          │
│  1. first_node_is_uncaptured?                                            │
│       YES → emplace_back()   ← LEADING EMPTY BATCH (batch[0])           │
│                                 Slot for BuildSyncPlan dep-wait barrier  │
│                                                                          │
│  2. For each node in segment:                                            │
│       captured   → collect into newBatch                                 │
│                    inner while loop grabs all consecutive captured nodes │
│                    push_back(newBatch)  ← CONTENT BATCH                  │
│       uncaptured → set has_uncaptured_nodes = true                       │
│                    node_capture_status[i] = false                        │
│                    (no batch added — runs at dispatch time)              │
│                                                                          │
│  3. last_node_uncaptured?                                                │
│       YES → emplace_back()   ← TRAILING EMPTY BATCH (batch[N])          │
│                                 Slot for BuildSyncPlan completion barrier│
└──────────────────────────────────────────────────────────────────────────┘
```

### 3.3 Batch layout for each segment type

#### All-captured segment (e.g. seg=0, seg=2)

```
Nodes:  [ K1(captured) ]

packet_batches:
  [0]  { KERNEL_DISPATCH:K1 }    ← single content batch
                                    BuildSyncPlan may prepend BARRIER_AND here
                                    and/or append completion BARRIER_AND
```

#### Mixed segment with uncaptured first node (seg=1)

```
Nodes:  [ DtoH(uncaptured),  accumulate(captured),  DtoH(uncaptured) ]
          ^^^first uncap                               ^^^last uncap

packet_batches:
  [0]  { }                        ← LEADING EMPTY  (first_node uncaptured)
         BuildSyncPlan fills:  → { BARRIER_AND }
  [1]  { KERNEL_DISPATCH:accum }  ← CONTENT BATCH  (captured accumulate)
  [2]  { }                        ← TRAILING EMPTY (last_node uncaptured)
         BuildSyncPlan fills:  → { } or { BARRIER_AND } if completion needed
```

---

## 4. BuildSyncPlan — Filling the Batches

`BuildSyncPlan` runs **after** `CaptureAndFormPacketsForGraph`. It never looks at
individual nodes — it only sees batches and segment metadata.

### 4.1 PASS 1 — HW event slot assignment

```
For each segment:
  needs_completion_signal == true?
    → seg_to_hw_event[seg.id] = next_slot++

needs_completion_signal = true when ANY downstream segment is on a
different stream/device, OR this is a leaf requiring sync.
```

### 4.2 PASS 2 — Barrier materialization

For each segment, two decisions:

#### Front: dep-wait barrier → packet_batches[0]

```
barrier_dep_indices = cross-stream deps of this segment

if barrier_dep_indices not empty:
  ┌─ OPTIMIZATION PATH (single dep, first packet is ext-kernel-dispatch):
  │    Embed dep_signal directly into firstBatch.dispatchPackets[0]
  │    No separate barrier packet needed
  │    patch_list entry: { pkt=first_dispatch, hw_slot, kExtDispatchDepSignal }
  │
  └─ GENERAL PATH:
       barrier_count = ceil(num_deps / 5)   ← AQL barrier holds max 5 dep signals
       for each barrier:
         barrier_pkt = device->CreateBarrierPacket()
         patch_list entries: { pkt=barrier_pkt, hw_slot[dep], slot_index }
         firstBatch.dispatchPackets.insert(BEGIN, barrier_pkt)  ← PREPEND
         firstBatch.dispatchKernelNames.insert(BEGIN, nullptr)  ← sentinel
       Update nodeRanges[i].startIndex += barrier_count
```

**Key**: always goes into `packet_batches[0]`.
- If `packet_batches[0]` is the leading empty batch → barrier is its only packet
- If `packet_batches[0]` is a content batch → barrier is prepended before the kernels

#### Back: completion barrier → packet_batches.back()

```
if last_node_uncaptured AND needs_completion_signal:
  completion_barrier = device->CreateBarrierPacket()
  lastBatch.dispatchPackets.push_back(completion_barrier)   ← APPEND
  patch_list entry: { pkt=completion_barrier, hw_slot, kCompletionSignal }

if last_node_captured AND needs_completion_signal:
  completion_barrier appended to lastBatch
  (patching the last kernel packet directly would corrupt profiling metadata)
```

### 4.3 Complete flat buffer for seg=1 after BuildSyncPlan

```
packet_batches[0]  { BARRIER_AND }              ← dep-wait for seg=2 (K1 on stream 1)
packet_batches[1]  { KERNEL_DISPATCH:accum }    ← captured accumulate kernel
packet_batches[2]  { }                          ← empty (no completion signal needed)

patch_list:
  { packet=BARRIER_AND[0], hw_slot=seg2_slot, dep_signal_index=0 }
  (at launch: ApplyHwEventPatches writes K1's completion signal into this field)
```

---

## 5. Flat Buffer Rebuild

After `BuildSyncPlan`, `rebuildFlatBuffer()` is called for every non-empty batch:

```
flatPacketData  = contiguous byte array, 64 bytes per packet
                  header zeroed to INVALID (type=1) until dispatch commits it

validPacketFullHeaders = saved original headers, restored atomically during dispatch

dispatchPackets[i] → flatPacketData[i * 64 .. i * 64 + 63]
```

```
seg=1, batch[0] flat buffer (64 bytes):
  [0..1]   header = 0x0001  (INVALID until dispatch)
  [2]      amd_format = 0
  [3..7]   ...
  [8..47]  dep_signal slots [0..4] — ApplyHwEventPatches writes K1 signal here
  [48..55] completion_signal = 0
  [56..63] ...

seg=2, batch[0] flat buffer (128 bytes = 2 × 64):
  Packet 0 (KERNEL_DISPATCH: spin_add):
    [0..1]   header = 0x0001 (INVALID until dispatch)
    ...
  Packet 1 (BARRIER_AND: completion):
    [64..65] header = 0x0001
    [120..127] completion_signal slot ← ApplyHwEventPatches writes K1's signal here
```

---

## 6. EnqueueSegmentedGraph — Dispatch at Launch Time

Called on every `hipGraphLaunch`. Processes segments level by level.

```
Level 0: EnqueueSegment(seg=0, stream[0])   ← K2 on launch stream
         EnqueueSegment(seg=2, stream[1])   ← K1 on parallel HW queue

Level 1: EnqueueSegment(seg=1, stream[0])   ← consumer on launch stream
```

### 6.1 EnqueueSegment dispatch sequence

```
EnqueueSegment(segment, stream):
  │
  ├─ [THE FIX] first node uncaptured AND batch[0] has packets?
  │    → dispatchCurrentBatch()     ← dispatch batch[0] (BARRIER_AND)
  │      batchIndex becomes 1
  │
  ├─ for i in segment.nodes:
  │    ├─ node_capture_status[i] == true (captured):
  │    │    → dispatchCurrentBatch()   ← dispatch batch[batchIndex]
  │    │      batchIndex++
  │    │
  │    └─ node_capture_status[i] == false (uncaptured):
  │         → pre_marker.enqueue()
  │           node.CreateCommand(stream)
  │           node.EnqueueCommands(stream)
  │           post_marker.enqueue()
  │
  └─ dispatch remaining batches (trailing completion barrier if any)
```

### 6.2 dispatchCurrentBatch

```
dispatchCurrentBatch():
  batch = packet_batches[batchIndex]
  if batch.dispatchPackets.empty():
    batchIndex++; return   ← skip empty trailing batch

  ApplyHwEventPatches(batch)  ← write live HW event signals into flatPacketData
  stream->dispatchAqlPacketBatchFlat(batch.flatPacketData, ...)
  batchIndex++
```

---

## 7. The Ordering Bug and Fix

### Without the fix — wrong AQL queue order

```
seg=1 dispatch on HW queue 0 (stream[0]):

  [pre_marker]                    ← no cross-dep barrier before this
  [DtoH: read *d_a]               ← races with K1 still spinning on HW queue 1
  [post_marker]
  [BARRIER_AND: wait for K1]      ← barrier arrives AFTER DtoH already submitted
  [KERNEL_DISPATCH: accumulate]

HW queue 1 (stream[1]):
  [KERNEL_DISPATCH: spin_add]     ← still running while DtoH fires on queue 0
  [BARRIER_AND: completion sig]
```

```
TIMELINE (without fix):
  t=0   K1 starts spinning on HW queue 1
  t=0   K2 starts spinning on HW queue 0
  t=1   DtoH submitted to HW queue 0  ← no barrier, reads stale *d_a = 0
  t=2   BARRIER_AND submitted to HW queue 0
  t=500 K1 finishes, writes *d_a = 10
  t=501 BARRIER_AND clears (K1 signal received)
  t=502 accumulate runs — reads correct *d_a = 10 (barrier helped here)
  t=503 DtoH h_a reads whatever arrived (could be 0 or 10 depending on GPU)
```

### With the fix — correct AQL queue order

```
seg=1 dispatch on HW queue 0 (stream[0]):

  [BARRIER_AND: wait for K1]      ← dispatched FIRST by the fix
  [pre_marker]
  [DtoH: read *d_a]               ← guaranteed K1 has finished
  [post_marker]
  [KERNEL_DISPATCH: accumulate]

HW queue 1 (stream[1]):
  [KERNEL_DISPATCH: spin_add]
  [BARRIER_AND: completion sig]
```

```
TIMELINE (with fix):
  t=0   K1 starts spinning on HW queue 1
  t=0   K2 starts spinning on HW queue 0
  t=1   BARRIER_AND submitted to HW queue 0  ← fix dispatches batch[0] first
        GPU stalls HW queue 0 waiting for K1's completion signal
  t=500 K1 finishes, signals → BARRIER_AND clears
  t=501 DtoH executes — reads *d_a = 10  ✓
  t=502 accumulate runs — reads *d_a = 10  ✓
```

### The fix (hip_graph_internal.cpp:2044)

```cpp
// Dispatch the leading batch (cross-dep barrier from BuildSyncPlan) before
// the uncaptured first node so the AQL barrier precedes its GPU commands.
if (segBatch && !segBatch->node_capture_status.empty() &&
    !segBatch->node_capture_status[0] &&            // first node is uncaptured
    batchIndex < segBatch->packet_batches.size()) { // batch[0] exists
  status = dispatchCurrentBatch();   // dispatches batch[0] (BARRIER_AND)
}
```

---

## 8. Complete Per-Segment Summary

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  seg=0  { K2 }  stream=0  level=0                                           │
│                                                                             │
│  CaptureAndFormPacketsForGraph:                                             │
│    first_uncaptured=NO, last_uncaptured=NO                                  │
│    batch[0] = { KERNEL_DISPATCH:K2 }                                        │
│                                                                             │
│  BuildSyncPlan:                                                             │
│    cross-deps: none from other streams                                      │
│    completion signal: not needed (same-stream consumer seg=1 exists)       │
│    → batch[0] unchanged: { KERNEL_DISPATCH:K2 }                             │
│                                                                             │
│  EnqueueSegment:                                                            │
│    i=0 captured → dispatchCurrentBatch() → { KERNEL_DISPATCH:K2 }          │
└─────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────────┐
│  seg=2  { K1 }  stream=1  level=0                                           │
│                                                                             │
│  CaptureAndFormPacketsForGraph:                                             │
│    first_uncaptured=NO, last_uncaptured=NO                                  │
│    batch[0] = { KERNEL_DISPATCH:K1 }                                        │
│                                                                             │
│  BuildSyncPlan:                                                             │
│    cross-deps: none incoming                                                │
│    completion signal: YES (seg=1 on stream=0 depends on this)               │
│    → appends completion BARRIER_AND to batch[0]                             │
│    batch[0] = { KERNEL_DISPATCH:K1, BARRIER_AND:completion }                │
│                                                                             │
│  EnqueueSegment:                                                            │
│    i=0 captured → dispatchCurrentBatch()                                    │
│      → { KERNEL_DISPATCH:K1, BARRIER_AND:completion }                       │
│      ApplyHwEventPatches writes K1's HW event signal into BARRIER_AND       │
└─────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────────┐
│  seg=1  { DtoH, accum, DtoH }  stream=0  level=1                           │
│                                                                             │
│  CaptureAndFormPacketsForGraph:                                             │
│    first_uncaptured=YES → emplace_back()  batch[0]={}  (leading empty)     │
│    i=0 DtoH   uncaptured → has_uncaptured=true, status[0]=false            │
│    i=1 accum  captured   → batch[1]={ KERNEL_DISPATCH:accum }              │
│    i=2 DtoH   uncaptured → status[2]=false                                 │
│    last_uncaptured=YES  → emplace_back()  batch[2]={}  (trailing empty)    │
│                                                                             │
│  BuildSyncPlan:                                                             │
│    cross-deps: seg=2 (stream 1 ≠ stream 0) → need barrier                  │
│    firstBatch = batch[0] (empty) → use_ext_dep=false → create BARRIER_AND  │
│    batch[0] = { BARRIER_AND:dep-wait-K1 }                                  │
│    completion signal: not needed (seg=1 is leaf on launch stream → skipped) │
│    batch[2] stays empty                                                     │
│                                                                             │
│  EnqueueSegment (WITH FIX):                                                 │
│    first_uncaptured=YES, batch[0] has pkts                                  │
│    → dispatchCurrentBatch()  ← BARRIER_AND submitted first  ✓              │
│      batchIndex=1                                                           │
│    i=0 DtoH   uncaptured → pre_marker, CreateCommand, EnqueueCommands,     │
│                             post_marker  (after BARRIER_AND has fired)      │
│    i=1 accum  captured   → dispatchCurrentBatch()                          │
│                             batch[1]={ KERNEL_DISPATCH:accum }  batchIdx=2 │
│    i=2 DtoH   uncaptured → pre_marker, CreateCommand, EnqueueCommands,     │
│                             post_marker                                     │
│    remaining: dispatchCurrentBatch() → batch[2] empty → skip               │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 9. DOT Visualization

Set `DEBUG_HIP_GRAPH_DOT_PRINT=2` before `hipGraphLaunch` to dump a `.dot` file.
Each node is annotated with its `StreamId`, `HW Queue`, `SignalIsRequired`, and
`DeviceId`.  Nodes in the same segment are grouped into a coloured cluster labelled
with `Segment N`, `Stream: X`, and `Level: Y`.

---

## 10. Function Call Map

```
hipGraphInstantiate()
  └─ GraphExec::Init()
       ├─ ScheduleNodesIntoBatches()
       │    ├─ FindExecutionPathsHierarchical()  → segments_
       │    ├─ ResolveSegmentDependencies()      → segment_ids_dependencies/edges
       │    └─ PrecomputeStreamAssignment()      → stream_id, needs_completion_signal
       │
       └─ CaptureAQLPackets()
            └─ CaptureAndFormPacketsForGraph()
                 ├─ per segment: create leading/trailing empty batches
                 ├─ per captured node: CaptureAndFormPacket() → dispatchPackets[]
                 ├─ BuildSyncPlan()
                 │    ├─ PASS 1: assign HW event slots (seg_to_hw_event[])
                 │    └─ PASS 2: for each segment:
                 │         ├─ prepend dep-wait BARRIER_AND into packet_batches[0]
                 │         └─ append completion BARRIER_AND into packet_batches.back()
                 └─ rebuildFlatBuffer() per batch → flatPacketData[]

hipGraphLaunch()
  └─ GraphExec::Run()
       ├─ ApplyHwEventPatches()  ← write live HW signals into flat buffers
       └─ EnqueueSegmentedGraph()
            └─ for each level, for each segment:
                 EnqueueSegment(segment, stream)
                   ├─ [FIX] if first_node uncaptured: dispatchCurrentBatch(batch[0])
                   ├─ for each node:
                   │    captured   → dispatchCurrentBatch(batch[N])
                   │    uncaptured → CreateCommand + EnqueueCommands
                   └─ dispatch remaining trailing batches
```

---

## 11. Key Invariants

| Invariant | Enforced by |
|---|---|
| `packet_batches[0]` always exists | `BuildSyncPlan` creates one if missing (line 479) |
| Dep-wait barrier always in `packet_batches[0]` | `BuildSyncPlan` PASS 2 (line 483, 528) |
| Completion barrier always in `packet_batches.back()` | `BuildSyncPlan` PASS 2 (line 549, 554) |
| Leading empty batch exists iff first node uncaptured | `CaptureAndFormPacketsForGraph` (line 1419) |
| Trailing empty batch exists iff last node uncaptured | `CaptureAndFormPacketsForGraph` (line 1495) |
| Barrier dispatched before uncaptured first node | `EnqueueSegment` fix (line 2044) |
| `nodeRanges[i].startIndex` valid after barrier prepend | `BuildSyncPlan` fixup (line 535) |
