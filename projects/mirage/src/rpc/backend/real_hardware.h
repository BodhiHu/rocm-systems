// real_hardware.h — Backend that forwards all requests to real GPU hardware
#pragma once

#include "../server.h"
#include <atomic>
#include <cstdint>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace amdgpu_proxy {

/// A server-side mmap region tracked for doorbell/event relay.
struct ServerMmapRegion {
    enum Type { DOORBELL, EVENT, GPU_MEMORY };
    Type type;
    void* real_mapping;      // server's mmap of real device memory
    void* memfd_mapping;     // server's mmap of shared memfd
    uint64_t length;
    int memfd_fd;            // server-side memfd fd
    std::vector<uint64_t> last_values; // for change detection (doorbell relay)
};

/// RealHardwareBackend opens real GPU device files and forwards all ioctl
/// requests to the actual kernel driver. Each client Open creates a real
/// device fd; ioctls are forwarded as-is; Close releases the fd.
///
/// mmap requests are handled server-side:
///   - Doorbell/event pages: memfd + polling relay thread
///   - GPU memory (VRAM/GTT): dma-buf export via EXPORT_DMABUF/PRIME, or memfd fallback
class RealHardwareBackend : public IBackend {
public:
    RealHardwareBackend();
    ~RealHardwareBackend() override;

    BackendResponse HandleRequest(const uint8_t* data, uint32_t size) override;

private:
    BackendResponse HandleOpen(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleClose(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleInfo(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleGemCreate(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleGemMmap(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleCtx(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleGemVa(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleDrmVersion(const uint8_t* data, uint32_t size);
    BackendResponse HandleMmap(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleSysfsRead(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleRawDrmIoctl(const uint8_t* data, uint32_t size);

    // KFD handlers
    std::vector<uint8_t> HandleKfdGetVersion(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdAcquireVm(const uint8_t* data, uint32_t size);
    BackendResponse HandleKfdAllocMemory(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdFreeMemory(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdMapMemory(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdUnmapMemory(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdCreateQueue(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdDestroyQueue(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdCreateEvent(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdDestroyEvent(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdSetEvent(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdResetEvent(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdWaitEvents(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdGetProcessApertures(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdGetClockCounters(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdSetMemoryPolicy(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdAvailableMemory(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdSetXnackMode(const uint8_t* data, uint32_t size);
    BackendResponse HandleKfdSvm(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdRuntimeEnable(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdSetScratchBackingVa(const uint8_t* data, uint32_t size);
    std::vector<uint8_t> HandleKfdSetTrapHandler(const uint8_t* data, uint32_t size);

    std::vector<uint8_t> BuildErrorResponse(int32_t err, const char* msg);

    /// Handle KFD fd mmap (doorbell, event, reserved, mmio).
    BackendResponse HandleKfdMmap(int real_fd, uint64_t offset, uint64_t length, int prot, int flags);

    /// Handle DRM render fd mmap (GPU memory via dma-buf export or memfd fallback).
    BackendResponse HandleDrmMmap(int real_fd, uint64_t offset, uint64_t length, int prot, int flags);

    /// Try to export a KFD allocation as dma-buf. Returns fd or -1.
    int ExportKfdDmaBuf(uint64_t kfd_handle);

    /// Try to export a DRM GEM handle as dma-buf. Returns fd or -1.
    int ExportDrmPrime(int drm_fd, uint32_t gem_handle);

    /// Create a memfd backed by a copy of a real mapping.
    /// Returns {memfd_fd, memfd_mapping} or {-1, nullptr} on failure.
    std::pair<int, void*> CreateMemfdCopy(void* real_mapping, uint64_t length);

    /// Get or lazily open a real device fd for the given path.
    int GetDeviceFd(const std::string& path);

    /// Start the relay thread (if not already running).
    void StartRelayThread();

    /// Relay thread function: polls memfd doorbells → real MMIO, real events → memfd.
    void RelayLoop();

    /// Sync all tracked GPU_MEMORY regions (memfd → real) before doorbell write.
    void SyncMemoryToGpu();

    /// Sync all tracked GPU_MEMORY regions (real → memfd) after GPU work completes.
    void SyncMemoryFromGpu();

    std::mutex mutex_;

    // Maps virtual_fd → real kernel fd
    std::unordered_map<int32_t, int> vfd_to_real_;
    // Maps virtual_fd → device path (to determine device type)
    std::unordered_map<int32_t, std::string> vfd_to_path_;
    // Maps device path → real kernel fd (shared across opens of same path)
    std::unordered_map<std::string, int> path_to_fd_;
    int32_t next_virtual_fd_ = 100;

    // Allocation tracking for dma-buf export at mmap time
    // Maps DRM mmap_offset → KFD handle (for EXPORT_DMABUF)
    std::unordered_map<uint64_t, uint64_t> mmap_offset_to_kfd_handle_;
    // Maps KFD handle → gpu_id (for MAP_MEMORY debugging/fallback)
    std::unordered_map<uint64_t, uint32_t> alloc_handle_to_gpu_;
    // Handles that were pre-MAP'd to allocating GPU during ALLOC
    std::unordered_set<uint64_t> pre_mapped_handles_;
    // Maps DRM mmap_offset → {gem_handle, drm_fd} (for PRIME_HANDLE_TO_FD)
    struct GemInfo { uint32_t handle; int drm_fd; };
    std::unordered_map<uint64_t, GemInfo> mmap_offset_to_gem_;

    // SVM address translation: client_va → server_va
    struct SvmMapping { uint64_t server_addr; uint64_t size; void* server_mapping; int memfd_fd; };
    std::unordered_map<uint64_t, SvmMapping> svm_addr_map_;

    // USERPTR address translation: client_va → server_va (for CREATE_QUEUE address translation)
    struct UserptrMapping { uint64_t server_va; uint64_t size; };
    std::unordered_map<uint64_t, UserptrMapping> userptr_addr_map_;

    // Server-side mmap regions for relay
    std::mutex relay_mutex_;
    std::vector<ServerMmapRegion> relay_regions_;
    std::thread relay_thread_;
    std::atomic<bool> relay_running_{false};
};

} // namespace amdgpu_proxy
