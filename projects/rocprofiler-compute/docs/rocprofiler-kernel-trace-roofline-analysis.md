# ROCm Profiler 系列：Kernel Launch API Trace 捕捉與 Roofline 模型分析

> **分析日期**: 2026-08-11
> **分析範圍**: rocprofiler、rocprofiler-compute、rocprofiler-systems、rocprofiler-sdk
> **重點架構**: gfx1250 (MI450) — 新增專題分析

---

## 目錄

1. [專案總覽](#1-專案總覽)
2. [Kernel Launch API Trace 捕捉機制](#2-kernel-launch-api-trace-捕捉機制)
   - [2.1 rocprofiler-sdk：三層攔截架構](#21-rocprofiler-sdk三層攔截架構)
   - [2.2 rocprofiler (v1/v2)：ROCTracer 兼容層](#22-rocprofiler-v1v2roctracer-兼容層)
   - [2.3 rocprofiler-systems：Perfetto 事件匯出](#23-rocprofiler-systemsperfetto-事件匯出)
   - [2.4 rocprofiler-compute：SDK 整合](#24-rocprofiler-computesdk-整合)
3. [Roofline 模型計算](#3-roofline-模型計算)
   - [3.1 Roofline 理論基礎](#31-roofline-理論基礎)
   - [3.2 三層計算架構](#32-三層計算架構)
   - [3.3 具體 Hardware Counter](#33-具體-hardware-counter)
   - [3.4 核心計算公式](#34-核心計算公式)
   - [3.5 關鍵原始碼位置總表](#35-關鍵原始碼位置總表)
   - [3.6 以 MI300 (gfx942) 為例的完整計算流程](#36-以-mi300-gfx942-為例的完整計算流程)
4. [附錄：Counter 分類與含義](#4-附錄counter-分類與含義)
5. [gfx1250 (MI450) 架構專題分析](#5-gfx1250-mi450-架構專題分析-) ⭐ 完整實作
   - [5.1 硬體識別與架構定位](#51-硬體識別與架構定位)
   - [5.2 記憶體階層：GL0 → GL1 → GL2 → HBM](#52-記憶體階層gl0--gl1--gl2--hbm)
   - [5.3 Roofline Counter 定義（實際公式）](#53-roofline-counter-定義實際公式)
   - [5.4 Arithmetic Intensity 公式](#54-arithmetic-intensity-ai-公式)
   - [5.5 Peak 計算方式](#55-peak-計算方式)
   - [5.6 Benchmark 參數](#56-benchmark-參數)
   - [5.7 gfx1250 vs gfx942 完整對比](#57-gfx1250-vs-gfx942-完整對比)
   - [5.8 完整原始碼位置總表](#58-gfx1250-完整原始碼位置總表)
   - [5.9 Roofline 計算完整流程](#59-roofline-計算完整流程-gfx1250)

---

## 1. 專案總覽

| 專案 | 角色 | 語言 |
|------|------|------|
| **rocprofiler-sdk** | 新一代 profiling/tracing SDK，為其他三個專案的底層基礎。**已支援 gfx1250 (MI450)** | C++ (含 Python bindings) |
| **rocprofiler** | 傳統 CLI 工具 (rocprof/rocprofv2)，基於 ROCTracer | C++ |
| **rocprofiler-compute** | GPU Kernel 效能分析 + Roofline 模型。**已完整支援 gfx1250！** | Python + C++ |
| **rocprofiler-systems** | 全系統 CPU/GPU tracing (原 Omnitrace)，匯出 Perfetto | C++ |

### 依賴關係

```
rocprofiler-compute  ──▶  rocprofiler-sdk  ◀──  rocprofiler-systems
        │                        │
        ▼                        ▼
   Roofline 計算         HSA Queue 攔截
   Counter 評估          HIP/HSA API 攔截
                         KFD ioctl 攔截
```

---

## 2. Kernel Launch API Trace 捕捉機制

### 2.1 rocprofiler-sdk：三層攔截架構

rocprofiler-sdk 是整個 ROCm profiling 生態系統的核心。它使用**三層攔截**來完整捕捉 kernel launch：

```
┌─────────────────────────────────────────────────────┐
│                   Layer 1: API Table Interception   │
│  (GOTCHA-based HIP/HSA function wrapping)           │
│  hip.cpp / hsa.cpp                                  │
├─────────────────────────────────────────────────────┤
│                   Layer 2: HSA Queue Interposition  │
│  (Write-pointer virtualization + AQL packet scan)   │
│  queue_interposition.cpp                            │
├─────────────────────────────────────────────────────┤
│                   Layer 3: KFD ioctl Interception   │
│  (Kernel name/symbol resolution)                    │
│  kfd.cpp                                            │
└─────────────────────────────────────────────────────┘
```

#### Layer 1：HIP/HSA API Table 攔截 (GOTCHA)

透過巨集展開的 API 攔截表，在 HIP runtime / HSA core API 被呼叫時注入 tracing 回呼。

**HIP Runtime API 攔截** ([`hip/hip.def.cpp:74-80`](source/lib/rocprofiler-sdk/hip/hip.def.cpp#L74-L80))：

```cpp
HIP_API_TABLE_LOOKUP_DEFINITION(ROCPROFILER_HIP_TABLE_ID_Compiler, ...)
HIP_API_TABLE_LOOKUP_DEFINITION(ROCPROFILER_HIP_TABLE_ID_Runtime, ...)
```

兩個 table domain：
- `ROCPROFILER_HIP_TABLE_ID_Runtime` → 對應 `hip_runtime_api_table_t`（包含 `hipLaunchKernel`, `hipMemcpy`, `hipStreamCreate` 等）
- `ROCPROFILER_HIP_TABLE_ID_Compiler` → 對應 `hip_compiler_api_table_t`

每條 API 都對應到一個 `HIP_API_INFO_DEFINITION_V` 巨集，定義了 callback domain、buffered domain、external correlation domain 三種通知路徑。

**HSA Core API 攔截** ([`hsa/hsa.def.cpp:33-69`](source/lib/rocprofiler-sdk/hsa/hsa.def.cpp#L33-L69))：

```cpp
HSA_API_TABLE_LOOKUP_DEFINITION(ROCPROFILER_HSA_TABLE_ID_Core, ::CoreApiTable, core)
HSA_API_TABLE_LOOKUP_DEFINITION(ROCPROFILER_HSA_TABLE_ID_AmdExt, ::AmdExtTable, amd_ext)
HSA_API_TABLE_LOOKUP_DEFINITION(ROCPROFILER_HSA_TABLE_ID_ImageExt, ::ImageExtTable, img_ext)
HSA_API_TABLE_LOOKUP_DEFINITION(ROCPROFILER_HSA_TABLE_ID_FinalizeExt, ::FinalizerExtTable, fini_ext)
```

五個 HSA table domains：
- `HSA_TABLE_ID_Core` — `hsa_queue_create`, `hsa_signal_create`, `hsa_executable_*` 等
- `HSA_TABLE_ID_AmdExt` — AMD 擴展 API
- `HSA_TABLE_ID_ImageExt` / `HSA_TABLE_ID_FinalizeExt` / `HSA_TABLE_ID_AmdTool`

**兩種 tracing 模式**：
1. **Callback Tracing** (`api_callback_tracing/client.cpp`): 在 API 進入/退出時同步回呼，可以檢查參數
2. **Buffered Tracing** (`api_buffered_tracing/client.cpp`): 非同步寫入 ring buffer，低 overhead

關鍵原始碼：
- [`samples/api_buffered_tracing/client.cpp`](samples/api_buffered_tracing/client.cpp) — Buffered tracing 範例（行 143-348 處理 HSA/HIP API、Kernel Dispatch、Memory Copy、Scratch Memory 等記錄類型）
- [`samples/api_callback_tracing/client.cpp`](samples/api_callback_tracing/client.cpp) — Callback tracing 範例（行 103-156 在 ENTER/EXIT phase 記錄時間戳和參數）
- [`samples/intercept_table/client.cpp`](samples/intercept_table/client.cpp) — **直接攔截 HIP runtime table**（行 221-250 攔截所有 kernel launch API：`hipLaunchKernel`, `hipExtLaunchKernel`, `hipModuleLaunchKernel`, `hipLaunchCooperativeKernel` 等）

#### Layer 2：HSA Queue Interposition（核心機制）

這是最關鍵的 kernel dispatch 攔截機制。它**虛擬化 HSA queue 的 write pointer**，在 packet 被提交到硬體之前掃描和記錄 AQL (Architected Queuing Language) kernel dispatch packet。

關鍵原始碼：[`hsa/queue_interposition.cpp`](source/lib/rocprofiler-sdk/hsa/queue_interposition.cpp) (1277 行)

**核心概念**（行 23-30 註解）：

```
SDK-level HSA queue interposition: wraps hsa_queue_*_write_index_* and
hsa_signal_store_* to virtualize the queue write pointer. Producer threads
advance QueueState::virtual_wptr; the real write_dispatch_id only advances
at doorbell time after process_doorbell_impl runs the WriteInterceptor chain.
```

**關鍵函數**：

| 函數 | 行號 | 功能 |
|------|------|------|
| `lookup_queue_state()` | 126 | 查詢/建立 per-queue 追蹤狀態 |
| `lookup_queue_state_by_doorbell()` | 143 | 透過 doorbell signal 反向查找 queue |
| `add_write_index_impl()` | 163 | 攔截 `hsa_queue_add_write_index_*` |
| `store_write_index_impl()` | 169 | 攔截 `hsa_queue_store_write_index_*` |
| `cas_write_index_impl()` | 175 | 攔截 `hsa_queue_cas_write_index_*` |
| `process_doorbell_impl()` | 832 | **核心**：處理 doorbell 信號，掃描 AQL packets，提取 kernel dispatch 資訊 |
| `ring_published_doorbell()` | 244 | 實際 ring doorbell 使 packet 對硬體可見 |

**攔截的 HSA API**（行 109 附近）：
- `hsa_queue_create` / `hsa_queue_destroy` — 追蹤 queue 生命週期
- `hsa_signal_create` / `hsa_signal_destroy` — 追蹤 completion signal
- `hsa_amd_queue_intercept_create` — 舊版攔截介面
- `hsa_amd_memory_async_copy` — 非同步 memory copy

#### Layer 3：Kernel Dispatch Tracing

在 queue interposition 解析出 AQL kernel dispatch packet 後，kernel dispatch tracing 子系統負責時間戳記錄與 dispatch 完成通知。

關鍵原始碼：[`kernel_dispatch/tracing.cpp`](source/lib/rocprofiler-sdk/kernel_dispatch/tracing.cpp)

| 函數 | 行號 | 功能 |
|------|------|------|
| `get_dispatch_time()` | 45 | 取得 kernel dispatch 的開始/結束時間戳 |
| `dispatch_complete()` | 58 | 處理 kernel dispatch 完成事件，組裝 `rocprofiler_buffer_tracing_kernel_dispatch_record_t` |

Dispatch record 包含 (見 `api_buffered_tracing/client.cpp:257-289`)：
- `kernel_id` — kernel 識別碼
- `kernel_name` — 從 code object 符號表解析出的 kernel 名稱
- `agent_id` / `queue_id` — 執行的 GPU agent 和 HSA queue
- `workgroup_size` / `grid_size` — launch configuration
- `start_timestamp` / `end_timestamp` — 精確時間戳
- `correlation_id` — 與 host API call 的關聯 ID

### 2.2 rocprofiler (v1/v2)：ROCTracer 兼容層

rocprofiler v1/v2 是較舊的工具，使用 ROCTracer 相容的攔截機制。

關鍵原始碼路徑：

```
src/core/session/tracer/src/roctracer.cpp
```

**核心函數 `GetHipKernelName()`**（約行 843）：解析所有 HIP launch API 參數來獲取 kernel 名稱，支援：

| HIP API | 辨識方式 |
|---------|---------|
| `hipLaunchKernel` | `function_address` |
| `hipExtLaunchKernel` | `function_address` |
| `hipLaunchCooperativeKernel` | `function_address` |
| `hipLaunchByPtr` | `hostFunction` |
| `hipGraphAddKernelNode` | `kernelNodeParams` |
| `hipModuleLaunchKernel` | `function` (hipFunction_t) |
| `hipExtModuleLaunchKernel` | `function` |
| `hipHccModuleLaunchKernel` | `function` |
| `hipExtLaunchMultiKernelMultiDevice` | `launchParamsList` |

註冊的 tracing domains：
- `ACTIVITY_DOMAIN_HSA_API` / `ACTIVITY_DOMAIN_HSA_OPS` / `ACTIVITY_DOMAIN_HSA_EVT`
- `ACTIVITY_DOMAIN_HIP_API` / `ACTIVITY_DOMAIN_HIP_OPS`
- `ACTIVITY_DOMAIN_EXT_API`

Packet-level 攔截 ([`src/core/intercept_queue.cpp`](src/core/intercept_queue.cpp))：解析 `hsa_kernel_dispatch_packet_t`，捕獲 `completion_signal` 和 `kernel_object`。

### 2.3 rocprofiler-systems：Perfetto 事件匯出

rocprofiler-systems (原 Omnitrace) 基於 rocprofiler-sdk，將 kernel dispatch 事件轉換為 **Perfetto trace** 格式。

核心原始碼：[`source/lib/rocprof-sys/library/rocprofiler-sdk.cpp`](source/lib/rocprof-sys/library/rocprofiler-sdk.cpp) (3107 行)

處理：
- `ROCPROFILER_CALLBACK_TRACING_KERNEL_DISPATCH` — kernel dispatch callback 記錄
- `ROCPROFILER_BUFFER_TRACING_KERNEL_DISPATCH` — kernel dispatch buffer 記錄
- `ROCPROFILER_CODE_OBJECT_DEVICE_KERNEL_SYMBOL_REGISTER` — kernel 名稱解析

產出 Perfetto "GPU Kernel Dispatch [Queue N]" 事件軌道。

### 2.4 rocprofiler-compute：SDK 整合

rocprofiler-compute 有兩個 profiler 後端：

1. **rocprofiler-sdk 後端** ([`profiler_rocprofiler_sdk.py`](src/rocprof_compute_profile/profiler_rocprofiler_sdk.py))：使用 rocprofv3 CLI 收集 counter 數據
2. **rocprof v3 後端** ([`profiler_rocprof_v3.py`](src/rocprof_compute_profile/profiler_rocprof_v3.py))：使用 rocprofv3 進行 PMC 收集

它透過 rocprofiler-sdk 的 kernel dispatch tracing 取得 kernel 執行時間，再結合從 hardware counter 取得的 FLOPs 和 data traffic 來計算 Roofline。

---

## 3. Roofline 模型計算

### 3.1 Roofline 理論基礎

**Roofline Model** 是一個效能分析模型，用於判斷 kernel 是**計算瓶頸 (Compute Bound)** 還是**記憶體頻寬瓶頸 (Memory Bound)**。

核心公式：

```
Arithmetic Intensity (AI) = Total FLOPs / Total Bytes Transferred

Performance (GFLOP/s) = Total FLOPs / Execution Time (seconds)
```

- 如果 kernel 的 AI 值落在頻寬屋頂線的斜坡區域（AI < 臨界值）→ **Memory Bound**
- 如果 kernel 的 AI 值落在計算屋頂線的水平區域（AI ≥ 臨界值）→ **Compute Bound**

屋頂線 (Ceilings)：
- **頻寬屋頂 (Bandwidth Ceiling)**: `Performance ≤ Peak_BW × AI`（對角線，斜率 = Peak_BW）
- **計算屋頂 (Compute Ceiling)**: `Performance ≤ Peak_FLOPs`（水平線）

### 3.2 三層計算架構

rocprofiler-compute 的 Roofline 計算分為三個層級：

```
┌──────────────────────────────────────────────────────┐
│  Tier A: 繪圖與視覺化                                 │
│  roofline_main.py — Roofline 類別                    │
│  roofline_calc.py — calc_ceilings(), calc_ai_analyze()│
├──────────────────────────────────────────────────────┤
│  Tier B: 經驗峰值量測 (Empirical Peak Benchmarking)   │
│  benchmark_base.py — Bench_base                      │
│  量測實際 Peak FLOPs 和 Peak Bandwidth               │
│  產出 roofline.csv                                   │
├──────────────────────────────────────────────────────┤
│  Tier C: 應用效能計數器評估                           │
│  0400_roofline.yaml — 定義 per-arch counter 公式     │
│  MetricEvaluator — 評估 counter 表達式                │
│  產出 AI (FLOPs/Byte) 和 Performance (GFLOP/s)       │
└──────────────────────────────────────────────────────┘
```

### 3.3 具體 Hardware Counter

以下是 **gfx942 (MI300 系列)** 用於 Roofline 計算的關鍵 hardware counters：

#### 計算 FLOPs (VALU) 的 Counters — SQ (Sequencer) Block

| Counter | 含義 |
|---------|------|
| `SQ_INSTS_VALU_ADD_F16` | FP16 ADD 指令數 |
| `SQ_INSTS_VALU_MUL_F16` | FP16 MUL 指令數 |
| `SQ_INSTS_VALU_FMA_F16` | FP16 FMA 指令數（計為 2 FLOPs） |
| `SQ_INSTS_VALU_TRANS_F16` | FP16 TRANS 指令數 |
| `SQ_INSTS_VALU_ADD_F32` | FP32 ADD 指令數 |
| `SQ_INSTS_VALU_MUL_F32` | FP32 MUL 指令數 |
| `SQ_INSTS_VALU_FMA_F32` | FP32 FMA 指令數（計為 2 FLOPs） |
| `SQ_INSTS_VALU_TRANS_F32` | FP32 TRANS 指令數 |
| `SQ_INSTS_VALU_ADD_F64` | FP64 ADD 指令數 |
| `SQ_INSTS_VALU_MUL_F64` | FP64 MUL 指令數 |
| `SQ_INSTS_VALU_FMA_F64` | FP64 FMA 指令數（計為 2 FLOPs） |
| `SQ_INSTS_VALU_TRANS_F64` | FP64 TRANS 指令數 |

#### 計算 FLOPs (MFMA) 的 Counters

| Counter | 含義 |
|---------|------|
| `SQ_INSTS_VALU_MFMA_MOPS_F16` | MFMA FP16 Matrix Ops |
| `SQ_INSTS_VALU_MFMA_MOPS_BF16` | MFMA BF16 Matrix Ops |
| `SQ_INSTS_VALU_MFMA_MOPS_F32` | MFMA FP32 Matrix Ops |
| `SQ_INSTS_VALU_MFMA_MOPS_F64` | MFMA FP64 Matrix Ops |
| `SQ_INSTS_VALU_MFMA_MOPS_F8` | MFMA FP8 Matrix Ops |
| `SQ_INSTS_VALU_MFMA_MOPS_I8` | MFMA INT8 Matrix Ops |

#### 計算 HBM Bandwidth 的 Counters — TCC (Texture Cache Controller) Block

| Counter | 含義 |
|---------|------|
| `TCC_BUBBLE_sum` | TCC pipeline bubble cycles 導致的讀取 |
| `TCC_EA0_RDREQ_sum` | HBM 讀取請求總數 |
| `TCC_EA0_RDREQ_32B_sum` | 32-byte HBM 讀取請求 |
| `TCC_EA0_WRREQ_sum` | HBM 寫入請求總數 |
| `TCC_EA0_WRREQ_64B_sum` | 64-byte HBM 寫入請求 |

#### 計算 L2 Cache Bandwidth 的 Counters — TCP Block

| Counter | 含義 |
|---------|------|
| `TCP_TCC_WRITE_REQ_sum` | L2 寫入請求 |
| `TCP_TCC_ATOMIC_WITH_RET_REQ_sum` | L2 atomic (with return) 請求 |
| `TCP_TCC_ATOMIC_WITHOUT_RET_REQ_sum` | L2 atomic (no return) 請求 |
| `TCP_TCC_READ_REQ_sum` | L2 讀取請求 |

#### 計算 L1 Cache Bandwidth 的 Counters

| Counter | 含義 |
|---------|------|
| `TCP_TOTAL_CACHE_ACCESSES_sum` | vL1D cache access 總數 |

#### 計算 LDS Bandwidth 的 Counters

| Counter | 含義 |
|---------|------|
| `SQ_LDS_IDX_ACTIVE` | LDS 活躍 cycle 數 |
| `SQ_LDS_BANK_CONFLICT` | LDS bank conflict cycle 數 |

#### 系統級變數

| 變數 | 含義 | 來源 |
|------|------|------|
| `$wave_size` | Wavefront 大小（通常 64 for gfx9, 32 for gfx11） | sysinfo.csv |
| `$lds_banks_per_cu` | 每個 CU 的 LDS bank 數 | sysinfo.csv |
| `End_Timestamp` / `Start_Timestamp` | Kernel dispatch 時間戳 | SDK kernel dispatch record |
| `GRBM_GUI_ACTIVE` | GPU 活躍 cycle 數 | GRBM block |

### 3.4 核心計算公式

所有公式來自 `src/rocprof_compute_soc/analysis_configs/<arch>/0400_roofline.yaml`。

#### 3.4.1 VALU FLOPs (per datatype)

以 FP32 為例（[gfx942:0400_roofline.yaml:22](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L22)）：

```
VALU FLOPs (F32) = SUM(
    $wave_size * (
        SQ_INSTS_VALU_ADD_F32 +
        SQ_INSTS_VALU_MUL_F32 +
        2 * SQ_INSTS_VALU_FMA_F32 +    ← FMA 計為 2 FLOPs
        SQ_INSTS_VALU_TRANS_F32
    )
) / SUM(End_Timestamp - Start_Timestamp)
```

說明：
- 每條指令在一個 wavefront 中對 $wave_size (64) 個 thread 同時執行
- FMA (Fused Multiply-Add) 計為 2 個 FLOPs
- ADD、MUL、TRANS 各計為 1 個 FLOP
- `SUM()` 聚合所有 kernel dispatch 的數據
- 除以總執行時間得到 GFLOP/s

#### 3.4.2 MFMA FLOPs (per datatype)

以 FP16 為例（[gfx942:0400_roofline.yaml:38](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L38)）：

```
MFMA FLOPs (F16) = 512 * SUM(SQ_INSTS_VALU_MFMA_MOPS_F16)
                   / SUM(End_Timestamp - Start_Timestamp)
```

說明：
- MFMA 每條指令處理一個 16×16×16 矩陣乘法（MI200）或 32×32×8 矩陣乘法（MI300）
- 常數 512 是每條 MFMA 指令的 FLOPs 數（因架構而異）

#### 3.4.3 HBM Bandwidth

gfx942 (MI300) — [gfx942:0400_roofline.yaml:54](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L54)：

```
HBM Bandwidth = SUM(
    TCC_BUBBLE_sum * 128 +
    TCC_EA0_RDREQ_32B_sum * 32 +
    (TCC_EA0_RDREQ_sum - TCC_BUBBLE_sum - TCC_EA0_RDREQ_32B_sum) * 64 +
    (TCC_EA0_WRREQ_sum - TCC_EA0_WRREQ_64B_sum) * 32 +
    TCC_EA0_WRREQ_64B_sum * 64
) / SUM(End_Timestamp - Start_Timestamp)
```

說明：
- Bubble 讀取：128 bytes/request
- 32B 讀取：32 bytes/request
- 剩餘讀取（64B cache line）：64 bytes/request
- 32B 寫入：32 bytes/request
- 64B 寫入：64 bytes/request
- gfx90a (MI200) 使用不同的 64B cache line size：[gfx90a:0400_roofline.yaml:50](src/rocprof_compute_soc/analysis_configs/gfx90a/0400_roofline.yaml#L50)

#### 3.4.4 L2 Cache Bandwidth

gfx942 — [gfx942:0400_roofline.yaml:58](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L58)：

```
L2 Cache BW = 128 * SUM(
    TCP_TCC_WRITE_REQ_sum +
    TCP_TCC_ATOMIC_WITH_RET_REQ_sum +
    TCP_TCC_ATOMIC_WITHOUT_RET_REQ_sum +
    TCP_TCC_READ_REQ_sum
) / SUM(End_Timestamp - Start_Timestamp)
```

說明：
- MI300 (gfx942) L2 cache line size = 128 bytes
- MI200 (gfx90a) L2 cache line size = 64 bytes

#### 3.4.5 L1 Cache Bandwidth

gfx942 — [gfx942:0400_roofline.yaml:62](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L62)：

```
L1 Cache BW = 128 * SUM(TCP_TOTAL_CACHE_ACCESSES_sum)
             / SUM(End_Timestamp - Start_Timestamp)
```

#### 3.4.6 LDS Bandwidth

gfx942 — [gfx942:0400_roofline.yaml:66](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L66)：

```
LDS BW = SUM(
    (SQ_LDS_IDX_ACTIVE - SQ_LDS_BANK_CONFLICT) * 4 * $lds_banks_per_cu
) / SUM(End_Timestamp - Start_Timestamp)
```

說明：
- 每個 CU 有 $lds_banks_per_cu 個 LDS bank（gfx942: 32 banks）
- 每個 bank 每個 cycle 可以傳輸 4 bytes
- `SQ_LDS_IDX_ACTIVE - SQ_LDS_BANK_CONFLICT` 給出實際有效傳輸的 cycle 數

#### 3.4.7 Arithmetic Intensity (AI) at each Memory Level

總 FLOPs = VALU FLOPs (all datatypes) + MFMA FLOPs (all datatypes)

**AI HBM** = Total FLOPs / HBM Bytes

([gfx942:0400_roofline.yaml:80-81](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L80-L81))：

```
AI HBM = SUM(TotalVALUFLOPs + TotalMFMAFLOPs)
       / SUM(HBM Bytes expression)

其中 TotalVALUFLOPs = $wave_size * (
    SQ_INSTS_VALU_ADD_F16 + SQ_INSTS_VALU_MUL_F16 + 2*SQ_INSTS_VALU_FMA_F16 + SQ_INSTS_VALU_TRANS_F16 +
    SQ_INSTS_VALU_ADD_F32 + SQ_INSTS_VALU_MUL_F32 + 2*SQ_INSTS_VALU_FMA_F32 + SQ_INSTS_VALU_TRANS_F32 +
    SQ_INSTS_VALU_ADD_F64 + SQ_INSTS_VALU_MUL_F64 + 2*SQ_INSTS_VALU_FMA_F64 + SQ_INSTS_VALU_TRANS_F64
)

其中 TotalMFMAFLOPs =
    SQ_INSTS_VALU_MFMA_MOPS_F16 * 512 +
    SQ_INSTS_VALU_MFMA_MOPS_BF16 * 512 +
    SQ_INSTS_VALU_MFMA_MOPS_F32 * 512 +
    SQ_INSTS_VALU_MFMA_MOPS_F64 * 512 +
    SQ_INSTS_VALU_MFMA_MOPS_F8  * 512
```

類似地：
- **AI L2** = Total FLOPs / L2 Bytes（[行 83-84](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L83-L84)）
- **AI L1** = Total FLOPs / L1 Bytes（[行 86-87](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L86-L87)）
- **AI LDS** = Total FLOPs / LDS Bytes（[行 89-90](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L89-L90)）
- **AI L0** (僅 gfx950/MI350) = Total FLOPs / L0 Bytes

#### 3.4.8 Performance (GFLOP/s)

[gfx942:0400_roofline.yaml:92-93](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml#L92-L93)：

```
Performance (GFLOPs) = Total FLOPs
    / (SUM(End_Timestamp - Start_Timestamp) / 1e9)  ← 轉換為秒
    / 1e9                                              ← 轉換為 Giga
```

#### 3.4.9 Kernel Bound Status 判斷

[`roofline_main.py:146-184`](src/roofline/roofline_main.py#L146-L184) 中的 `_determine_kernel_bound_status()`：

```python
x_intersect = min_peak / bandwidth   # 計算屋頂交點

if ai_value < x_intersect:
    return "Memory Bound"
else:
    return "Compute Bound"
```

其中 `min_peak = min(Peak_VALU_FLOPs, Peak_Matrix_FLOPs)`。

#### 3.4.10 經驗峰值 (Empirical Peaks) — 屋頂線

Peak values 來自 Tier B 的 microbenchmark（`benchmark_base.py`），而非理論峰值。Benchmark 產出寫入 `roofline.csv`。

VALU peak benchmark（[`benchmark_base.py:605-667`](src/roofline/benchmark/benchmark_base.py#L605-L667)）：

```
total_flops = threads * iterations * VALU_NFMA * 2
```

其中：
- `VALU_NFMA = 1024` (行 53) — 每個 thread 每次 iteration 的 FMA 操作數
- 乘以 2 因為每個 FMA = 2 FLOPs
- 時間由 `hipEventElapsedTime` 量測（行 294）

Matrix ops benchmark（[`benchmark_base.py:670-738`](src/roofline/benchmark/benchmark_base.py#L670-L738)）：

```
total_flops = workgroups * workgroup_size / WAVEFRONT_SIZE * iters * matrix_ops[type]
```

Cache bandwidth benchmark（[`benchmark_base.py:475-533`](src/roofline/benchmark/benchmark_base.py#L475-L533)）：

```
total_bytes = workgroups * iters * cache_size
```

HBM bandwidth benchmark（[`benchmark_base.py:419-472`](src/roofline/benchmark/benchmark_base.py#L419-L472)）：

```
total_bytes = dataset_entries * sizeof(double) * 2  # read + write
```

### 3.5 關鍵原始碼位置總表

| 功能 | 檔案 | 行號 | 語言 |
|------|------|------|------|
| **Roofline 繪圖主類別** | [`src/roofline/roofline_main.py`](src/roofline/roofline_main.py) | 91-1268 | Python |
| AI 計算入口 | [`src/utils/roofline_calc.py`](src/utils/roofline_calc.py) | 417-561 (`calc_ai_analyze`) | Python |
| 屋頂線計算 | [`src/utils/roofline_calc.py`](src/utils/roofline_calc.py) | 258-410 (`calc_ceilings`) | Python |
| 屋頂建構 | [`src/utils/roofline_calc.py`](src/utils/roofline_calc.py) | 564-646 (`construct_roof`) | Python |
| Bound 判斷 | [`src/roofline/roofline_main.py`](src/roofline/roofline_main.py) | 146-184 | Python |
| 表達式求值引擎 | [`src/utils/metrics/metric_evaluator.py`](src/utils/metrics/metric_evaluator.py) | 44-104 | Python |
| Pipeline 入口 | [`src/utils/metrics/evaluation_pipeline.py`](src/utils/metrics/evaluation_pipeline.py) | 180-276 (`eval_metric`) | Python |
| Counter 定義解析 | [`src/utils/utils_counter_defs.py`](src/utils/utils_counter_defs.py) | 1-164 | Python |
| **gfx942 Roofline 公式** | [`src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml`](src/rocprof_compute_soc/analysis_configs/gfx942/0400_roofline.yaml) | 1-184 | YAML |
| **gfx90a Roofline 公式** | [`src/rocprof_compute_soc/analysis_configs/gfx90a/0400_roofline.yaml`](src/rocprof_compute_soc/analysis_configs/gfx90a/0400_roofline.yaml) | 1-100+ | YAML |
| **gfx950 Roofline 公式** | [`src/rocprof_compute_soc/analysis_configs/gfx950/0400_roofline.yaml`](src/rocprof_compute_soc/analysis_configs/gfx950/0400_roofline.yaml) | 1-X | YAML |
| **gfx115x Roofline 公式** | [`src/rocprof_compute_soc/analysis_configs/gfx115x/0400_roofline.yaml`](src/rocprof_compute_soc/analysis_configs/gfx115x/0400_roofline.yaml) | 1-X | YAML |
| Peak FLOPs Benchmark | [`src/roofline/benchmark/benchmark_base.py`](src/roofline/benchmark/benchmark_base.py) | 605-667 (`flops_bench`) | Python |
| Peak Matrix Benchmark | [`src/roofline/benchmark/benchmark_base.py`](src/roofline/benchmark/benchmark_base.py) | 670-738 (`matrix_bench`) | Python |
| Peak HBM BW Benchmark | [`src/roofline/benchmark/benchmark_base.py`](src/roofline/benchmark/benchmark_base.py) | 419-472 | Python |
| Peak Cache BW Benchmark | [`src/roofline/benchmark/benchmark_base.py`](src/roofline/benchmark/benchmark_base.py) | 475-533 | Python |
| Peak LDS BW Benchmark | [`src/roofline/benchmark/benchmark_base.py`](src/roofline/benchmark/benchmark_base.py) | 552-602 | Python |
| Benchmark 入口 | [`src/roofline/run_benchmark.py`](src/roofline/run_benchmark.py) | — | Python |
| Metric 描述 (gfx942) | [`docs/data/metrics/gfx942_metrics.yaml`](docs/data/metrics/gfx942_metrics.yaml) | 1-200+ | YAML |

### 3.6 以 MI300 (gfx942) 為例的完整計算流程

**Step 1 — Benchmark 階段** (Tier B)：

```
run_roofline_benchmark() → Bench_gfx942.run_benchmark()
  ├── hbm_bw_benchmark()    → roofline.csv: HBMBw = ~3.35 TB/s
  ├── l2_bw_bench()         → roofline.csv: L2Bw  = ~5.0 TB/s
  ├── l1_bw_bench()         → roofline.csv: L1Bw  = ~10+ TB/s
  ├── lds_bw_benchmark()    → roofline.csv: LDSBw = ~16+ TB/s
  ├── fp32_benchmark()      → roofline.csv: FP32Flops = ~43.5 TFLOP/s (VALU)
  ├── fp16_benchmark()      → roofline.csv: FP16Flops = ~87 TFLOP/s (VALU packed)
  ├── matrix_f16_bench()    → roofline.csv: MFMAF16Flops = ~326 TFLOP/s (MFMA)
  └── ...其他 datatype...
```

**Step 2 — Profile 階段** (Tier C)：

```
rocprofv3 → SDK counter collection
  ├── SQ block counters: instruction counts
  ├── TCP block counters: L1/L2 traffic
  ├── TCC block counters: HBM traffic
  ├── GRBM: active cycles
  └── dispatch timestamps: start/end per kernel
```

**Step 3 — Analyze 階段** (Tier A)：

```
eval_metric() → MetricEvaluator.eval_expression()
  ├── For each kernel:
  │   ├── VALU FLOPs = formula from 0400_roofline.yaml table 401
  │   ├── HBM BW     = formula from 0400_roofline.yaml table 401
  │   ├── L2 BW      = formula from 0400_roofline.yaml table 401
  │   ├── AI HBM     = Total FLOPs / HBM Bytes   (table 402)
  │   ├── AI L2      = Total FLOPs / L2 Bytes    (table 402)
  │   ├── AI L1      = Total FLOPs / L1 Bytes    (table 402)
  │   └── Performance = Total FLOPs / Time       (table 402)
  │
  └── calc_ceilings() → 計算屋頂線:
      ├── HBM roof:   line from (XMIN, XMIN*Peak_HBM_BW) to (Peak_FLOPs/Peak_HBM_BW, Peak_FLOPs)
      ├── L2 roof:    line from (XMIN, XMIN*Peak_L2_BW) to (Peak_FLOPs/Peak_L2_BW, Peak_FLOPs)
      ├── VALU roof:  horizontal at y = Peak_VALU_FLOPs
      └── MFMA roof:  horizontal at y = Peak_MFMA_FLOPs
```

**Step 4 — 繪圖** (Tier A)：

```
Roofline.generate_plot() / cli_generate_plot()
  → Plotly HTML / plotext terminal 圖：
    x-axis: Arithmetic Intensity (FLOPs/Byte) — log scale
    y-axis: Performance (GFLOP/s) — log scale
    - 對角線：頻寬屋頂 (斜率 = Peak Bandwidth)
    - 水平線：計算屋頂 (Peak VALU / Peak Matrix)
    - 散點：每個 kernel 在各個 cache level 的 (AI, Performance)
```

---

## 4. 附錄：Counter 分類與含義

### SQ (Sequencer) Block
- 管理 wavefront 排程和指令發出
- 提供所有 VALU/MFMA 指令計數器
- 提供 LDS 相關計數器 (LDS_IDX_ACTIVE, LDS_BANK_CONFLICT)

### TCP (Texture Cache Pipe) Block
- vL1D cache controller
- `TCP_TOTAL_CACHE_ACCESSES_sum` — vL1D 總訪問
- `TCP_TCC_*_REQ_sum` — 從 L1 到 L2 的請求

### TCC (Texture Cache Controller) Block
- L2 cache + HBM memory controller
- `TCC_EA0_*` — HBM 讀寫請求（EA = External Access）
- `TCC_BUBBLE_sum` — pipeline bubble（無效資料傳輸）

### GRBM (Graphics Register Bus Manager) Block
- `GRBM_GUI_ACTIVE` — GPU 活躍 cycle 計數
- 用於標準化和檢查 GPU 是否真的在執行

### SPI (Shader Processor Input) Block
- Workgroup 分派和資源分配
- 提供 workgroup 排程統計

### Cache Line Size 差異

| 架構 | GPU | L1/L2 Cache Line Size |
|------|-----|----------------------|
| gfx90a | MI200 系列 | 64 bytes |
| gfx942 | MI300 系列 | 128 bytes |
| gfx950 | MI350 系列 | 128 bytes |
| gfx115x | RDNA 3.5 | 128 bytes |

這直接影響 Bandwidth 計算公式中每個 request 對應的 bytes 數量。

---

> **參考**
> - ROCm Documentation: https://rocm.docs.amd.com
> - Roofline Model: Williams, S., Waterman, A., & Patterson, D. (2009). "Roofline: an insightful visual performance model for multicore architectures." *Communications of the ACM*, 52(4), 65-76.

---

## 5. gfx1250 (MI450) 架構專題分析 ⭐

> **更新日期**: 2026-08-11（基於最新程式碼）
> **狀態**: rocprofiler-compute **已完整支援** gfx1250！包含 SoC 定義、Roofline 公式、Benchmark、Counter Sets 與 Metric 說明。

### 5.1 硬體識別與架構定位

| 屬性 | 值 | 說明 |
|------|-----|------|
| **GPU** | MI450 | AMD Instinct AI 加速器 |
| **gfx IP** | `gc_12_5_0` | gfx12 世代 (CDNA5-class) |
| **athub IP** | `athub_4_2_0` | Address Translation Hub |
| **SoC Class** | `gfx1250_soc(OmniSoC_Base)` | [`soc_gfx1250.py:13`](src/rocprof_compute_soc/soc_gfx1250.py#L13) |
| **Benchmark Class** | `Bench_gfx1250(Bench_gfx12)` | [`benchmark_gfx1250.py:17`](src/roofline/benchmark/gfx12/benchmark_gfx1250.py#L17) |
| **Matrix Type** | **WMMA** (Wave Matrix Multiply-Accumulate) | 非 MFMA! |
| **Wavefront Size** | **32** | [`benchmark_gfx12_base.py:21`](src/roofline/benchmark/gfx12/benchmark_gfx12_base.py#L21) |
| **LDS Banks/CU** | 32 | [`soc_gfx1250.py:23`](src/rocprof_compute_soc/soc_gfx1250.py#L23) |
| **L2 Banks** | 24 | [`soc_gfx1250.py:22`](src/rocprof_compute_soc/soc_gfx1250.py#L22) |
| **Pipes/GPU** | 4 | [`soc_gfx1250.py:24`](src/rocprof_compute_soc/soc_gfx1250.py#L24) |
| **GFX12 Variant** | `0x1250` | [`gfx12_def.h:28`](source/lib/aqlprofile/def/gfx12_def.h#L28) |
| **Profiler 兼容** | `rocprofv3`, `rocprofiler-sdk` | [`soc_gfx1250.py:17`](src/rocprof_compute_soc/soc_gfx1250.py#L17) |

### 5.2 記憶體階層：GL0 → GL1 → GL2 → HBM

gfx1250 使用全新的命名體系與 data path：

```
WGP (CU)                 GL0 (L0) Cache               GL1 Cache (12ch/SE)
┌──────────┐    TX_VCA   ┌──────────────┐   TX_VMW    ┌─────────────────┐
│ VALU/SP  │◄──────────►│ Vector Cache │◄──────────►│ GL1A (Arbiter)  │
│ WMMA/SQ  │   Load/Store │ (per CU)     │  GL1 Reqs  │ GL1C (Cache)    │
│ LDS      │   Atomic    │ Hit/Miss     │  128B/line │                 │
└──────────┘             └──────────────┘             └────────┬────────┘
                                                               │
                              GL2 Cache (per AID)              │
                         ┌──────────────────────────┐          │
                         │ GL2A (Arbiter) ×24        │◄─────────┘
                         │ GL2C (Cache) ×96          │  256B/line
                         │ Hit/Miss                  │
                         └────────┬─────────────────┘
                                  │
                    HBM (via GC_EA_SE ×96) 32B/request
                         ┌────────┴─────────────────┐
                         │ GC_EA_SE_SARB_DRAM_RD/WR │
                         │ DRAM Read/Write Cmd Pop   │
                         └──────────────────────────┘
```

### 5.3 Roofline Counter 定義（實際公式）

以下所有公式來自 [`analysis_configs/gfx1250/0400_roofline.yaml`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml)。

#### 5.3.1 VALU FLOPs Counters — SP Block

gfx1250 的 VALU FLOPs 使用 **SP (Stream Processor)** block 的預聚合 counter，而非 gfx942 的 SQ block 逐指令計數。這是關鍵差異！

| Counter | 含義 | 公式中使用 |
|---------|------|-----------|
| `SP_VALU_FLOPS_FP16_sum` | 預先計算的 FP16 VALU FLOPs（不含 TRANS） | 直接用於吞吐量 |
| `SP_VALU_FLOPS_FP32_sum` | 預先計算的 FP32 VALU FLOPs（不含 TRANS） | 直接用於吞吐量 |
| `SP_VALU_FLOPS_FP64_sum` | 預先計算的 FP64 VALU FLOPs（不含 TRANS） | 直接用於吞吐量 |
| `SP_VALU_IOPS_sum` | 預先計算的 Integer VALU OPs | 直接用於吞吐量 |
| `SP_VALU_FLOPS_FP16_TRANS_sum` | FP16 transcendental FLOPs | 直接用於吞吐量 |
| `SP_VALU_FLOPS_FP32_TRANS_sum` | FP32 transcendental FLOPs | 直接用於吞吐量 |
| `SP_VALU_FLOPS_FP64_TRANS_sum` | FP64 transcendental FLOPs | 直接用於吞吐量 |

**VALU FLOPs (FP16)** — [`0400_roofline.yaml:30`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L30)：

```
VALU FLOPs (FP16) = AVG(
    SP_VALU_FLOPS_FP16_sum / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $max_sclk * $cu_per_gpu * 128 / 1000
```

**VALU FLOPs (FP32)** — [`0400_roofline.yaml:34`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L34)：

```
VALU FLOPs (FP32) = AVG(
    SP_VALU_FLOPS_FP32_sum / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $max_sclk * $cu_per_gpu * 64 / 1000   ← 64 FLOPs/CU/cycle
```

**VALU FLOPs (FP64)** — [`0400_roofline.yaml:38`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L38)：

```
VALU FLOPs (FP64) = AVG(
    SP_VALU_FLOPS_FP64_sum / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $max_sclk * $cu_per_gpu * 32 / 1000
```

> **關鍵差異 vs gfx942**：gfx1250 使用 `SP_VALU_FLOPS_*`（SP block 預聚合 counter），不再使用 `$wave_size * SUM(SQ_INSTS_VALU_ADD_* + SQ_INSTS_VALU_MUL_* + 2*SQ_INSTS_VALU_FMA_* + ...)`。這大幅簡化了公式，因為硬體已預先計算好 FLOPs。

#### 5.3.2 WMMA FLOPs Counters — SQ Block

gfx1250 使用 **WMMA (Wave Matrix Multiply-Accumulate)** 而非 MFMA。這是最重要的架構差異。

| Counter | WMMA 指令 | FLOPs/指令 | 來源 |
|---------|----------|-----------|------|
| `SQ_VALU_WMMA_FLOP_FP64_sum` | `V_WMMA_F64_16X16X4_F64` | **64** | benchmark 註解: 16×16×4×2=2048 ops? 但 multiplier=64 |
| `SQ_VALU_WMMA_FLOP_FP32_sum` | `V_WMMA_F32_16X16X4_F32` | **128** | `benchmark_gfx12_base.py:137`: 16×16×4×2=2048 |
| `SQ_VALU_WMMA_FLOP_FP16_sum` | `V_WMMA_F16_16X16X32_F16` | **2048** | `benchmark_gfx12_base.py:171`: 16×16×32×2=16384 ops |
| `SQ_VALU_WMMA_FLOP_BF16_sum` | `V_WMMA_BF16_16X16X32_BF16` | **2048** | 同上 |
| `SQ_VALU_WMMA_FLOP_FP8_sum` | `V_WMMA_F32_16X16X64_FP8_FP8` | **4096** | `benchmark_gfx12_base.py:274`: 16×16×64×2=32768 |
| `SQ_VALU_WMMA_FLOP_FP6_sum` | `V_WMMA_F32_16X16X128_F8F6F4` (FP6) | **8192** | benchmark: 16×16×128×2=65536 |
| `SQ_VALU_WMMA_FLOP_FP4_sum` | `V_WMMA_F32_16X16X128_F8F6F4` (FP4) | **8192** | benchmark: 16×16×128×2=65536 |
| `SQ_VALU_WMMA_FLOP_F6F4_sum` | `V_WMMA_F32_16X16X128_F8F6F4` (Mixed) | **8192** | benchmark: 16×16×128×2=65536 |
| `SQ_VALU_WMMA_FLOP_I8_sum` | `V_WMMA_I32_16X16X64_IU8` | **4096** | `benchmark_gfx12_base.py:240`: 16×16×64×2=32768 |

> **注意**: Roofline 公式中的 multiplier 與 benchmark kernel 註解中的 ops 數不完全一致。Roofline 使用的 multiplier 是 benchmark ops 的 1/8（因為 benchmark 使用 `total_flops / time` 而 Roofline counter 報告的是指令數量）。

**WMMA FLOPs (FP16)** — [`0400_roofline.yaml:66`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L66)：

```
WMMA FLOPs (FP16) = AVG(
    SQ_VALU_WMMA_FLOP_FP16_sum * 2048
    / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $WMMAF16Flops_empirical_peak  ← 來自 roofline.csv benchmark
```

**WMMA FLOPs (FP8)** — [`0400_roofline.yaml:74`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L74)：

```
WMMA FLOPs (FP8) = AVG(
    SQ_VALU_WMMA_FLOP_FP8_sum * 4096
    / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $WMMAF8Flops_empirical_peak
```

**WMMA IOPs (Int8)** — [`0400_roofline.yaml:90`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L90)：

```
WMMA IOPs (Int8) = AVG(
    SQ_VALU_WMMA_FLOP_I8_sum * 4096
    / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $WMMAI8Ops_empirical_peak
```

#### 5.3.3 HBM Bandwidth Counters — GC_EA_SE Block

gfx1250 使用 **GC_EA_SE** (External Access per Shader Engine) 的 SARB (Store Arbitration) sub-block 來計算 HBM BW。

| Counter | 含義 |
|---------|------|
| `GC_EA_SE_SARB_DRAM_RD_SIZE_REQ_sum` | DRAM 讀取請求（以 32B 為單位） |
| `GC_EA_SE_SARB_DRAM_WR_SIZE_REQ_sum` | DRAM 寫入請求（以 32B 為單位） |
| `GC_EA_SE_SARB_OUTSTANDING_DRAM_RD_sum` | Outstanding DRAM reads (latency 計算) |
| `GC_EA_SE_DRAM_RD_CMD_POP_sum` | DRAM read command pop |
| `GC_EA_SE_DRAM_WR_CMD_POP_sum` | DRAM write command pop |

**HBM Bandwidth** — [`0400_roofline.yaml:102`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L102)：

```
HBM BW = AVG(
    (GC_EA_SE_SARB_DRAM_RD_SIZE_REQ_sum +
     GC_EA_SE_SARB_DRAM_WR_SIZE_REQ_sum) * 32
    / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $HBMBw_empirical_peak
```

> **HBM BW 計算非常簡潔**: 每個 request 固定 32 bytes，無需像 gfx942 那樣區分 32B/64B/128B/bubble。

#### 5.3.4 GL2 Cache Bandwidth Counters — TX_VMW Block

使用 **TX_VMW** (Vector Memory Writeback) block 到 GL1 的請求：

| Counter | 含義 |
|---------|------|
| `TX_VMW_GL1_REQ_READ_sum` | GL2 讀取請求 |
| `TX_VMW_GL1_REQ_WRITE_sum` | GL2 寫入請求 |
| `TX_VMW_GL1_REQ_ATOMIC_WITH_RET_sum` | GL2 atomic（有返回） |
| `TX_VMW_GL1_REQ_ATOMIC_WITHOUT_RET_sum` | GL2 atomic（無返回） |

**GL2 Cache BW** — [`0400_roofline.yaml:106-108`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L106-L108)：

```
GL2 Cache BW = AVG(
    (TX_VMW_GL1_REQ_READ_sum +
     TX_VMW_GL1_REQ_WRITE_sum +
     TX_VMW_GL1_REQ_ATOMIC_WITH_RET_sum +
     TX_VMW_GL1_REQ_ATOMIC_WITHOUT_RET_sum) * 128  ← 128B cache line
    / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $L2Bw_empirical_peak
```

#### 5.3.5 GL0 Cache Bandwidth Counters — TX_VCA Block

使用 **TX_VCA** (Vector Cache Access) block：

| Counter | 含義 |
|---------|------|
| `TX_VCA_CACHE_LOAD_BANDWIDTH_BYTES_sum` | GL0 load bandwidth (bytes) |
| `TX_VCA_CACHE_STORE_BANDWIDTH_BYTES_sum` | GL0 store bandwidth (bytes) |
| `TX_VCA_CACHE_ATOMIC_BANDWIDTH_BYTES_sum` | GL0 atomic bandwidth (bytes) |

**GL0 Cache BW** — [`0400_roofline.yaml:110`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L110)：

```
GL0 Cache BW = AVG(
    (TX_VCA_CACHE_LOAD_BANDWIDTH_BYTES_sum +
     TX_VCA_CACHE_STORE_BANDWIDTH_BYTES_sum +
     TX_VCA_CACHE_ATOMIC_BANDWIDTH_BYTES_sum)
    / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $L0Bw_empirical_peak
```

> **GL0 使用預先計算的 byte counter**（與 gfx115x 的 `TCP_REQ * 64` 不同），無需乘以 cache line size。

#### 5.3.6 LDS Bandwidth Counters — TX_VMW Block

| Counter | 含義 |
|---------|------|
| `TX_VMW_LDS_INPUT_ACTIVE_sum` | LDS input active cycles |
| `TX_VMW_LDS_BANK_CONFLICT_sum` | LDS bank conflict cycles |
| `TX_VCA_LDS_LOAD_BANDWIDTH_BYTES_sum` | LDS load bandwidth (bytes) |
| `TX_VCA_LDS_STORE_BANDWIDTH_BYTES_sum` | LDS store bandwidth (bytes) |

**LDS BW** — [`0400_roofline.yaml:114`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L114)：

```
LDS BW = AVG(
    (TX_VMW_LDS_INPUT_ACTIVE_sum - TX_VMW_LDS_BANK_CONFLICT_sum)
    * 4 * $lds_banks_per_cu                  ← lds_banks_per_cu = 32
    / ((End_Timestamp - Start_Timestamp) / 1e9) / 1e9
)
Peak = $LDSBw_empirical_peak
```

#### 5.3.7 系統級 Counters

| Counter | 含義 | 來源 Panel |
|---------|------|-----------|
| `SQG_LEVEL_WGP_ACTIVE_sum` | Active WGP count | `0200_system_speed_of_light.yaml:113` |
| `SQ_INSTS_ALL_sum` | Total instructions | `0200_system_speed_of_light.yaml:119` |
| `SQ_INSTS_INTERNAL_sum` | Internal instructions | IPC 計算 |
| `SQ_BUSY_CYCLES_sum` | SQ busy cycles | IPC 計算 |
| `SQG_WAVE_CYCLES_sum` | Wave cycles | Wavefront Occupancy |
| `GRBM_GUI_ACTIVE_PER_XCD` | GPU active cycles per XCD | 全域變數 |
| `TX_VMW_REQ_sum` | Total GL0 requests | GL0 Hit Rate |
| `TX_VMW_REQ_MISS_sum` | GL0 request misses | GL0 Hit Rate |
| `GL2C_HIT_sum` / `GL2C_MISS_sum` | L2 cache hit/miss | GL2 Hit Rate |
| `GL2C_REQ_sum` | Total L2 requests (×256B) | GL2 Cache BW |
| `SQC_DCACHE_HITS_sum` / `_MISSES_sum` | Scalar data cache | `0200` Panel |
| `SQC_ICACHE_HITS_sum` / `_MISSES_sum` | Instruction cache | `0200` Panel |

### 5.4 Arithmetic Intensity (AI) 公式

以下所有公式來自 [`0400_roofline.yaml:130-140`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L130-L140)。

**總 FLOPs 的分子**在所有 AI 公式中相同：

```
TotalFLOPs = SP_VALU_FLOPS_FP16_sum + SP_VALU_FLOPS_FP32_sum
           + SP_VALU_FLOPS_FP64_sum
           + SP_VALU_FLOPS_FP16_TRANS_sum + SP_VALU_FLOPS_FP32_TRANS_sum
           + SP_VALU_FLOPS_FP64_TRANS_sum
           + SQ_VALU_WMMA_FLOP_FP16_sum * 2048
           + SQ_VALU_WMMA_FLOP_BF16_sum * 2048
           + SQ_VALU_WMMA_FLOP_FP32_sum * 128
           + SQ_VALU_WMMA_FLOP_FP64_sum * 64
           + SQ_VALU_WMMA_FLOP_FP8_sum  * 4096
           + SQ_VALU_WMMA_FLOP_FP6_sum  * 8192
           + SQ_VALU_WMMA_FLOP_FP4_sum  * 8192
           + SQ_VALU_WMMA_FLOP_F6F4_sum * 8192
```

**AI HBM** — [`0400_roofline.yaml:131`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L131)：

```
AI HBM = SUM(TotalFLOPs)
       / (32 * SUM(GC_EA_SE_SARB_DRAM_RD_SIZE_REQ_sum
                 + GC_EA_SE_SARB_DRAM_WR_SIZE_REQ_sum))
```

**AI GL2** — [`0400_roofline.yaml:134`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L134)：

```
AI GL2 = SUM(TotalFLOPs)
       / (128 * SUM(TX_VMW_GL1_REQ_READ_sum + TX_VMW_GL1_REQ_WRITE_sum
                  + TX_VMW_GL1_REQ_ATOMIC_WITH_RET_sum
                  + TX_VMW_GL1_REQ_ATOMIC_WITHOUT_RET_sum))
```

**AI GL0** — [`0400_roofline.yaml:137`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L137)：

```
AI GL0 = SUM(TotalFLOPs)
       / SUM(TX_VCA_CACHE_LOAD_BANDWIDTH_BYTES_sum
           + TX_VCA_CACHE_STORE_BANDWIDTH_BYTES_sum
           + TX_VCA_CACHE_ATOMIC_BANDWIDTH_BYTES_sum)
```

> **注意**: gfx1250 Roofline 目前**無 AI LDS** 數據點（與 gfx942 不同），僅有 AI HBM、AI GL2、AI GL0 三個 memory level。

**Performance (GFLOPs)** — [`0400_roofline.yaml:140`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml#L140)：

```
Performance (GFLOPs) = SUM(TotalFLOPs)
                     / (SUM(End_Timestamp - Start_Timestamp) / 1e9) / 1e9
```

### 5.5 Peak 計算方式

gfx1250 的 Peak 使用**兩種來源混合**：

#### 5.5.1 理論峰值（VALU）

VALU peak 使用 clock × CU 數 × per-CU throughput：

| Metric | Peak 公式 | per-CU Rate |
|--------|----------|------------|
| VALU FP16 | `$max_sclk * $cu_per_gpu * 128 / 1000` | 128 FLOPs/CU/cycle |
| VALU FP32 | `$max_sclk * $cu_per_gpu * 64 / 1000` | 64 FLOPs/CU/cycle |
| VALU FP64 | `$max_sclk * $cu_per_gpu * 32 / 1000` | 32 FLOPs/CU/cycle |
| VALU IOPs | `$max_sclk * $cu_per_gpu * 64 / 1000` | 64 IOPs/CU/cycle |
| VALU FP16 Trans | `$max_sclk * $cu_per_gpu * 64 / 1000` | 64 FLOPs/CU/cycle |
| VALU FP32 Trans | `$max_sclk * $cu_per_gpu * 32 / 1000` | 32 FLOPs/CU/cycle |
| VALU FP64 Trans | `$max_sclk * $cu_per_gpu * 16 / 1000` | 16 FLOPs/CU/cycle |

> **注意**: gfx1250 的 VALU *不使用* `$wave_size` 在公式中，因為 `SP_VALU_FLOPS_*` counter 已經報告了最終 FLOPs 數。

#### 5.5.2 經驗峰值（WMMA + Memory BW）

WMMA peak 和所有 BW peak 使用 **`roofline.csv` 實測值**（empirical peaks）：

```
$WMMAF16Flops_empirical_peak   ← benchmark_gfx1250.py 產出
$WMMABF16Flops_empirical_peak
$WMMAF8Flops_empirical_peak
$WMMAF6Flops_empirical_peak
$WMMAF4Flops_empirical_peak
$WMMAF6F4Flops_empirical_peak
$WMMAF32Flops_empirical_peak
$WMMAF64Flops_empirical_peak
$WMMAI8Ops_empirical_peak
$HBMBw_empirical_peak
$L2Bw_empirical_peak
$L0Bw_empirical_peak
$LDSBw_empirical_peak
```

### 5.6 Benchmark 參數

來自 [`benchmark_gfx1250.py`](src/roofline/benchmark/gfx12/benchmark_gfx1250.py) 和 [`benchmark_gfx12_base.py`](src/roofline/benchmark/gfx12/benchmark_gfx12_base.py)：

#### WMMA Matrix Ops per Instruction

| Datatype | Matrix Ops/Instr | Built-in | 說明 |
|----------|-----------------|----------|------|
| **F4** | **65536** | `__builtin_amdgcn_wmma_f32_16x16x128_f8f6f4(FP4)` | 16×16×128×2 |
| **F6** | **65536** | `__builtin_amdgcn_wmma_f32_16x16x128_f8f6f4(FP6)` | 同上 |
| **F6F4** | **65536** | `__builtin_amdgcn_wmma_f32_16x16x128_f8f6f4(Mixed)` | 混合精度 |
| **F8** | **32768** | `__builtin_amdgcn_wmma_f32_16x16x64_fp8_fp8` | 16×16×64×2 |
| **F16** | **16384** | `__builtin_amdgcn_wmma_f16_16x16x32_f16` | 16×16×32×2 |
| **BF16** | **16384** | `__builtin_amdgcn_wmma_bf16_16x16x32_bf16` | 同上 |
| **F32** | **2048** | `__builtin_amdgcn_wmma_f32_16x16x4_f32` | 16×16×4×2 |
| **F64** | **0 (unused)** | — | gfx1250 不支援 FP64 WMMA |
| **I8** | **32768** | `__builtin_amdgcn_wmma_i32_16x16x64_iu8` | 16×16×64×2 |

#### Benchmark 產出的 CSV Columns

來自 [`benchmark_gfx12_base.py:64-86`](src/roofline/benchmark/gfx12/benchmark_gfx12_base.py#L64-L86)：

| CSV Column | Benchmark |
|-----------|----------|
| `HBMBw` | `hbm_bw_benchmark()` |
| `L2Bw` | `l2_bw_bench()` |
| `L1Bw` | `l1_bw_bench()` |
| `L0Bw` | `l0_bw_bench()` |
| `LDSBw` | `lds_bw_benchmark()` |
| `FP16Flops` | `fp16_benchmark()` |
| `FP32Flops` | `fp32_benchmark()` |
| `FP64Flops` | `fp64_benchmark()` |
| `I8Ops` / `I32Ops` / `I64Ops` | `int8_benchmark()` / `int32` / `int64` |
| `WMMAF4`/`F6`/`F6F4`/`F8`/`F16`/`BF16`/`F32`/`F64`/`I8` | `matrix_*_bench()` |

> **L1 benchmark 被標記為 unsupported**（[`benchmark_gfx1250.py:21`](src/roofline/benchmark/gfx12/benchmark_gfx1250.py#L21): `self.unsupported_data_types = ["L1", "MALL", "WMMA-F64"]`）。

### 5.7 gfx1250 vs gfx942 完整對比

| 面向 | gfx942 (MI300) | gfx1250 (MI450) |
|------|---------------|-----------------|
| **Matrix 指令集** | MFMA | **WMMA** |
| **VALU Counter 來源** | SQ block 逐指令 (`SQ_INSTS_VALU_ADD_*` etc.) | **SP block 預聚合** (`SP_VALU_FLOPS_*`) |
| **HBM Counter** | `TCC_EA0_RDREQ/WRREQ_*` (分類 32B/64B) | **`GC_EA_SE_SARB_DRAM_RD/WR_SIZE_REQ`** (統一 32B) |
| **L2 Counter** | `TCP_TCC_*_REQ` × 128B | **`TX_VMW_GL1_REQ_*`** × 128B |
| **L1 Counter** | `TCP_TOTAL_CACHE_ACCESSES` × 128B (gfx942) | **N/A** (L1 benchmark unsupported) |
| **L0/GL0 Counter** | N/A | **`TX_VCA_CACHE_*_BANDWIDTH_BYTES`** (byte counter) |
| **Cache Line Size** | L1/L2: 128B, HBM: 32-128B | **GL2: 128B, GL2C: 256B, HBM: 32B** |
| **Wavefront Size** | 64 | **32** |
| **LDS Banks/CU** | 32 or 64 | **32** |
| **Peak VALU FP32** | Empirical (`$FP32Flops_empirical_peak`) | **理論公式** (`$max_sclk * $cu_per_gpu * 64 / 1000`) |
| **Peak WMMA** | Empirical | **Empirical** |
| **Roofline Memory Levels** | AI HBM, AI L2, AI L1, AI LDS | **AI HBM, AI GL2, AI GL0** (無 AI L1, 無 AI LDS) |
| **Roofline 公式複雜度** | 高（逐指令 SUM + wave_size） | **低**（預聚合 counter 直接使用） |

### 5.8 gfx1250 完整原始碼位置總表

| 功能 | 檔案路徑 | 關鍵行號 |
|------|---------|---------|
| **SoC Class** | [`src/rocprof_compute_soc/soc_gfx1250.py`](src/rocprof_compute_soc/soc_gfx1250.py) | 13-40 |
| **Roofline 公式** | [`src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml`](src/rocprof_compute_soc/analysis_configs/gfx1250/0400_roofline.yaml) | 1-217 |
| **Speed-of-Light** | [`src/rocprof_compute_soc/analysis_configs/gfx1250/0200_system_speed_of_light.yaml`](src/rocprof_compute_soc/analysis_configs/gfx1250/0200_system_speed_of_light.yaml) | 1-330+ |
| **GL2 Cache 分析** | [`src/rocprof_compute_soc/analysis_configs/gfx1250/1300_gl2_cache.yaml`](src/rocprof_compute_soc/analysis_configs/gfx1250/1300_gl2_cache.yaml) | 1-X |
| **Benchmark Class** | [`src/roofline/benchmark/gfx12/benchmark_gfx1250.py`](src/roofline/benchmark/gfx12/benchmark_gfx1250.py) | 17-46 |
| **Benchmark Base (gfx12)** | [`src/roofline/benchmark/gfx12/benchmark_gfx12_base.py`](src/roofline/benchmark/gfx12/benchmark_gfx12_base.py) | 17-403 |
| **Counter Sets** | [`src/rocprof_compute_soc/profile_configs/sets/gfx1250_sets.yaml`](src/rocprof_compute_soc/profile_configs/sets/gfx1250_sets.yaml) | 1-34 |
| **Metric Descriptions** | [`tools/per_arch_metric_definitions/gfx1250_metrics_description.yaml`](tools/per_arch_metric_definitions/gfx1250_metrics_description.yaml) | 1-150+ |
| **Docs Metrics** | [`docs/data/metrics/gfx1250_metrics.yaml`](docs/data/metrics/gfx1250_metrics.yaml) | 1-X |
| **GPU Specs (YAML)** | [`src/utils/mi_gpu_spec.yaml`](src/utils/mi_gpu_spec.yaml) | (gfx1250 section) |
| **aqlprofile Factory** | [`source/lib/aqlprofile/core/gfx1250_factory.cpp`](source/lib/aqlprofile/core/gfx1250_factory.cpp) | 33-141 |
| **gfx12 Block Info** | [`source/lib/aqlprofile/gfxip/gfx12/gfx12_block_info.h`](source/lib/aqlprofile/gfxip/gfx12/gfx12_block_info.h) | 215-332 |
| **gfx12 Block Table** | [`source/lib/aqlprofile/gfxip/gfx12/gfx12_block_table.h`](source/lib/aqlprofile/gfxip/gfx12/gfx12_block_table.h) | 114-651 |
| **gfx12 Primitives** | [`source/lib/aqlprofile/gfxip/gfx12/gfx12_primitives.h`](source/lib/aqlprofile/gfxip/gfx12/gfx12_primitives.h) | 38-732 |
| **PC Sampling Parser** | [`source/lib/rocprofiler-sdk/pc_sampling/parser/gfx1250.hpp`](source/lib/rocprofiler-sdk/pc_sampling/parser/gfx1250.hpp) | 26-31 |
| **gfx12 Defines** | [`source/lib/aqlprofile/def/gfx12_def.h`](source/lib/aqlprofile/def/gfx12_def.h) | 26-73 |
| **HIP TDM Header** | [`clr/hipamd/include/hip/amd_detail/amd_gfx1250_TDM.h`](clr/hipamd/include/hip/amd_detail/amd_gfx1250_TDM.h) | 1-X |

### 5.9 Roofline 計算完整流程 (gfx1250)

```
Phase 1: Benchmark (產生 roofline.csv)
  run_roofline_benchmark()
    → Bench_gfx1250.run_benchmark(device_id)
      ├── hbm_bw_benchmark()     → HBMBw_empirical_peak
      ├── l2_bw_bench()          → L2Bw_empirical_peak
      ├── l0_bw_bench()          → L0Bw_empirical_peak
      ├── lds_bw_benchmark()     → LDSBw_empirical_peak
      ├── fp16_benchmark()       → FP16Flops_empirical_peak
      ├── fp32_benchmark()       → FP32Flops_empirical_peak
      ├── fp64_benchmark()       → FP64Flops_empirical_peak
      ├── int8_benchmark()       → I8Ops_empirical_peak
      ├── matrix_f16_bench()     → WMMAF16Flops_empirical_peak  ← WMMA, not MFMA!
      ├── matrix_f8_bench()      → WMMAF8Flops_empirical_peak
      └── ...其他 WMMA datatypes

Phase 2: Profile (rocprofv3 收集 PMC counters)
  使用 gfx1250 profile_configs 中定義的 counter blocks:
    SP, SQ, TX_VCA, TX_VMW, GC_EA_SE, GL2C, GL1A, SQG, SQ, etc.
  → 產出 pmc_perf.csv

Phase 3: Analyze (eval_metric → Roofline)
  載入 0400_roofline.yaml 定義:
    Table 401 (Roofline Performance Rates):
      ├── SP_VALU_FLOPS_* / time → VALU GFLOP/s (per datatype)
      ├── SQ_VALU_WMMA_FLOP_* × N / time → WMMA GFLOP/s (per datatype)
      ├── (GC_EA_SE_*_REQ) × 32 / time → HBM GB/s
      ├── (TX_VMW_GL1_REQ_*) × 128 / time → GL2 GB/s
      ├── TX_VCA_*_BANDWIDTH_BYTES / time → GL0 GB/s
      └── (TX_VMW_LDS_* - conflict) × 4 × 32 / time → LDS GB/s

    Table 402 (Roofline Plot Points):
      ├── AI HBM  = TotalFLOPs / HBM_Bytes
      ├── AI GL2  = TotalFLOPs / GL2_Bytes
      ├── AI GL0  = TotalFLOPs / GL0_Bytes
      └── Performance = TotalFLOPs / time

Phase 4: 繪圖 (Roofline.generate_plot)
  calc_ceilings() 從 roofline.csv 讀取峰值 → 畫 bandwidth ceilings + compute ceilings
  散點圖: x=AI, y=Performance, 每個 kernel 每個 memory level 一個點
