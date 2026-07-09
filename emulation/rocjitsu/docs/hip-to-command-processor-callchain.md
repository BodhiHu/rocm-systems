# HIP 程序到 CommandProcessor 的完整調用鏈

本文檔詳細追蹤從 HIP 應用程序運行到 `vm/amdgpu/command_processor.cpp` 的完整調用鏈，涵蓋 CLI 啟動、LD_PRELOAD 攔截、KMD 模擬、AQL 隊列註冊、門鈴監控、以及 kernel dispatch 的全過程。

---

## 目錄

1. [架構總覽](#架構總覽)
2. [雙層攔截架構](#雙層攔截架構)
3. [第一階段：CLI 啟動](#第一階段cli-啟動)
4. [第二階段：LD_PRELOAD Constructor 初始化](#第二階段ld_preload-constructor-初始化)
5. [第三階段：攔截 /dev/kfd 的 open → VM 創建](#第三階段攔截-devkfd-的-open--vm-創建)
6. [第四階段：攔截 KFD ioctl → SimulatedDriver 分發](#第四階段攔截-kfd-ioctl--simulateddriver-分發)
7. [第五階段：create_queue_ioctl → CommandProcessor::register_queue](#第五階段create_queue_ioctl--commandprocessorregister_queue)
8. [第六階段：門鈴監控循環](#第六階段門鈴監控循環)
9. [第七階段：AQL Packet 獲取與解析](#第七階段aql-packet-獲取與解析)
10. [第八階段：Workgroup 分發到 ComputeUnit](#第八階段workgroup-分發到-computeunit)
11. [第九階段：完成追蹤與中斷信號](#第九階段完成追蹤與中斷信號)
12. [完整調用鏈圖](#完整調用鏈圖)
13. [關鍵文件索引](#關鍵文件索引)
14. [數據結構附錄](#數據結構附錄)

---

## 架構總覽

rocjitsu 是一個全系統 GPU 模擬器。當運行 `rocjitsu --config amdgpu_cdna4_kmd.json -- python test.py` 時，未修改的 HIP/ROCm 二進制文件在模擬的 GPU 上運行。模擬器在**兩個層級**透明地攔截 ROCm 運行時與真實硬件/內核驅動之間的交互。

```
┌──────────────────────────────────────────────────────────────┐
│  HIP 應用程序 (Python + PyTorch + HIP runtime)                │
│  未修改的二進制文件                                           │
├──────────────────────────────────────────────────────────────┤
│  ROCR (HSA Runtime) + libhsakmt (KFD thunk library)          │
│    │                                                         │
│    ├── [Layer 2: HSA_TOOLS_LIB]   librocjitsu_hooks.so       │
│    │    攔截 code object 加載，做 DBT 二進制翻譯              │
│    │                                                         │
│    ├── open("/dev/kfd")                                       │
│    ├── ioctl(kfd_fd, AMDKFD_IOC_*, ...)                      │
│    └── mmap(kfd_fd, ...)                                     │
│         │                                                    │
│         └── [Layer 1: LD_PRELOAD]   librocjitsu_kmd.so       │
│              攔截 libc syscall，路由到模擬驅動                │
├──────────────────────────────────────────────────────────────┤
│  SimulatedDriver (kmd/linux/simulated_driver.cpp)             │
│    KFD ioctl 處理：隊列、內存、事件、門鈴                      │
├──────────────────────────────────────────────────────────────┤
│  VirtualMachine → SoC → XCD → CommandProcessor               │
│    AQL packet 解析、workgroup 分發、完成追蹤                  │
├──────────────────────────────────────────────────────────────┤
│  ComputeUnit → Wavefront → ISA Execution                     │
│    GPU 指令執行                                               │
└──────────────────────────────────────────────────────────────┘
```

---

## 雙層攔截架構

### Layer 1: LD_PRELOAD 系統調用攔截 (`librocjitsu_kmd.so`)

**攔截點**：Linux libc syscall 級別（`open`, `ioctl`, `mmap`, `close`, `fopen` 等）

**機制**：`LD_PRELOAD` 環境變量注入共享庫，庫的 `__attribute__((constructor))` 函數在 main() 之前執行

**攔截的關鍵路徑**：
- `open("/dev/kfd")` → 返回假的 memfd，觸發 VM 創建
- `open("/dev/dri/renderD*")` → 返回假的 DRM fd
- `ioctl(kfd_fd, ...)` → 路由到 `SimulatedDriver::ioctl()`
- `mmap(kfd_fd, ...)` → 路由到 `SimulatedDriver::mmap()`
- `fopen("/sys/class/drm/...")` → 重定向到模擬的 sysfs 目錄

**源文件**：[`lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp)

### Layer 2: HSA Tools API 攔截 (`librocjitsu_hooks.so`)

**攔截點**：HSA runtime (ROCR) 內部的 Core API table

**機制**：`HSA_TOOLS_LIB` 環境變量，由 ROCR 在 `hsa_init()` 期間加載。`OnLoad()` 回調替換 API table 中的函數指針

**攔截的關鍵函數**：
- `hsa_code_object_reader_create_from_memory()` → 記錄原始 ELF 字節
- `hsa_executable_load_agent_code_object()` → 調用 BinaryTranslator 做 DBT 翻譯

**源文件**：[`lib/rocjitsu/src/rocjitsu/hooks/rj_hsa_dbt_hooks.cpp`](lib/rocjitsu/src/rocjitsu/hooks/rj_hsa_dbt_hooks.cpp)

---

## 第一階段：CLI 啟動

**入口文件**：[`tools/rocjitsu/main.cpp`](tools/rocjitsu/main.cpp)

rocjitsu CLI 支持三種運行模式：

| 模式 | 命令 | 模擬引擎位置 |
|------|------|-------------|
| **Local** | `rocjitsu --config cfg.json -- ./app` | 與應用程序同進程（後台線程） |
| **Daemon** | `rocjitsu --daemon --config cfg.json -- ./app` | 獨立守護進程，通過 Unix socket RPC |
| **Attach** | `rocjitsu --attach --config cfg.json -- ./app` | 連接到已有守護進程 |

### Local 模式啟動序列（最常用）

```
rocjitsu CLI (main.cpp)
  │
  ├─ 1. 解析命令行參數 (--config, --daemon, --, <app>)
  │
  ├─ 2. find_interposer_lib()
  │     查找 librocjitsu_kmd.so 的路徑（相對於 rocjitsu 二進制文件）
  │
  ├─ 3. write_config_file(abs_config)
  │     將配置文件的絕對路徑寫入 $XDG_RUNTIME_DIR/rocjitsu/config_path
  │     這使得攔截器在 LD_PRELOAD 之後可以找到配置
  │
  ├─ 4. setenv("LD_PRELOAD", lib_path, 1)         ← main.cpp:429
  │     設置 LD_PRELOAD 為 librocjitsu_kmd.so
  │
  └─ 5. execvp(app_argv[0], app_argv)             ← main.cpp:430
        用 LD_PRELOAD 執行目標應用程序
        操作系統加載器在應用程序代碼之前加載 librocjitsu_kmd.so
```

### Daemon 模式啟動序列

```
rocjitsu CLI (main.cpp)
  │
  ├─ fork()
  │   ├─ 子進程 (daemon):
  │   │   run_daemon_server(abs_config)
  │   │     ├─ rj_vm_create(config, RJ_VM_MODE_DAEMON, &vm)
  │   │     ├─ rj_vm_run(vm, nullptr)             ← 啟動模擬引擎線程
  │   │     └─ bind/listen/accept on Unix socket
  │   │        循環: handle_client() 處理 RPC 請求
  │   │
  │   └─ 父進程 (launcher):
  │       等待 daemon socket 出現
  │       setenv("LD_PRELOAD", ...)
  │       execvp(app_argv[0], app_argv)
  │         └─ 應用程序中的攔截器通過 RemoteDriver RPC 連接到守護進程
```

---

## 第二階段：LD_PRELOAD Constructor 初始化

**文件**：[`lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp)

當 `librocjitsu_kmd.so` 被加載時（在 `main()` 之前），constructor 自動執行：

```cpp
// interposer.cpp:419
__attribute__((constructor)) static void init_interposer() {
    InterposerContext::init();
}
```

### InterposerContext::init() 初始化內容

1. **解析真實 libc 函數**：通過 `dlsym(RTLD_NEXT)` 獲取所有真實 syscall 包裝函數的指針
   - `real.open`, `real.openat`, `real.close`, `real.ioctl`
   - `real.mmap`, `real.munmap`, `real.mprotect`, `real.madvise`
   - `real.dup`, `real.dup2`, `real.dup3`, `real.fcntl`
   - `real.fopen`, `real.freopen`, `real.opendir`
   - `real.stat`, `real.lstat`, `real.access`
   - `real.fork`

2. **初始化內部狀態**：
   - `fd_mutex_` — 保護 fd 追蹤的互斥鎖
   - `init_mutex_` — 保護延遲初始化（VM 創建）的互斥鎖
   - `drm_fds_` — DRM render node fd 追蹤集合
   - `kfd_fd_` — 假的 KFD 文件描述符
   - `kfd_dup_fds_` — 被複製的 KFD fd 集合

### LibcPassthrough 類

```cpp
// interposer.cpp:87-138
class LibcPassthrough {
    // 保存所有真實 libc 函數指針
    // 提供 ready() 檢查，確保所有符號已解析
};
```

---

## 第三階段：攔截 /dev/kfd 的 open → VM 創建

### open() 攔截器

**位置**：[`interposer.cpp:425-474`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L425-L474)

當 ROCR runtime（通過 libhsakmt）調用 `open("/dev/kfd", ...)` 時：

```
open("/dev/kfd", flags, mode)
  │
  ├─ 檢查: 是否為 "/dev/dri/renderD*" 路徑？
  │   └─ 是 → 創建假的 DRM memfd，返回高 fd 號碼
  │
  ├─ 檢查: path == "/dev/kfd" ?
  │   └─ 是 →
  │       ├─ get_or_create_remote()
  │       │   檢查是否有運行中的守護進程
  │       │   如果有 → 連接到 daemon socket (RemoteDriver)
  │       │
  │       └─ get_or_create()                     ← interposer.cpp:314
  │           │   (只在首次 open 時執行)
  │           │
  │           ├─ 1. 讀取配置文件
  │           │     $XDG_RUNTIME_DIR/rocjitsu/config_path
  │           │     (由 CLI 在 execvp 之前寫入)
  │           │
  │           ├─ 2. rj_vm_create(cfg_buf, RJ_VM_MODE_LOCAL, &vm)
  │           │     │
  │           │     ├─ config::load_config(json_path, schema)
  │           │     │   解析 JSON + FlatBuffers schema
  │           │     │   構建 SoC 拓撲樹
  │           │     │
  │           │     └─ create_from_loaded(loaded, mode, &handle)
  │           │          │                                    ← rj_vm.cpp:34
  │           │          │
  │           │          ├─ new SimulationEngine(config)      ← rj_vm.cpp:52
  │           │          │
  │           │          ├─ new VirtualMachine(soc, daemon)   ← rj_vm.cpp:92
  │           │          │   └─ driver_ = new SimulatedDriver(*soc, daemon)
  │           │          │                                      ← virtual_machine.cpp:28
  │           │          │
  │           │          ├─ engine->topology().set_root(vm)   ← rj_vm.cpp:94
  │           │          ├─ loaded.wire_links(topology)       ← rj_vm.cpp:95
  │           │          ├─ soc->wire_backing(topology)       ← rj_vm.cpp:96
  │           │          ├─ engine->build()                   ← rj_vm.cpp:99
  │           │          │   分區拓撲，初始化所有組件
  │           │          │
  │           │          ├─ driver->setup_topology(...)       ← rj_vm.cpp:105
  │           │          │   生成模擬的 sysfs 拓撲目錄
  │           │          │   設置環境變量指向模擬路徑
  │           │          │
  │           │          └─ driver->open()                    ← rj_vm.cpp:107
  │           │              創建 KfdProcess
  │           │              初始化每個 GPU 的 CP 回調
  │           │
  │           ├─ 3. 設置 execution plugins                    ← interposer.cpp:340-377
  │           │    根據環境變量:
  │           │    - RJ_LOG=1       → KernelLoggingPlugin
  │           │    - RJ_RACE=1      → RaceDetectorPlugin
  │           │    - RJ_SINKS=...   → 輸出 sink 配置
  │           │
  │           └─ 4. 啟動引擎線程                              ← interposer.cpp:379
  │                std::thread([vm] {
  │                    rj_vm_run(vm, nullptr);
  │                }).detach()
  │
  └─ drv->open()
       └─ 返回假的 KFD memfd (InterposerContext::driver_fd())
```

### rj_vm_create() 詳解

**文件**：[`lib/rocjitsu/src/rocjitsu/vm/rj_vm.cpp`](lib/rocjitsu/src/rocjitsu/vm/rj_vm.cpp)

```cpp
// rj_vm.cpp:143
rj_status_t rj_vm_create(const char *json_path, rj_vm_mode_t mode, rj_vm_t **vm) {
    auto loaded = config::load_config(json_path, kEmbeddedSchema);
    return create_from_loaded(loaded, mode, vm);
}
```

`create_from_loaded()` 創建的核心對象：

```
rj_vm_t
  ├─ engine: unique_ptr<SimulationEngine>
  │     └─ topology
  │           └─ root: unique_ptr<VirtualMachine>
  │                 ├─ SoC (作為子組件)
  │                 │   ├─ XCD[0..N]
  │                 │   │   ├─ CommandProcessor ("cp")
  │                 │   │   │   ├─ 門鈴輪詢線程
  │                 │   │   │   ├─ AQL packet 處理器
  │                 │   │   │   └─ Dispatch 控制器
  │                 │   │   ├─ L2Cache
  │                 │   │   └─ ShaderEngine[0..M]
  │                 │   │       └─ ComputeUnit[0..K]
  │                 │   ├─ IOD
  │                 │   │   ├─ MemorySideCache
  │                 │   │   └─ HbmController
  │                 │   └─ GpuMemory (共享)
  │                 └─ driver_: unique_ptr<SimulatedDriver>
  │                       ├─ GpuDevice[] (gpu_id, SoC*, CP state)
  │                       ├─ KfdProcess map
  │                       ├─ EventState map
  │                       └─ Doorbell page management
  ├─ soc: SoC* (指向 VirtualMachine 內部的 SoC)
  ├─ vm: VirtualMachine* (指向 topology root)
  └─ loaded: config::LoadedConfig
```

### VirtualMachine 構造函數

**文件**：[`lib/rocjitsu/src/rocjitsu/vm/virtual_machine.cpp`](lib/rocjitsu/src/rocjitsu/vm/virtual_machine.cpp)

```cpp
// virtual_machine.cpp:23-29
VirtualMachine::VirtualMachine(std::unique_ptr<SoC> soc, bool daemon_mode)
    : simdojo::CompositeComponent(soc->name()), soc_(soc.get()) {
    set_weight(0);
    adopt_children(*soc);
    add_child(std::move(soc));
    driver_ = std::make_unique<SimulatedDriver>(*soc_, daemon_mode);
}
```

### SimulatedDriver::open() 初始化

**文件**：[`lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.cpp`](lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.cpp)

`open()` 方法執行的關鍵初始化：

1. **創建假的 KFD memfd**：`memfd_create("rocjitsu_kfd", MFD_CLOEXEC)`
2. **創建 KfdProcess**：分配 process_id，初始化頁表
3. **為每個 GPU 配置 CommandProcessor 回調**：
   - `set_interrupt_callback(cb)` — CP 生成的中斷路由回 KFD EventState
   - `set_scratch_backing_resolver(cb)` — 查找進程的 scratch VA 映射
   - `set_scratch_backing_allocator(cb)` — 為 scratch 訪問分配物理頁面

---

## 第四階段：攔截 KFD ioctl → SimulatedDriver 分發

### ioctl() 攔截器

**位置**：[`interposer.cpp:626-693`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L626-L693)

```
ioctl(fd, request, arg)
  │
  ├─ 是否為 DRM fd？
  │   ├─ DRM_IOCTL_VERSION → 返回模擬的版本信息
  │   └─ DRM_IOCTL_AMDGPU_INFO → 返回零初始化的信息
  │
  ├─ 是否為遠程守護進程 fd？
  │   └─ remote->ioctl(request, arg) → RPC to daemon
  │
  ├─ 是否為本地 KFD fd（或其複製）？
  │   └─ SimulatedDriver::ioctl(request, arg)
  │       └─ dispatch_ioctl(proc, request, arg)       ← simulated_driver.cpp:588
  │
  └─ 否則 → real.ioctl(fd, request, arg)  (passthrough)
```

### dispatch_ioctl() 命令分發表

**位置**：[`simulated_driver.cpp:588-690`](lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.cpp#L588-L690)

| IOCTL 命令 | 處理函數 | 功能 |
|-----------|---------|------|
| `AMDKFD_IOC_GET_VERSION` | `get_version_ioctl` | 返回 KFD 接口版本 |
| `AMDKFD_IOC_GET_CLOCK_COUNTERS` | `get_clock_counters_ioctl` | GPU 時鐘計數器 |
| `AMDKFD_IOC_GET_PROCESS_APERTURES_NEW` | `get_apertures_ioctl` | LDS/scratch 基地址 |
| `AMDKFD_IOC_ACQUIRE_VM` | `acquire_vm_ioctl` | 無操作（VM 始終已獲取） |
| `AMDKFD_IOC_ALLOC_MEMORY_OF_GPU` | `alloc_memory_ioctl` | 分配 GPU 內存 |
| `AMDKFD_IOC_FREE_MEMORY_OF_GPU` | `free_memory_ioctl` | 釋放 GPU 內存 |
| `AMDKFD_IOC_MAP_MEMORY_TO_GPU` | `map_memory_ioctl` | 映射內存到 GPU |
| `AMDKFD_IOC_UNMAP_MEMORY_FROM_GPU` | `unmap_memory_ioctl` | 取消 GPU 內存映射 |
| **`AMDKFD_IOC_CREATE_QUEUE`** | **`create_queue_ioctl`** | **★ 創建 AQL 隊列 → 註冊到 CP** |
| `AMDKFD_IOC_UPDATE_QUEUE` | `update_queue_ioctl` | 更新隊列 ring buffer |
| `AMDKFD_IOC_DESTROY_QUEUE` | `destroy_queue_ioctl` | 銷毀隊列 → 從 CP 取消註冊 |
| `AMDKFD_IOC_CREATE_EVENT` | `create_event_ioctl` | 創建 KFD 事件 |
| `AMDKFD_IOC_DESTROY_EVENT` | `destroy_event_ioctl` | 銷毀 KFD 事件 |
| `AMDKFD_IOC_SET_EVENT` | `set_event_ioctl` | 設置事件信號 |
| `AMDKFD_IOC_RESET_EVENT` | `reset_event_ioctl` | 重置事件信號 |
| `AMDKFD_IOC_WAIT_EVENTS` | `wait_events_ioctl` | 等待事件（最長 100ms 超時） |
| `AMDKFD_IOC_SET_XNACK_MODE` | `set_xnack_mode_ioctl` | 設置 XNACK 模式 |
| `AMDKFD_IOC_SET_MEMORY_POLICY` | `set_memory_policy_ioctl` | 設置內存策略 |
| `AMDKFD_IOC_AVAILABLE_MEMORY` | (內聯) | 返回可用 VRAM |
| `AMDKFD_IOC_RUNTIME_ENABLE` | `runtime_enable_ioctl` | 啟用運行時 |
| `AMDKFD_IOC_SVM` | `svm_ioctl` | 共享虛擬內存操作 |

---

## 第五階段：create_queue_ioctl → CommandProcessor::register_queue

這是**連接 KMD 模擬與 CommandProcessor 的關鍵橋樑**。

### create_queue_ioctl() 詳解

**位置**：[`simulated_driver.cpp:1208-1281`](lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.cpp#L1208-L1281)

```
create_queue_ioctl(proc, arg)
  │
  ├─ 1. 解析 kfd_ioctl_create_queue_args
  │     ├─ gpu_id           → 目標 GPU
  │     ├─ ring_base_address → ring buffer 的 GPU VA
  │     ├─ ring_size        → ring buffer 大小
  │     ├─ read_pointer_address  → read pointer 地址
  │     ├─ write_pointer_address → write pointer 地址
  │     └─ queue_type       → COMPUTE / SDMA / SDMA_XGMI
  │
  ├─ 2. (Local 模式) 自動映射 ring buffer 和 rptr/wptr 頁面
  │     map_to_gpu(proc, va, host_ptr, size, Mtype::UC)
  │
  ├─ 3. 分配 doorbell offset
  │     從 gpu_state.free_doorbell_offsets 或線性分配
  │
  ├─ 4. 構建 HwQueue 結構體
  │     struct HwQueue {
  │         uint32_t process_id;
  │         uint32_t queue_id;
  │         uint64_t ring_base_va;       // ring buffer GPU 虛擬地址
  │         uint32_t ring_size;
  │         uint64_t read_ptr_va;        // read pointer 地址
  │         uint64_t write_ptr_va;       // write pointer 地址
  │         uint32_t doorbell_offset;    // 門鈴頁面內的偏移
  │         void    *doorbell_base;      // 門鈴頁面的主機指針
  │         uint64_t doorbell_va;        // 門鈴的 GPU VA（內部隊列）
  │         uint64_t last_doorbell;      // 上次檢測到的門鈴值
  │         bool     host_accessible;    // true = KFD 隊列
  │         bool     is_sdma;            // SDMA 隊列標記
  │         uint64_t queue_desc_va;      // amd_queue_t 描述符地址
  │     };
  │
  ├─ 5. 初始化 SDMA 隊列的 read/write pointers 為 0
  │
  ├─ 6. target_cp = gpu->soc->assign_queue_cp()
  │     選擇目標 CommandProcessor
  │
  ├─ 7. ★ target_cp->register_queue(std::move(hw))
  │     │                                        ← command_processor.cpp:371
  │     │
  │     ├─ 將 HwQueue 加入 hw_queues_ 向量
  │     ├─ 創建對應的 HwQueueState（追蹤 entries、dispatch 進度）
  │     ├─ 如果是第一個 host-accessible 隊列：
  │     │   註冊為 primary component
  │     │   啟動 doorbell 輪詢線程:                    ← command_processor.cpp:398
  │     │     doorbell_thread_ = std::jthread(
  │     │         [this](stop_token stop) { doorbell_poll_loop(stop); }
  │     │     )
  │     └─ 返回
  │
  └─ 8. 設置返回值
       args->queue_id = queue_id
       args->doorbell_offset = KFD_MMAP_TYPE_DOORBELL | gpu_id | db_offset
```

### register_queue() 詳解

**位置**：[`command_processor.cpp:371-399`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L371-L399)

```cpp
void CommandProcessor::register_queue(HwQueue queue) {
    // 1. 記錄隊列註冊（調試用）
    // 2. 鎖定 hw_queue_mutex_
    // 3. 將 HwQueue 和對應的 HwQueueState 加入向量
    // 4. 如果是第一個 host-accessible 隊列 → 啟動門鈴輪詢線程
    if (start_poll && !doorbell_thread_.joinable()) {
        doorbell_thread_ = std::jthread([this](std::stop_token stop) {
            doorbell_poll_loop(stop);
        });
    }
}
```

**重要**：門鈴輪詢線程只在第一個 KFD（host-accessible）隊列註冊時啟動。內部測試隊列通過 `schedule_event_now()` 直接注入門鈴事件。

---

## 第六階段：門鈴監控循環

### 硬件門鈴機制背景

在真實 AMD GPU 上：
- 每個 HSA 隊列有一個 doorbell（門鈴），是 PCIe MMIO 寄存器
- 用戶空間通過 mmap 的門鈴頁面寫入 doorbell
- 每次寫入 doorbell 告訴 CP「有新的 AQL packet 可用」
- CP 固件輪詢門鈴值，比較上次的值來檢測變化

在 rocjitsu 中，門鈴頁面是 `memfd_create()` 創建的共享內存頁面。`SimulatedDriver` 管理 doorbell page 的 mmap，而 `CommandProcessor` 的輪詢線程通過 `std::atomic_ref` 讀取門鈴值。

### doorbell_poll_loop() 詳解

**位置**：[`command_processor.cpp:504-551`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L504-L551)

```
doorbell_poll_loop(stop_token)
  │
  └─ while (!stop.stop_requested()):
       │
       ├─ scan_doorbells()                          ← command_processor.cpp:474
       │   │
       │   ├─ 鎖定 hw_queue_mutex_
       │   ├─ 遍歷所有 hw_queues_
       │   │   ├─ host_accessible 隊列：
       │   │   │   使用 atomic_ref<uint64_t> 原子讀取 doorbell
       │   │   │   val = *(doorbell_base + doorbell_offset)
       │   │   │
       │   │   └─ 內部隊列：
       │   │       通過 GpuMemory 讀取 doorbell_va
       │   │
       │   ├─ 比較 val != q.last_doorbell
       │   │   └─ 不同 → 更新 last_doorbell，標記 found = true
       │   │
       │   └─ 返回 found
       │
       ├─ 如果 scan_doorbells() 返回 true：
       │   engine()->schedule_event_now(&doorbell_event_)
       │   │
       │   │   doorbell_event_ 被註冊為 TIMER_CALLBACK 事件
       │   │   handler: [this](Tick ts, Message*) { handle_doorbell(ts); }
       │   │   引擎在下一個事件循環中調用 handle_doorbell
       │   │
       │   └─ 繼續輪詢（不 sleep）
       │
       ├─ 如果 scan_doorbells() 返回 false：
       │   sleep_for(100us)                           ← 空閒時休眠 100 微秒
       │
       ├─ 每 100 次輪詢（約 10ms）：
       │   HQD idle 監控：
       │   對所有空隊列重新廣播 interrupt_cb_(process_id, 0)
       │   確保延遲創建的事件也能看到隊列空閒狀態
       │
       └─ 每 5000 次輪詢（約 500ms）：
           調試日誌輸出所有隊列的門鈴狀態
```

### scan_doorbells() 詳解

**位置**：[`command_processor.cpp:474-502`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L474-L502)

```cpp
bool CommandProcessor::scan_doorbells() {
    bool found = false;
    std::lock_guard<std::recursive_mutex> lock(hw_queue_mutex_);
    for (auto &q : hw_queues_) {
        uint64_t val;
        if (q.host_accessible) {
            if (!q.doorbell_base) continue;
            // ★ 直接從主機內存原子讀取門鈴值
            val = std::atomic_ref<uint64_t>(
                *reinterpret_cast<uint64_t*>(
                    static_cast<char*>(q.doorbell_base) + q.doorbell_offset))
                .load(std::memory_order_acquire);
        } else {
            if (q.doorbell_va == 0) continue;
            val = read_gpu_u64(q.doorbell_va, q.process_id);
        }
        if (val != q.last_doorbell) {
            q.last_doorbell = val;
            found = true;
        }
    }
    return found;
}
```

**關鍵**：`memory_order_acquire` 保證門鈴讀取與 AQL packet 寫入之間的同步——當門鈴值變化被檢測到時，ring buffer 中的 AQL packet 數據保證可見。

### handle_doorbell() → process_queues()

**位置**：[`command_processor.cpp:1142-1144`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L1142-L1144) 和 [`command_processor.cpp:712-741`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L712-L741)

```
handle_doorbell(timestamp)
  └─ handle_doorbell_sync(timestamp)
       │
       ├─ 1. fetch_packets()
       │    遍歷所有隊列，從 ring buffer 獲取新的 AQL packets
       │    （詳見第七階段）
       │
       └─ 2. process_queues()
            對所有非 SDMA 隊列：
              while (有未處理的 dispatch entries):
                ├─ barrier 檢查（barrier_satisfied）
                │   如果 entry 有 barrier bit 且前序 entries 未完成 → 跳過
                │
                ├─ 非 kernel entry (barrier/signal) → 立即標記完成
                │
                ├─ kernel entry → dispatch_workgroups(entry)
                │   │                                   ← command_processor.cpp:582
                │   │   （詳見第八階段）
                │   │
                │   ├─ 如果全部 workgroups 已分發 → 移動到下一個 entry
                │   └─ 如果 CU 反壓 (sent == 0) → break
                │
                └─ 移動到下一個 entry 或隊列
```

---

## 第七階段：AQL Packet 獲取與解析

### fetch_from_queue() 詳解

**位置**：[`command_processor.cpp:925-979`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L925-L979)

```
fetch_from_queue(queue, qs)
  │
  ├─ 檢查: memory_ 存在？doorbell_base 有效？
  │
  ├─ 1. 讀取 write_idx 和 read_idx
  │     write_idx = read_gpu_u64(queue.write_ptr_va, queue.process_id)
  │     read_idx  = read_gpu_u64(queue.read_ptr_va, queue.process_id)
  │
  ├─ 2. SDMA 隊列特殊處理
  │     - 使用 byte-granularity pointers
  │     - doorbell 值可能大於 write pointer（批量提交）
  │     - 調用 process_sdma_ring() 處理 SDMA packets
  │
  ├─ 3. AQL doorbell 鉗制（僅計算隊列）
  │     process_limit = min(write_idx, doorbell + 1)
  │     確保只處理 doorbell 已通知的 packets
  │
  ├─ 4. 計算 packet 數量: delta = process_limit - read_idx
  │
  └─ 5. 循環讀取每個 AQL packet (64 bytes each)
        │
        ├─ read_gpu_block(pkt_va, &pkt, 64, vmid)
        │
        ├─ 檢查 packet header:
        │   ├─ HSA_PACKET_TYPE_KERNEL_DISPATCH
        │   │   └─ process_aql_packet(pkt, queue, pkt_addr, qs)
        │   │
        │   ├─ HSA_PACKET_TYPE_BARRIER_AND / BARRIER_OR
        │   │   └─ 設置 queue 的 implicit_barrier_next
        │   │      前序 entries 必須全部完成後才能處理新 entries
        │   │
        │   └─ HSA_PACKET_TYPE_VENDOR_SPECIFIC
        │       └─ 檢查是否為 extended kernel dispatch (format == 3)
        │
        └─ 更新 read_idx
```

### process_aql_packet() 詳解

**位置**：[`command_processor.cpp:870-922`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L870-L922)

```
process_aql_packet(pkt, queue, pkt_addr, qs)
  │
  ├─ 1. read_kernel_descriptor(pkt.kernel_object, vmid)
  │     │                                              ← command_processor.cpp:752
  │     │
  │     ├─ 通過 GpuMemory 從 kernel_object 地址讀取
  │     │   amdhsa::kernel_descriptor_t (64 bytes)
  │     │
  │     └─ kernel descriptor 包含：
  │         ├─ kernel_code_entry_byte_offset  → entry PC
  │         ├─ group_segment_fixed_size       → LDS 大小
  │         ├─ private_segment_fixed_size     → scratch 大小
  │         ├─ compute_pgm_rsrc1/2/3          → 寄存器配置
  │         ├─ kernel_code_properties         → 啟用的功能
  │         └─ workitem_*_register_count      → SGPR/VGPR 計數
  │
  ├─ 2. 計算 entry_pc = kernel_object + kernel_code_entry_byte_offset
  │
  ├─ 3. 決定 SGPR/VGPR 粒度
  │     sgpr_gran = descriptor encoded? compute_pgm_rsrc1 >> 6 : 0
  │
  ├─ 4. 計算每 workgroup 的 wavefront 數量
  │     wfs_per_wg = ceil(grid_size / wavefront_size)
  │
  ├─ 5. 計算總 workgroup 數量
  │     total_wgs = grid_size_x * grid_size_y * grid_size_z
  │
  ├─ 6. 創建 DispatchEntry
  │     struct DispatchEntry {
  │         uint32_t dispatch_id;
  │         uint64_t entry_pc;
  │         uint32_t sgprs_per_wf, vgprs_per_wf;
  │         uint32_t wfs_per_wg, total_wgs;
  │         uint32_t completed_wgs;         // 原子計數器
  │         uint64_t kernarg_addr;
  │         uint64_t completion_signal;
  │         amdhsa::kernel_descriptor_t kd;
  │         // ...
  │     };
  │
  └─ 7. qs.entries.push_back(dispatch_entry)
```

### AMD HSA Kernel Descriptor 結構

```cpp
// 來自 hsa/AMDHSAKernelDescriptor.h
struct kernel_descriptor_t {
    uint8_t  reserved0[16];
    uint64_t kernel_code_entry_byte_offset; // kernel 程式碼入口偏移
    uint8_t  reserved1[20];
    uint32_t compute_pgm_rsrc1;             // GRANULATED_WAVEFRONT_SGPR_COUNT, VGPRS, SGPRS
    uint32_t compute_pgm_rsrc2;             // SCRATCH_EN, USER_SGPR_COUNT, etc.
    uint32_t compute_pgm_rsrc3;             // COMPUTE_PGM_RSRC3 (CDNA4+)
    uint32_t kernel_code_properties;        // 啟用的功能
    uint16_t workitem_private_segment_size;
    uint16_t workgroup_group_segment_size;
    uint32_t gds_segment_size;
    uint64_t kernarg_segment_size;
    uint32_t workitem_vgpr_count;           // 每個 workitem 的 VGPR 數量
    uint16_t workitem_sgpr_count;           // 每個 workitem 的 SGPR 數量
    uint16_t reserved2;
    // ... 更多字段
};
```

---

## 第八階段：Workgroup 分發到 ComputeUnit

### dispatch_workgroups() 詳解

**位置**：[`command_processor.cpp:582-710`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L582-L710)

```
dispatch_workgroups(entry)
  │
  ├─ 1. acquire/release fence 處理
  │     根據 kernel descriptor 的 acquire_fence_overflow 標記
  │     如果需要 → 刷新所有 L2 cache 中該 VMID 的行
  │
  ├─ 2. 確定 CU 分配策略
  │     ├─ 如果有 SPI (Shader Processor Input)：
  │     │   使用 SPI 進行 CU 分配（更接近真實硬件）
  │     │
  │     └─ 否則：round-robin 分配
  │         next_cu_ 循環遞增
  │
  ├─ 3. 遍歷未分配的 workgroups
  │     for each unassigned workgroup:
  │       │
  │       ├─ 選擇目標 CU (round-robin 或 SPI)
  │       │
  │       ├─ ★ 檢查 CU 容量（all-or-nothing placement）
  │       │   必須確保 CU 有足夠的：
  │       │   - 空閒 wavefront 槽位 (>= wfs_per_wg)
  │       │   - SGPR 寄存器空間
  │       │   - VGPR 寄存器空間
  │       │   - LDS 空間
  │       │   如果任何資源不足 → 跳過此 CU
  │       │
  │       ├─ 為 workgroup 中的每個 wavefront：
  │       │   │
  │       │   ├─ cu->allocate_wavefront_slot()
  │       │   │   在 CU 上分配 wavefront 槽位
  │       │   │
  │       │   ├─ init_wavefront_regs(cu, wf, entry, global_wg_id, wf_index)
  │       │   │   │                                      ← command_processor.cpp:161
  │       │   │   │
  │       │   │   ├─ 設置 SGPR 寄存器 (AMDHSA ABI):
  │       │   │   │   s[0:1]  = workgroup_id_x, workgroup_id_y
  │       │   │   │   s[2]    = workgroup_id_z
  │       │   │   │   s[3]    = (grid_size_x * grid_size_y) 的高 32 位
  │       │   │   │   s[4:5]  = kernarg 指針（低 32 和高 32 位）
  │       │   │   │   s[6:7]  = kernel 入口地址
  │       │   │   │   s[8:9]  = completion_signal VA
  │       │   │   │   s[10]   = dispatch_id
  │       │   │   │   s[11:12]= private_segment_size, group_segment_size
  │       │   │   │   s[13:15]= 其他系統 SGPR
  │       │   │   │
  │       │   │   ├─ 設置 VGPR 寄存器:
  │       │   │   │   v[0]    = workitem_id (在 workgroup 內)
  │       │   │   │   每個 wavefront 中的 workitems 有不同的 v[0]
  │       │   │   │
  │       │   │   └─ 設置其他工作項狀態：PC, M0, EXEC, MODE, STATUS
  │       │   │
  │       │   └─ cu->activate_wavefront(wf)
  │       │        激活 wavefront，使其開始執行
  │       │
  │       └─ 更新 entry 的已分配 workgroup 計數
  │
  └─ 4. 返回實際分配的 workgroup 數量
```

### AMDHSA ABI 寄存器佈局

當一個 wavefront 被激活時，CommandProcessor 按照 AMDHSA ABI 規範初始化寄存器：

| 寄存器 | 內容 | 說明 |
|--------|------|------|
| `SGPR[0:1]` | `workgroup_id.{x,y}` | workgroup 在 grid 中的二維 ID |
| `SGPR[2]` | `workgroup_id.z` | workgroup 在 grid 中的 z ID |
| `SGPR[3]` | `grid_size.x * grid_size.y` (高32位) | 用於計算扁平化 workgroup ID |
| `SGPR[4:5]` | `kernarg_address` | kernel 參數的 GPU 虛擬地址 |
| `SGPR[6:7]` | `kernel_entry_pc` | kernel 入口程式碼地址 |
| `SGPR[8:9]` | `completion_signal` | 完成信號的 GPU 虛擬地址 |
| `SGPR[10]` | `dispatch_id` | 此次 dispatch 的唯一標識符 |
| `SGPR[11]` | `private_segment_size` | scratch 記憶體大小 |
| `SGPR[12]` | `group_segment_size` | LDS 大小 |
| `VGPR[0]` | `workitem_id` | workgroup 內的工作項 ID (0..63) |

---

## 第九階段：完成追蹤與中斷信號

### notify_wg_complete() 工作流程

**位置**：[`command_processor.cpp:1142`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp) (handle_doorbell_sync)

當 CU 中的一個 workgroup 完成時，會調用 `CommandProcessor::notify_wg_complete()`：

```
CU 完成 workgroup
  └─ CommandProcessor::notify_wg_complete(dispatch_id, wg_id)
       │
       ├─ 增加 entry.completed_wgs (原子操作)
       │
       └─ 如果 completed_wgs == total_wgs（所有 workgroup 完成）：
            │
            ├─ 寫入 completion_signal（通過 GpuMemory）
            │   write_gpu_block(signal_va, &dispatch_id, sizeof(uint64_t), vmid)
            │
            ├─ 標記 entry 為 fully_completed
            │
            ├─ CompletionTracker 檢查隊列完成狀態
            │   如果所有 entries 都完成，隊列進入空閒狀態
            │
            └─ interrupt_cb_(process_id, event_id)
                 觸發 KFD 事件信號
                 └─ EventState::notify()
                      寫入信號頁面 (signal_page[event_id] = 1)
                      └─ ROCR wait_events 線程檢測到信號
                           kernel 完成通知傳播回 HIP runtime
```

### CompletionTracker

`CompletionTracker` 確保完成信號**按提交順序**寫入，模擬真實硬件上的隊列順序完成語義。如果 entry[N] 在 entry[N-1] 之前完成，信號會被延遲，直到所有前序 entries 也完成。

---

## 完整調用鏈圖

```
┌──────────────────────────────────────────────────────────────────────────┐
│  HIP 應用程序 (Python/PyTorch/HIP)                                        │
│  hipLaunchKernel / hipModuleLaunchKernel                                  │
└──────────────────────────────────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  ROCR (HSA Runtime) + libhsakmt (KFD thunk)                              │
│                                                                          │
│  [Layer 2: HSA_TOOLS_LIB → librocjitsu_hooks.so]                         │
│    ├─ OnLoad() → 替換 CoreApiTable 中的函數指針                           │
│    ├─ hsa_code_object_reader_create_from_memory() → 記錄 ELF              │
│    └─ hsa_executable_load_agent_code_object() → BinaryTranslator::translate│
│                                                                          │
│  [Layer 1: LD_PRELOAD → librocjitsu_kmd.so]                              │
│    │                                                                     │
│    ├─ open("/dev/kfd")                                                   │
│    │   └─ interposer.cpp:463                                             │
│    │       └─ InterposerContext::get_or_create()                         │
│    │           ├─ rj_vm_create()           ← rj_vm.cpp:143               │
│    │           │   ├─ config::load_config() ← 解析 JSON 拓撲             │
│    │           │   └─ create_from_loaded() ← rj_vm.cpp:34                │
│    │           │       ├─ new VirtualMachine(soc, daemon)                │
│    │           │       │   └─ new SimulatedDriver(*soc)                  │
│    │           │       │       ← virtual_machine.cpp:28                  │
│    │           │       ├─ engine->build()                                │
│    │           │       ├─ driver->setup_topology()                       │
│    │           │       └─ driver->open()                                 │
│    │           └─ std::thread([] { rj_vm_run(vm); }).detach()           │
│    │               ← interposer.cpp:379                                  │
│    │                                                                     │
│    ├─ ioctl(kfd_fd, AMDKFD_IOC_ALLOC_MEMORY, ...)                        │
│    │   └─ interposer.cpp:686                                             │
│    │       └─ SimulatedDriver::ioctl()                                   │
│    │           └─ dispatch_ioctl() → alloc_memory_ioctl()                │
│    │                                                                     │
│    ├─ ioctl(kfd_fd, AMDKFD_IOC_CREATE_QUEUE, ...)                        │
│    │   └─ interposer.cpp:686                                             │
│    │       └─ SimulatedDriver::create_queue_ioctl()                      │
│    │           ← simulated_driver.cpp:1208                               │
│    │           └─ cp->register_queue(hw)                                 │
│    │               ← command_processor.cpp:371                           │
│    │               └─ 啟動 doorbell_thread_                              │
│    │                   ← command_processor.cpp:398                       │
│    │                                                                     │
│    └─ [Kernel Launch: ROCR 寫入 AQL packet + 寫入 doorbell]              │
│                                                                          │
└──────────────────────────────────────────────────────────────────────────┘
    │
    ▼  (doorbell write 被 CP 輪詢線程檢測到)
┌──────────────────────────────────────────────────────────────────────────┐
│  CommandProcessor (command_processor.cpp)                                 │
│                                                                          │
│  doorbell_poll_loop()                    ← command_processor.cpp:504     │
│    ├─ scan_doorbells()                   ← command_processor.cpp:474     │
│    │   └─ atomic_ref<uint64_t> 讀取 doorbell 值                          │
│    │                                                                     │
│    └─ engine()->schedule_event_now(&doorbell_event_)                     │
│                                                                          │
│  handle_doorbell(ts)                     ← command_processor.cpp:1142    │
│    └─ handle_doorbell_sync(ts)           ← command_processor.cpp:1144    │
│        ├─ fetch_packets()                                                 │
│        │   └─ fetch_from_queue()         ← command_processor.cpp:925     │
│        │       ├─ read_gpu_u64(write_ptr_va)  // 讀取 write pointer      │
│        │       ├─ read_gpu_u64(read_ptr_va)   // 讀取 read pointer       │
│        │       ├─ read_gpu_block(&pkt, 64)    // 讀取 AQL packet         │
│        │       └─ process_aql_packet()   ← command_processor.cpp:870     │
│        │           └─ read_kernel_descriptor()                           │
│        │               ← command_processor.cpp:752                       │
│        │                                                                 │
│        └─ process_queues()               ← command_processor.cpp:718     │
│            └─ dispatch_workgroups(entry) ← command_processor.cpp:582     │
│                ├─ CU 容量檢查 (all-or-nothing)                            │
│                ├─ init_wavefront_regs()  (AMDHSA ABI)                    │
│                └─ cu->activate_wavefront(wf)                             │
│                                                                          │
│  notify_wg_complete(dispatch_id, wg_id) ← command_processor.cpp:148     │
│    └─ 所有 WG 完成後 → interrupt_cb_() → EventState::notify()            │
└──────────────────────────────────────────────────────────────────────────┘
    │
    ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  ComputeUnit (compute_unit.cpp)                                          │
│    └─ Wavefront 執行 → ISA 解碼 → 指令執行                               │
└──────────────────────────────────────────────────────────────────────────┘
```

---

## 雙層攔截的協同工作

### Layer 2 (HSA hooks) 做什麼？

在 kernel 被提交到 AQL 隊列**之前**，HSA hooks 攔截 code object 的加載過程：

1. **攔截 code object 讀取**：`rj_code_object_reader_create_from_memory()` 將原始 ELF 字節記錄到 `CodeObjectReaderRegistry` 中
2. **攔截 code object 加載**：`rj_executable_load_agent_code_object()` 檢查 ELF 的目標 ISA
   - 如果 source ISA != target ISA → 調用 `BinaryTranslator::translate()` 做 DBT
   - 用翻譯後的 ELF 替換原始 ELF
   - 這樣下游客戶（ROCR、CP）看到的已經是目標 ISA 的 code object

### Layer 1 (LD_PRELOAD) 做什麼？

Layer 1 處理所有需要內核驅動參與的操作：

1. **設備發現**：偽造 sysfs 拓撲文件，讓 ROCR 認為存在一個真實 GPU
2. **內存管理**：模擬 VRAM 分配、GPU VA 映射
3. **隊列管理**：創建/銷毀 AQL 隊列，註冊到 CommandProcessor
4. **門鈴頁面**：提供 mmap 的 doorbell 頁面，讓 ROCR 可以直接寫入
5. **事件信號**：管理信號頁面，用於 kernel 完成通知

### 為什麼需要兩層？

兩層攔截對應兩個不同的接口表面：

- **Layer 2 (HSA API)** 是 ROCR 對上層（HIP runtime）的接口。通過 `HSA_TOOLS_LIB` 攔截，可以在 ROCR 處理 code object 之前進行 DBT 翻譯
- **Layer 1 (KFD syscall)** 是 ROCR 對下層（內核驅動）的接口。通過 `LD_PRELOAD` 攔截，可以完全繞過真實內核驅動，在用戶空間模擬所有 KFD 功能

兩層**獨立運作、互不干擾**。Layer 2 專注於 code object 翻譯，Layer 1 專注於硬件模擬。

---

## 關鍵文件索引

### 啟動與配置

| 文件 | 角色 |
|------|------|
| [`tools/rocjitsu/main.cpp:429-430`](tools/rocjitsu/main.cpp#L429-L430) | CLI 設置 LD_PRELOAD 並 exec |
| [`tools/rocjitsu/main.cpp:37-120`](tools/rocjitsu/main.cpp) | `run_daemon_server()` 和 `handle_client()` |
| [`configs/amdgpu_cdna4_kmd.json`](configs/amdgpu_cdna4_kmd.json) | MI350X CDNA4 模擬配置 |
| [`lib/rocjitsu/src/rocjitsu/config/config_loader.h`](lib/rocjitsu/src/rocjitsu/config/config_loader.h) | JSON 配置加載和拓撲構建 |

### Layer 1: LD_PRELOAD 攔截 (librocjitsu_kmd.so)

| 文件 | 關鍵行號 | 角色 |
|------|---------|------|
| [`interposer.cpp:419`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L419) | Constructor | `__attribute__((constructor))` 入口 |
| [`interposer.cpp:87-138`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L87-L138) | `LibcPassthrough` | 保存真實 libc 函數指針 |
| [`interposer.cpp:141-417`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L141-L417) | `InterposerContext` | 攔截器核心狀態管理 |
| [`interposer.cpp:314-379`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L314-L379) | `get_or_create()` | VM 創建和引擎啟動 |
| [`interposer.cpp:425-474`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L425-L474) | `open()` 攔截 | `/dev/kfd` 和 `/dev/dri/renderD*` 攔截 |
| [`interposer.cpp:626-693`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L626-L693) | `ioctl()` 攔截 | KFD ioctl 路由 |
| [`interposer.cpp:830-870`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L830-L870) | `mmap()` 攔截 | 門鈴頁面和事件頁面 mmap |
| [`interposer.cpp:620-624`](lib/rocjitsu/src/rocjitsu/kmd/linux/interposer.cpp#L620-L624) | `close()` 攔截 | KFD fd 生命週期管理 |

### KMD 模擬驅動 (SimulatedDriver)

| 文件 | 關鍵行號 | 角色 |
|------|---------|------|
| [`simulated_driver.cpp:262-310`](lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.cpp#L262-L310) | `open()` | KFD process 創建和 CP 回調初始化 |
| [`simulated_driver.cpp:588-690`](lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.cpp#L588-L690) | `dispatch_ioctl()` | KFD ioctl 命令分發器 |
| [`simulated_driver.cpp:1208-1281`](lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.cpp#L1208-L1281) | `create_queue_ioctl()` | **★ 創建隊列 → cp->register_queue()** |
| [`simulated_driver.cpp:1284-1310`](lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.cpp#L1284-L1310) | `destroy_queue_ioctl()` | 銷毀隊列 → cp->unregister_queue() |
| [`simulated_driver.h`](lib/rocjitsu/src/rocjitsu/kmd/linux/simulated_driver.h) | 類定義 | GpuDevice, KfdProcess, EventState 結構 |

### VM 核心

| 文件 | 關鍵行號 | 角色 |
|------|---------|------|
| [`virtual_machine.cpp:23-29`](lib/rocjitsu/src/rocjitsu/vm/virtual_machine.cpp#L23-L29) | 構造函數 | 創建 SimulatedDriver |
| [`rj_vm.cpp:34-113`](lib/rocjitsu/src/rocjitsu/vm/rj_vm.cpp#L34-L113) | `create_from_loaded()` | VM 和引擎的完整構建 |
| [`rj_vm.cpp:143-152`](lib/rocjitsu/src/rocjitsu/vm/rj_vm.cpp#L143-L152) | `rj_vm_create()` | 公共 C API 入口 |
| [`rj_vm_impl.h`](lib/rocjitsu/src/rocjitsu/vm/rj_vm_impl.h) | `rj_vm_t` | VM 內部結構定義 |

### CommandProcessor (核心目標文件)

| 文件 | 關鍵行號 | 方法 | 角色 |
|------|---------|------|------|
| [`command_processor.h:86-268`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.h#L86-L268) | 類定義 | `CommandProcessor` | CP 硬件模型 |
| [`command_processor.h:54-68`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.h#L54-L68) | `HwQueue` | AQL 硬件隊列結構 |
| [`command_processor.cpp:150`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L150) | 構造函數 | CP 實例化 |
| [`command_processor.cpp:362-369`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L362-L369) | `startup()` | 初始化門鈴事件處理器和 CompletionTracker |
| [`command_processor.cpp:371-399`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L371-L399) | `register_queue()` | **★ 註冊隊列，啟動門鈴線程** |
| [`command_processor.cpp:402-415`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L402-L415) | `unregister_queue()` | 取消註冊隊列 |
| [`command_processor.cpp:474-502`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L474-L502) | `scan_doorbells()` | **★ 掃描所有隊列的門鈴值** |
| [`command_processor.cpp:504-551`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L504-L551) | `doorbell_poll_loop()` | **★ 門鈴輪詢主循環** |
| [`command_processor.cpp:553-568`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L553-L568) | `schedule_next_queue()` | 隊列調度（round-robin） |
| [`command_processor.cpp:570-579`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L570-L579) | `barrier_satisfied()` | Barrier 依賴檢查 |
| [`command_processor.cpp:582-710`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L582-L710) | `dispatch_workgroups()` | **★ 向 CU 分發 workgroups** |
| [`command_processor.cpp:712-741`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L712-L741) | `step()` / `process_queues()` | 處理所有隊列的 dispatch entries |
| [`command_processor.cpp:752-760`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L752-L760) | `read_kernel_descriptor()` | 讀取 AMD HSA kernel descriptor |
| [`command_processor.cpp:870-922`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L870-L922) | `process_aql_packet()` | **★ 解析 AQL packet，創建 DispatchEntry** |
| [`command_processor.cpp:925-979`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L925-L979) | `fetch_from_queue()` | **★ 從 ring buffer 讀取 AQL packets** |
| [`command_processor.cpp:1142-1144`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/command_processor.cpp#L1142-L1144) | `handle_doorbell()` | 門鈴事件處理入口 |

### Layer 2: HSA DBT Hooks (librocjitsu_hooks.so)

| 文件 | 關鍵行號 | 角色 |
|------|---------|------|
| [`rj_hsa_dbt_hooks.cpp:703-722`](lib/rocjitsu/src/rocjitsu/hooks/rj_hsa_dbt_hooks.cpp#L703-L722) | `OnLoad()` / `OnUnload()` | HSA tools 入口點 |
| [`rj_hsa_dbt_hooks.cpp:392-426`](lib/rocjitsu/src/rocjitsu/hooks/rj_hsa_dbt_hooks.cpp#L392-L426) | `RjHsaLayer::install()` | 替換 API table 函數指針 |
| [`rj_hsa_dbt_hooks.cpp:525-687`](lib/rocjitsu/src/rocjitsu/hooks/rj_hsa_dbt_hooks.cpp#L525-L687) | Wrapper 函數 | 攔截 code object 加載 |
| [`rj_hsa_dbt_hooks.cpp:276-371`](lib/rocjitsu/src/rocjitsu/hooks/rj_hsa_dbt_hooks.cpp#L276-L371) | `CodeObjectReaderRegistry` | ELF 字節暫存 |

### 輔助文件

| 文件 | 角色 |
|------|------|
| [`lib/rocjitsu/src/rocjitsu/vm/amdgpu/gpu_memory.h`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/gpu_memory.h) | GPU 虛擬地址空間管理 |
| [`lib/rocjitsu/src/rocjitsu/vm/amdgpu/compute_unit.h`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/compute_unit.h) | ComputeUnit 接口 |
| [`lib/rocjitsu/src/rocjitsu/vm/amdgpu/compute_unit.cpp`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/compute_unit.cpp) | CU 實現（wavefront 執行） |
| [`lib/rocjitsu/src/rocjitsu/vm/amdgpu/completion_tracker.h`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/completion_tracker.h) | 完成追蹤器 |
| [`lib/rocjitsu/src/rocjitsu/vm/amdgpu/dispatch_entry.h`](lib/rocjitsu/src/rocjitsu/vm/amdgpu/dispatch_entry.h) | DispatchEntry 數據結構 |
| [`lib/rocjitsu/src/rocjitsu/vm/driver.h`](lib/rocjitsu/src/rocjitsu/vm/driver.h) | Driver 抽象接口 |
| [`lib/rocjitsu/src/rocjitsu/kmd/linux/sysfs.h`](lib/rocjitsu/src/rocjitsu/kmd/linux/sysfs.h) | 模擬 sysfs 拓撲 |
| [`lib/rocjitsu/src/rocjitsu/kmd/linux/events.h`](lib/rocjitsu/src/rocjitsu/kmd/linux/events.h) | KFD 事件子系統 |
| [`lib/rocjitsu/src/rocjitsu/kmd/linux/kfd_process.h`](lib/rocjitsu/src/rocjitsu/kmd/linux/kfd_process.h) | 每進程 KFD 狀態 |
| [`lib/rocjitsu/src/rocjitsu/code/dbt/binary_translator.h`](lib/rocjitsu/src/rocjitsu/code/dbt/binary_translator.h) | 二進制翻譯器接口 |

### 相關文檔

| 文檔 | 內容 |
|------|------|
| [`docs/architecture.md`](docs/architecture.md) | 整體架構概覽 |
| [`docs/vm-design.md`](docs/vm-design.md) | AMDGPU VM 硬件模型設計 |
| [`docs/rocjitsu-cli.md`](docs/rocjitsu-cli.md) | CLI 和守護進程 RPC 協議 |
| [`docs/dbt-design.md`](docs/dbt-design.md) | 動態二進制翻譯設計 |
| [`docs/simdojo.md`](docs/simdojo.md) | Simdojo 模擬框架設計 |

---

## 數據結構附錄

### HwQueue（硬件隊列）

```cpp
// command_processor.h:54-68
struct HwQueue {
    uint32_t process_id;         // KFD 進程 ID
    uint32_t queue_id;           // 隊列 ID（每個進程唯一）
    uint64_t ring_base_va;       // ring buffer 的 GPU 虛擬地址
    uint32_t ring_size;          // ring buffer 大小（字節）
    uint64_t read_ptr_va;        // read pointer 的 GPU VA
    uint64_t write_ptr_va;       // write pointer 的 GPU VA
    uint32_t doorbell_offset;    // 門鈴頁面內的偏移
    void    *doorbell_base;      // 門鈴頁面的主機指針
    uint64_t doorbell_va;        // 門鈴的 GPU VA（內部測試隊列）
    uint64_t last_doorbell;      // 上次檢測到的門鈴值（用於變化檢測）
    bool     host_accessible;    // true = KFD 隊列（門鈴在主機內存中）
    bool     is_sdma;            // true = SDMA 隊列
    uint64_t queue_desc_va;      // amd_queue_t 描述符的 GPU VA
};
```

### HwQueueState（隊列狀態——CP 內部追蹤）

```cpp
struct HwQueueState {
    uint64_t queue_desc_va;      // amd_queue_t 描述符地址
    std::vector<DispatchEntry> entries;  // 已解析但可能未分發的 dispatch entries
    size_t next_dispatch_idx;    // 下一個要分發的 entry 索引
    bool implicit_barrier_next;  // 下一個 packet 需要等待前序 entries 完成
};
```

### DispatchEntry（Dispatch 項）

```cpp
struct DispatchEntry {
    uint32_t dispatch_id;        // 全局唯一的 dispatch 標識符
    uint64_t entry_pc;           // kernel 入口程式碼的 GPU 虛擬地址
    uint32_t sgprs_per_wf;       // 每個 wavefront 的 SGPR 數量
    uint32_t vgprs_per_wf;       // 每個 wavefront 的 VGPR 數量
    uint32_t wfs_per_wg;         // 每個 workgroup 的 wavefront 數量
    uint32_t total_wgs;          // 總 workgroup 數量
    std::atomic<uint32_t> completed_wgs;  // 已完成的 workgroup 計數
    uint64_t kernarg_addr;       // kernel 參數的 GPU VA
    uint64_t completion_signal;  // 完成信號的 GPU VA
    uint32_t process_id;         // 擁有進程 ID
    bool barrier_bit;            // barrier packet 標記
    amdhsa::kernel_descriptor_t kd;  // AMD HSA kernel descriptor
    // ... 其他字段
};
```

### AQL Dispatch Packet（HSA 標準）

```cpp
// 來自 hsa/hsa.h
struct hsa_kernel_dispatch_packet_t {
    uint16_t header;             // packet 類型 + barrier bit
    uint16_t setup;              // 維度、memory order
    uint16_t workgroup_size_x;   // workgroup 在 X 維度的大小
    uint16_t workgroup_size_y;   // workgroup 在 Y 維度的大小
    uint16_t workgroup_size_z;   // workgroup 在 Z 維度的大小
    uint16_t reserved0;
    uint32_t grid_size_x;        // grid 在 X 維度的大小
    uint32_t grid_size_y;        // grid 在 Y 維度的大小
    uint32_t grid_size_z;        // grid 在 Z 維度的大小
    uint32_t private_segment_size; // 每個 workitem 的私有內存
    uint32_t group_segment_size; // 每個 workgroup 的 LDS 大小
    uint64_t kernel_object;      // kernel code object 的 GPU VA
    uint64_t kernarg_address;    // kernel 參數的 GPU VA
    uint64_t reserved1;
    uint64_t completion_signal;  // 完成信號的 GPU VA (hsa_signal_t)
};  // 總共 64 bytes
```

---

## 運行時環境變量總結

| 環境變量 | 設置者 | 作用 |
|---------|-------|------|
| `LD_PRELOAD` | rocjitsu CLI | 注入 `librocjitsu_kmd.so` |
| `HSA_TOOLS_LIB` | rocjitsu CLI/用戶 | 注入 `librocjitsu_hooks.so` |
| `RJ_DBT_TARGET_ISA` | 用戶/HSA hooks | DBT 目標 ISA（例如 `gfx1201`） |
| `RJ_DBT_SOURCE_ISA` | 用戶/HSA hooks | DBT 源 ISA 覆蓋 |
| `RJ_DBT_LOG` | 用戶/HSA hooks | DBT 日誌級別 |
| `RJ_LOG` | 用戶 | 啟用 kernel 日誌插件 |
| `RJ_RACE` | 用戶 | 啟用 race detector 插件 |
| `RJ_SINKS` | 用戶 | 日誌輸出 sink（stderr/stdout/file） |
| `RJ_SINK_DIR` | 用戶 | 文件 sink 的輸出目錄 |
| `RJ_CP_PROFILE` | 用戶 | CP dispatch 性能分析 |
| `RJ_USE_PROFILED_EXECUTION_PLUGIN_GROUP` | 用戶 | 使用性能優化的插件組 |
| `ROCJITSU_RUNTIME_DIR` | 用戶/CLI | 覆蓋運行時文件目錄 |
| `XDG_RUNTIME_DIR` | 系統 | 運行時文件基礎目錄 |

---

## 一句話總結
HIP kernel launch → ROCR 寫 AQL packet 到 ring buffer 並寫 doorbell → librocjitsu_kmd.so（LD_PRELOAD）攔截了 mmap 使得 doorbell 頁面在用戶空間可見 → CommandProcessor::doorbell_poll_loop 輪詢檢測到 doorbell 變化 → fetch_from_queue 從 ring buffer 讀取 AQL dispatch packet → dispatch_workgroups 將 workgroups 分發到 ComputeUnit 上執行。

---

> 阿彌陀佛🏵️🏵️🙏
>
> 本文檔追蹤了從 HIP 應用程序到 `vm/amdgpu/command_processor.cpp` 的完整調用鏈。
> 核心路徑可總結為：
>
> **HIP kernel launch**
> → **ROCR 寫入 AQL packet 到 ring buffer 並寫入 doorbell**
> → **`librocjitsu_kmd.so` (LD_PRELOAD) 提供 mmap 的 doorbell 頁面**
> → **`CommandProcessor::doorbell_poll_loop` 輪詢檢測到 doorbell 值變化**
> → **`fetch_from_queue` 從 ring buffer 讀取 AQL dispatch packet**
> → **`dispatch_workgroups` 將 workgroups 分發到 ComputeUnit 上執行**
> → **`notify_wg_complete` → interrupt_cb → KFD 事件信號 → ROCR 收到完成通知**
