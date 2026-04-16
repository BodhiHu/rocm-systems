// real_hardware.cpp — Backend forwarding all requests to real GPU hardware
#include "real_hardware.h"
#include "rpc_generated.h"
#include "rpc/str_util.h"

#include <cerrno>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <fstream>
#include <sstream>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <linux/memfd.h>
#include <sys/syscall.h>
#include <unistd.h>
#include <sched.h>

// DRM ioctl definitions
#include <drm/drm.h>
#include <drm/amdgpu_drm.h>

// KFD ioctl definitions — use the tree's headers for newer ioctls
#include <linux/kfd_ioctl.h>

// Fallback definitions for newer ioctls not in system headers
#ifndef AMDKFD_IOC_AVAILABLE_MEMORY
struct kfd_ioctl_get_available_memory_args {
    __u64 available;
    __u32 gpu_id;
    __u32 pad;
};
#define AMDKFD_IOC_AVAILABLE_MEMORY \
    _IOWR('K', 0x23, struct kfd_ioctl_get_available_memory_args)
#endif

// KFD export dma-buf (0x24)
#ifndef AMDKFD_IOC_EXPORT_DMABUF
struct kfd_ioctl_export_dmabuf_args {
    __u64 handle;
    __u32 flags;
    __u32 dmabuf_fd;
};
#define AMDKFD_IOC_EXPORT_DMABUF \
    _IOWR('K', 0x24, struct kfd_ioctl_export_dmabuf_args)
#endif

// KFD runtime enable (0x25)
#ifndef AMDKFD_IOC_RUNTIME_ENABLE
struct kfd_ioctl_runtime_enable_args {
    __u64 r_debug;
    __u32 mode_mask;
    __u32 capabilities_mask;
};
#define AMDKFD_IOC_RUNTIME_ENABLE \
    _IOWR('K', 0x25, struct kfd_ioctl_runtime_enable_args)
#endif

// KFD mmap type encoding (top 2 bits of offset)
#define KFD_MMAP_TYPE_SHIFT    62
#define KFD_MMAP_TYPE_MASK     (0x3ULL << KFD_MMAP_TYPE_SHIFT)
#define KFD_MMAP_TYPE_DOORBELL (0x3ULL << KFD_MMAP_TYPE_SHIFT)
#define KFD_MMAP_TYPE_EVENTS   (0x2ULL << KFD_MMAP_TYPE_SHIFT)
#define KFD_MMAP_TYPE_RESERVED_MEM (0x1ULL << KFD_MMAP_TYPE_SHIFT)
#define KFD_MMAP_TYPE_MMIO     (0x0ULL << KFD_MMAP_TYPE_SHIFT)

using namespace flatbuffers;

namespace amdgpu_proxy {

RealHardwareBackend::RealHardwareBackend() = default;

RealHardwareBackend::~RealHardwareBackend() {
    // Stop relay thread
    relay_running_.store(false);
    if (relay_thread_.joinable()) relay_thread_.join();

    // Clean up relay regions
    {
        std::lock_guard<std::mutex> lock(relay_mutex_);
        for (auto& r : relay_regions_) {
            if (r.memfd_mapping) ::munmap(r.memfd_mapping, r.length);
            if (r.real_mapping) ::munmap(r.real_mapping, r.length);
            if (r.memfd_fd >= 0) ::close(r.memfd_fd);
        }
        relay_regions_.clear();
    }

    std::lock_guard<std::mutex> lock(mutex_);
    for (auto& [path, fd] : path_to_fd_) {
        if (fd >= 0) ::close(fd);
    }
    path_to_fd_.clear();
    vfd_to_real_.clear();
}

std::vector<uint8_t> RealHardwareBackend::BuildErrorResponse(
        int32_t err, const char* msg) {
    FlatBufferBuilder fbb(256);
    auto err_str = AmdgpuProxy::make_str<AmdgpuProxy::Str255>(msg);
    auto error = AmdgpuProxy::CreateError(fbb, err, &err_str);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_NONE, 0, error);
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

int RealHardwareBackend::GetDeviceFd(const std::string& path) {
    auto it = path_to_fd_.find(path);
    if (it != path_to_fd_.end()) return it->second;

    int fd = ::open(path.c_str(), O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        std::fprintf(stderr, "[real-hw] Failed to open %s: %s\n",
                     path.c_str(), strerror(errno));
        return -1;
    }
    std::fprintf(stderr, "[real-hw] Opened %s → fd %d\n", path.c_str(), fd);

    // Enable XNACK mode for /dev/kfd to allow SVM demand-paging
    if (path == "/dev/kfd") {
        struct kfd_ioctl_set_xnack_mode_args xnack{};
        xnack.xnack_enabled = 1;
        if (::ioctl(fd, AMDKFD_IOC_SET_XNACK_MODE, &xnack) < 0) {
            fprintf(stderr, "[server] SET_XNACK_MODE failed: errno=%d (%s) - XNACK not available\n",
                    errno, strerror(errno));
        } else {
            fprintf(stderr, "[server] XNACK mode enabled: xnack_enabled=%d\n",
                    xnack.xnack_enabled);
        }
    }

    path_to_fd_[path] = fd;
    return fd;
}

BackendResponse RealHardwareBackend::HandleRequest(
        const uint8_t* data, uint32_t size) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    if (!msg) return {BuildErrorResponse(-1, "Invalid RPC message"), -1};


    switch (msg->request_type()) {
        case AmdgpuProxy::RequestPayload_OpenRequest:
            return HandleOpen(data, size);
        case AmdgpuProxy::RequestPayload_CloseRequest:
            return {HandleClose(data, size), -1};
        case AmdgpuProxy::RequestPayload_InfoRequest:
            return {HandleInfo(data, size), -1};
        case AmdgpuProxy::RequestPayload_GemCreateRequest:
            return {HandleGemCreate(data, size), -1};
        case AmdgpuProxy::RequestPayload_GemMmapRequest:
            return {HandleGemMmap(data, size), -1};
        case AmdgpuProxy::RequestPayload_CtxRequest:
            return {HandleCtx(data, size), -1};
        case AmdgpuProxy::RequestPayload_GemVaRequest:
            return {HandleGemVa(data, size), -1};
        case AmdgpuProxy::RequestPayload_DrmVersionRequest:
            return {HandleDrmVersion(data, size), -1};
        case AmdgpuProxy::RequestPayload_MmapRequest:
            return HandleMmap(data, size);
        case AmdgpuProxy::RequestPayload_SysfsReadRequest:
            return {HandleSysfsRead(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdGetVersionRequest:
            return {HandleKfdGetVersion(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdAcquireVmRequest:
            return {HandleKfdAcquireVm(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdAllocMemoryRequest:
            return HandleKfdAllocMemory(data, size);
        case AmdgpuProxy::RequestPayload_KfdFreeMemoryRequest:
            return {HandleKfdFreeMemory(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdMapMemoryToGpuRequest:
            return {HandleKfdMapMemory(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdUnmapMemoryFromGpuRequest:
            return {HandleKfdUnmapMemory(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdCreateQueueRequest:
            return {HandleKfdCreateQueue(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdDestroyQueueRequest:
            return {HandleKfdDestroyQueue(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdCreateEventRequest:
            return {HandleKfdCreateEvent(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdDestroyEventRequest:
            return {HandleKfdDestroyEvent(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdSetEventRequest:
            return {HandleKfdSetEvent(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdResetEventRequest:
            return {HandleKfdResetEvent(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdWaitEventsRequest:
            return {HandleKfdWaitEvents(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdGetProcessAperturesRequest:
            return {HandleKfdGetProcessApertures(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdGetClockCountersRequest:
            return {HandleKfdGetClockCounters(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdSetMemoryPolicyRequest:
            return {HandleKfdSetMemoryPolicy(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdAvailableMemoryRequest:
            return {HandleKfdAvailableMemory(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdSetXnackModeRequest:
            return {HandleKfdSetXnackMode(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdSvmRequest:
            return HandleKfdSvm(data, size);
        case AmdgpuProxy::RequestPayload_KfdRuntimeEnableRequest:
            return {HandleKfdRuntimeEnable(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdSetScratchBackingVaRequest:
            return {HandleKfdSetScratchBackingVa(data, size), -1};
        case AmdgpuProxy::RequestPayload_KfdSetTrapHandlerRequest:
            return {HandleKfdSetTrapHandler(data, size), -1};
        default:
            fprintf(stderr, "[server] Unhandled request type: %d\n", (int)msg->request_type());
            return {BuildErrorResponse(ENOSYS, "Unhandled request type"), -1};
    }
}

// ---- File operations ----

BackendResponse RealHardwareBackend::HandleOpen(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_OpenRequest();

    std::string path = req ? AmdgpuProxy::str_to_string(req->args()->path()) : "";
    if (path.empty()) return {BuildErrorResponse(EINVAL, "Empty path"), -1};

    std::lock_guard<std::mutex> lock(mutex_);

    int real_fd = GetDeviceFd(path);
    if (real_fd < 0) return {BuildErrorResponse(errno, "Failed to open device"), -1};

    // No fd passing — all operations stay server-side.
    // KFD binds mm_struct at open() time, so the client can never use a
    // KFD fd directly. All ioctls and mmaps go through the proxy.

    int32_t vfd = next_virtual_fd_++;
    vfd_to_real_[vfd] = real_fd;
    vfd_to_path_[vfd] = path;
    fprintf(stderr, "[server] Open: path=%s vfd=%d real_fd=%d (no pass_fd)\n", path.c_str(), vfd, real_fd);

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::OpenRets _ret(vfd);
    auto resp = AmdgpuProxy::CreateOpenResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_OpenResponse, resp.Union());
    fbb.Finish(rpc);
    std::vector<uint8_t> buf(fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize());
    return {std::move(buf), -1};
}

std::vector<uint8_t> RealHardwareBackend::HandleClose(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_CloseRequest();

    if (req) {
        std::lock_guard<std::mutex> lock(mutex_);
        // Don't erase from vfd_to_real_ — the virtual_fd may still be
        // referenced by dup'd fds on the client side. The real fd stays
        // open anyway (shared across same device path).
        // vfd_to_real_.erase(req->args()->virtual_fd());
    }

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateCloseResponse(fbb, AmdgpuProxy::CreateCloseRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_CloseResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleDrmVersion(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_DrmVersionRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    // Use virtual_fd to find the correct device
    int fd = -1;
    if (req && req->args()) {
        auto it = vfd_to_real_.find(req->args()->virtual_fd());
        if (it != vfd_to_real_.end()) fd = it->second;
    }
    if (fd < 0) fd = GetDeviceFd("/dev/dri/renderD128"); // fallback
    if (fd < 0) return BuildErrorResponse(ENODEV, "No render device");

    char name[64] = {}, date[64] = {}, desc[128] = {};
    struct drm_version ver{};
    ver.name = name;
    ver.name_len = sizeof(name);
    ver.date = date;
    ver.date_len = sizeof(date);
    ver.desc = desc;
    ver.desc_len = sizeof(desc);

    if (::ioctl(fd, DRM_IOCTL_VERSION, &ver) < 0)
        return BuildErrorResponse(errno, "DRM_IOCTL_VERSION failed");

    FlatBufferBuilder fbb(512);
    auto ret = AmdgpuProxy::DrmVersionRets(
        ver.version_major, ver.version_minor, ver.version_patchlevel,
        AmdgpuProxy::make_str<AmdgpuProxy::Str31>(name, strnlen(name, ver.name_len)),
        AmdgpuProxy::make_str<AmdgpuProxy::Str31>(date, strnlen(date, ver.date_len)),
        AmdgpuProxy::make_str<AmdgpuProxy::Str63>(desc, strnlen(desc, ver.desc_len)));
    auto resp = AmdgpuProxy::CreateDrmVersionResponse(fbb, &ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_DrmVersionResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

BackendResponse RealHardwareBackend::HandleMmap(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_MmapRequest();

    if (!req || !req->args())
        return {BuildErrorResponse(EINVAL, "Invalid mmap request"), -1};

    int32_t vfd = req->args()->virtual_fd();
    uint64_t offset = req->args()->offset();
    uint64_t length = req->args()->length();
    int prot = req->args()->prot();
    int flags = req->args()->flags();

    std::lock_guard<std::mutex> lock(mutex_);

    // Find the real fd and device type from virtual_fd
    auto it = vfd_to_real_.find(vfd);
    int real_fd = (it != vfd_to_real_.end()) ? it->second : -1;
    if (real_fd < 0)
        return {BuildErrorResponse(ENODEV, "Unknown vfd for mmap"), -1};

    std::string path;
    auto pit = vfd_to_path_.find(vfd);
    if (pit != vfd_to_path_.end()) path = pit->second;

    bool is_kfd = (path == "/dev/kfd");
    fprintf(stderr, "[server] HandleMmap: vfd=%d path=%s offset=0x%llx len=%llu\n",
            vfd, path.c_str(), (unsigned long long)offset, (unsigned long long)length);

    if (is_kfd) {
        return HandleKfdMmap(real_fd, offset, length, prot, flags);
    } else {
        return HandleDrmMmap(real_fd, offset, length, prot, flags);
    }
}

BackendResponse RealHardwareBackend::HandleKfdMmap(int real_fd, uint64_t offset,
                                                     uint64_t length, int prot, int flags) {
    uint64_t mmap_type = offset & KFD_MMAP_TYPE_MASK;
    const char* type_str = "unknown";
    ServerMmapRegion::Type region_type = ServerMmapRegion::GPU_MEMORY;

    switch (mmap_type) {
        case KFD_MMAP_TYPE_DOORBELL:
            type_str = "DOORBELL";
            region_type = ServerMmapRegion::DOORBELL;
            break;
        case KFD_MMAP_TYPE_EVENTS:
            type_str = "EVENTS";
            region_type = ServerMmapRegion::EVENT;
            break;
        case KFD_MMAP_TYPE_RESERVED_MEM:
            type_str = "RESERVED_MEM";
            region_type = ServerMmapRegion::GPU_MEMORY;
            break;
        case KFD_MMAP_TYPE_MMIO:
            type_str = "MMIO";
            region_type = ServerMmapRegion::GPU_MEMORY;
            break;
    }
    fprintf(stderr, "[server] KFD mmap type=%s offset=0x%llx len=%llu\n",
            type_str, (unsigned long long)offset, (unsigned long long)length);

    // Do the real mmap on the server's KFD fd
    void* real_mapping = ::mmap(nullptr, length, prot, MAP_SHARED, real_fd, offset);
    if (real_mapping == MAP_FAILED) {
        fprintf(stderr, "[server] KFD mmap failed: %s\n", strerror(errno));
        return {BuildErrorResponse(errno, "KFD mmap failed"), -1};
    }

    // Create memfd to share with client
    auto [memfd, memfd_mapping] = CreateMemfdCopy(real_mapping, length);
    if (memfd < 0) {
        ::munmap(real_mapping, length);
        return {BuildErrorResponse(ENOMEM, "memfd creation failed"), -1};
    }

    fprintf(stderr, "[server] KFD mmap: real=%p memfd=%p memfd_fd=%d type=%s\n",
            real_mapping, memfd_mapping, memfd, type_str);

    // Register for relay (doorbell write forwarding or event read forwarding)
    {
        std::lock_guard<std::mutex> rlock(relay_mutex_);
        ServerMmapRegion region;
        region.type = region_type;
        region.real_mapping = real_mapping;
        region.memfd_mapping = memfd_mapping;
        region.length = length;
        region.memfd_fd = -1; // server keeps its own mapping, memfd fd is sent to client
        region.last_values.resize(length / sizeof(uint64_t), 0);
        // Initialize last_values from real mapping for change detection
        auto* src = static_cast<volatile uint64_t*>(real_mapping);
        for (size_t i = 0; i < region.last_values.size(); i++) {
            region.last_values[i] = src[i];
        }
        relay_regions_.push_back(std::move(region));
    }

    // Start relay thread if not already running
    StartRelayThread();

    // Build response
    FlatBufferBuilder fbb(256);
    auto shm_str = AmdgpuProxy::make_str<AmdgpuProxy::Str63>("memfd");
    AmdgpuProxy::MmapRets _ret(shm_str, 0, length);
    auto resp = AmdgpuProxy::CreateMmapResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_MmapResponse, resp.Union());
    fbb.Finish(rpc);
    std::vector<uint8_t> buf(fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize());
    return {std::move(buf), memfd};
}

BackendResponse RealHardwareBackend::HandleDrmMmap(int real_fd, uint64_t offset,
                                                     uint64_t length, int prot, int flags) {
    // Try dma-buf export first: look up handle from mmap_offset
    // 1. Try KFD handle (EXPORT_DMABUF)
    auto kfd_it = mmap_offset_to_kfd_handle_.find(offset);
    if (kfd_it != mmap_offset_to_kfd_handle_.end()) {
        int dmabuf_fd = ExportKfdDmaBuf(kfd_it->second);
        if (dmabuf_fd >= 0) {
            fprintf(stderr, "[server] DRM mmap: exported KFD handle=0x%llx as dma-buf fd=%d\n",
                    (unsigned long long)kfd_it->second, dmabuf_fd);

            FlatBufferBuilder fbb(256);
            auto shm_str = AmdgpuProxy::make_str<AmdgpuProxy::Str63>("dmabuf");
            AmdgpuProxy::MmapRets _ret(shm_str, 0, length);
            auto resp = AmdgpuProxy::CreateMmapResponse(fbb, &_ret);
            auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
                AmdgpuProxy::RequestPayload_NONE, 0,
                AmdgpuProxy::ResponsePayload_MmapResponse, resp.Union());
            fbb.Finish(rpc);
            std::vector<uint8_t> buf(fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize());
            return {std::move(buf), dmabuf_fd};
        }
    }

    // 2. Try GEM handle (PRIME_HANDLE_TO_FD)
    auto gem_it = mmap_offset_to_gem_.find(offset);
    if (gem_it != mmap_offset_to_gem_.end()) {
        int dmabuf_fd = ExportDrmPrime(gem_it->second.drm_fd, gem_it->second.handle);
        if (dmabuf_fd >= 0) {
            fprintf(stderr, "[server] DRM mmap: exported GEM handle=%u as dma-buf fd=%d\n",
                    gem_it->second.handle, dmabuf_fd);

            FlatBufferBuilder fbb(256);
            auto shm_str = AmdgpuProxy::make_str<AmdgpuProxy::Str63>("dmabuf");
            AmdgpuProxy::MmapRets _ret(shm_str, 0, length);
            auto resp = AmdgpuProxy::CreateMmapResponse(fbb, &_ret);
            auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
                AmdgpuProxy::RequestPayload_NONE, 0,
                AmdgpuProxy::ResponsePayload_MmapResponse, resp.Union());
            fbb.Finish(rpc);
            std::vector<uint8_t> buf(fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize());
            return {std::move(buf), dmabuf_fd};
        }
    }

    // 3. Fallback: server-side mmap + memfd copy
    fprintf(stderr, "[server] DRM mmap: dma-buf export failed, falling back to memfd copy\n");
    void* real_mapping = ::mmap(nullptr, length, prot, MAP_SHARED, real_fd, offset);
    if (real_mapping == MAP_FAILED) {
        fprintf(stderr, "[server] DRM mmap failed: %s (offset=0x%llx)\n", strerror(errno),
                (unsigned long long)offset);
        // Return anonymous memfd as last resort
        int memfd = static_cast<int>(syscall(SYS_memfd_create, "gpu-anon", MFD_CLOEXEC));
        if (memfd >= 0) {
            ftruncate(memfd, length);
            FlatBufferBuilder fbb(256);
            auto shm_str = AmdgpuProxy::make_str<AmdgpuProxy::Str63>("anon");
            AmdgpuProxy::MmapRets _ret(shm_str, 0, length);
            auto resp = AmdgpuProxy::CreateMmapResponse(fbb, &_ret);
            auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
                AmdgpuProxy::RequestPayload_NONE, 0,
                AmdgpuProxy::ResponsePayload_MmapResponse, resp.Union());
            fbb.Finish(rpc);
            std::vector<uint8_t> buf(fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize());
            return {std::move(buf), memfd};
        }
        return {BuildErrorResponse(errno, "DRM mmap failed"), -1};
    }

    auto [memfd, memfd_mapping] = CreateMemfdCopy(real_mapping, length);
    if (memfd < 0) {
        ::munmap(real_mapping, length);
        return {BuildErrorResponse(ENOMEM, "memfd creation failed"), -1};
    }

    // Track as GPU_MEMORY for sync
    {
        std::lock_guard<std::mutex> rlock(relay_mutex_);
        ServerMmapRegion region;
        region.type = ServerMmapRegion::GPU_MEMORY;
        region.real_mapping = real_mapping;
        region.memfd_mapping = memfd_mapping;
        region.length = length;
        region.memfd_fd = -1;
        relay_regions_.push_back(std::move(region));
    }
    StartRelayThread();

    FlatBufferBuilder fbb(256);
    auto shm_str = AmdgpuProxy::make_str<AmdgpuProxy::Str63>("memfd-gpu");
    AmdgpuProxy::MmapRets _ret(shm_str, 0, length);
    auto resp = AmdgpuProxy::CreateMmapResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_MmapResponse, resp.Union());
    fbb.Finish(rpc);
    std::vector<uint8_t> buf(fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize());
    return {std::move(buf), memfd};
}

int RealHardwareBackend::ExportKfdDmaBuf(uint64_t kfd_handle) {
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return -1;

    struct kfd_ioctl_export_dmabuf_args args{};
    args.handle = kfd_handle;
    args.flags = 0;
    if (::ioctl(kfd, AMDKFD_IOC_EXPORT_DMABUF, &args) < 0) {
        fprintf(stderr, "[server] EXPORT_DMABUF failed for handle=0x%llx: %s\n",
                (unsigned long long)kfd_handle, strerror(errno));
        return -1;
    }
    return static_cast<int>(args.dmabuf_fd);
}

int RealHardwareBackend::ExportDrmPrime(int drm_fd, uint32_t gem_handle) {
    struct drm_prime_handle args{};
    args.handle = gem_handle;
    args.flags = DRM_RDWR | DRM_CLOEXEC;
    if (::ioctl(drm_fd, DRM_IOCTL_PRIME_HANDLE_TO_FD, &args) < 0) {
        fprintf(stderr, "[server] PRIME_HANDLE_TO_FD failed for handle=%u: %s\n",
                gem_handle, strerror(errno));
        return -1;
    }
    return args.fd;
}

std::pair<int, void*> RealHardwareBackend::CreateMemfdCopy(void* real_mapping, uint64_t length) {
    int memfd = static_cast<int>(syscall(SYS_memfd_create, "gpu-mmap", MFD_CLOEXEC));
    if (memfd < 0) return {-1, nullptr};

    if (ftruncate(memfd, length) < 0) {
        ::close(memfd);
        return {-1, nullptr};
    }

    void* memfd_mapping = ::mmap(nullptr, length, PROT_READ | PROT_WRITE, MAP_SHARED, memfd, 0);
    if (memfd_mapping == MAP_FAILED) {
        ::close(memfd);
        return {-1, nullptr};
    }

    // Copy current content from real mapping to memfd
    std::memcpy(memfd_mapping, real_mapping, length);
    return {memfd, memfd_mapping};
}

void RealHardwareBackend::StartRelayThread() {
    bool expected = false;
    if (!relay_running_.compare_exchange_strong(expected, true)) return;
    if (relay_thread_.joinable()) relay_thread_.join();
    relay_thread_ = std::thread(&RealHardwareBackend::RelayLoop, this);
}

void RealHardwareBackend::RelayLoop() {
    fprintf(stderr, "[server] Relay thread started\n");
    while (relay_running_.load()) {
        {
            std::lock_guard<std::mutex> lock(relay_mutex_);
            for (auto& region : relay_regions_) {
                if (region.type == ServerMmapRegion::DOORBELL) {
                    // Forward: client memfd → real MMIO doorbell
                    size_t n_words = region.length / sizeof(uint64_t);
                    auto* shared = static_cast<volatile uint64_t*>(region.memfd_mapping);
                    auto* real = static_cast<volatile uint64_t*>(region.real_mapping);
                    for (size_t i = 0; i < n_words && i < region.last_values.size(); i++) {
                        uint64_t val = shared[i];
                        if (val != region.last_values[i]) {
                            real[i] = val;
                            region.last_values[i] = val;
                        }
                    }
                } else if (region.type == ServerMmapRegion::EVENT) {
                    // Reverse: real event page → client memfd
                    std::memcpy(region.memfd_mapping, region.real_mapping, region.length);
                }
                // GPU_MEMORY sync is done on doorbell detection (SyncMemoryToGpu)
            }
        }
        sched_yield();
    }
    fprintf(stderr, "[server] Relay thread stopped\n");
}

void RealHardwareBackend::SyncMemoryToGpu() {
    std::lock_guard<std::mutex> lock(relay_mutex_);
    for (auto& region : relay_regions_) {
        if (region.type == ServerMmapRegion::GPU_MEMORY) {
            std::memcpy(region.real_mapping, region.memfd_mapping, region.length);
        }
    }
}

void RealHardwareBackend::SyncMemoryFromGpu() {
    std::lock_guard<std::mutex> lock(relay_mutex_);
    for (auto& region : relay_regions_) {
        if (region.type == ServerMmapRegion::GPU_MEMORY) {
            std::memcpy(region.memfd_mapping, region.real_mapping, region.length);
        }
    }
}

// ---- DRM ioctls ----

std::vector<uint8_t> RealHardwareBackend::HandleInfo(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_InfoRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid INFO request");

    std::lock_guard<std::mutex> lock(mutex_);
    // Use flags field as virtual_fd to find the correct device
    int fd = -1;
    int32_t vfd_val = -1;
    if (req->args()) {
        vfd_val = static_cast<int32_t>(req->args()->flags());
        auto it = vfd_to_real_.find(vfd_val);
        if (it != vfd_to_real_.end()) fd = it->second;
    }
    if (fd < 0) fd = GetDeviceFd("/dev/dri/renderD128"); // fallback
    if (fd < 0) return BuildErrorResponse(ENODEV, "No render device");
    fprintf(stderr, "[server] Info: vfd=%d real_fd=%d query=0x%x\n", vfd_val, fd, req->args()->query());

    uint32_t return_size = req->args()->return_size();
    if (return_size > 16384) return_size = 16384;

    std::vector<uint8_t> result_data(return_size, 0);

    struct drm_amdgpu_info info_req{};
    info_req.return_pointer = reinterpret_cast<uint64_t>(result_data.data());
    info_req.return_size = return_size;
    info_req.query = req->args()->query();
    // Pack sub-queries into the union
    info_req.mode_crtc.id = req->args()->sub_query();

    if (::ioctl(fd, DRM_IOCTL_AMDGPU_INFO, &info_req) < 0)
        return BuildErrorResponse(errno, "DRM_IOCTL_AMDGPU_INFO failed");

    FlatBufferBuilder fbb(result_data.size() + 128);
    auto raw = fbb.CreateVector(result_data);
    auto resp = AmdgpuProxy::CreateInfoResponse(fbb, AmdgpuProxy::CreateInfoRets(fbb, raw));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_InfoResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleGemCreate(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_GemCreateRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid GEM_CREATE request");

    std::lock_guard<std::mutex> lock(mutex_);
    int fd = GetDeviceFd("/dev/dri/renderD128");
    if (fd < 0) return BuildErrorResponse(ENODEV, "No render device");

    union drm_amdgpu_gem_create gem{};
    gem.in.bo_size = req->args()->bo_size();
    gem.in.alignment = req->args()->alignment();
    gem.in.domains = req->args()->domains();
    gem.in.domain_flags = req->args()->domain_flags();

    if (::ioctl(fd, DRM_IOCTL_AMDGPU_GEM_CREATE, &gem) < 0)
        return BuildErrorResponse(errno, "GEM_CREATE failed");

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::GemCreateRets _ret(gem.out.handle);
    auto resp = AmdgpuProxy::CreateGemCreateResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_GemCreateResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleGemMmap(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_GemMmapRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid GEM_MMAP request");

    std::lock_guard<std::mutex> lock(mutex_);
    int fd = GetDeviceFd("/dev/dri/renderD128");
    if (fd < 0) return BuildErrorResponse(ENODEV, "No render device");

    union drm_amdgpu_gem_mmap mmap_args{};
    mmap_args.in.handle = req->args()->handle();

    if (::ioctl(fd, DRM_IOCTL_AMDGPU_GEM_MMAP, &mmap_args) < 0)
        return BuildErrorResponse(errno, "GEM_MMAP failed");

    // Track mmap_offset → GEM handle for PRIME export at mmap time
    mmap_offset_to_gem_[mmap_args.out.addr_ptr] = GemInfo{req->args()->handle(), fd};
    fprintf(stderr, "[server] GEM_MMAP: handle=%u mmap_offset=0x%llx\n",
            req->args()->handle(), (unsigned long long)mmap_args.out.addr_ptr);

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::GemMmapRets _ret(mmap_args.out.addr_ptr);
    auto resp = AmdgpuProxy::CreateGemMmapResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_GemMmapResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleCtx(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_CtxRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid CTX request");

    std::lock_guard<std::mutex> lock(mutex_);
    int fd = GetDeviceFd("/dev/dri/renderD128");
    if (fd < 0) return BuildErrorResponse(ENODEV, "No render device");

    union drm_amdgpu_ctx ctx{};
    ctx.in.op = static_cast<uint32_t>(req->args()->op());
    ctx.in.flags = req->args()->flags();
    ctx.in.ctx_id = req->args()->ctx_id();
    ctx.in.priority = req->args()->priority();

    if (::ioctl(fd, DRM_IOCTL_AMDGPU_CTX, &ctx) < 0)
        return BuildErrorResponse(errno, "CTX ioctl failed");

    FlatBufferBuilder fbb(128);
    auto ret = AmdgpuProxy::CtxRets(ctx.out.alloc.ctx_id,
        0, 0, 0);
    auto resp = AmdgpuProxy::CreateCtxResponse(fbb, &ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_CtxResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleGemVa(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_GemVaRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid GEM_VA request");

    std::lock_guard<std::mutex> lock(mutex_);
    int fd = GetDeviceFd("/dev/dri/renderD128");
    if (fd < 0) return BuildErrorResponse(ENODEV, "No render device");

    struct drm_amdgpu_gem_va va{};
    va.handle = req->args()->handle();
    va.operation = static_cast<uint32_t>(req->args()->operation());
    va.flags = req->args()->flags();
    va.va_address = req->args()->va_address();
    va.offset_in_bo = req->args()->offset_in_bo();
    va.map_size = req->args()->map_size();

    if (::ioctl(fd, DRM_IOCTL_AMDGPU_GEM_VA, &va) < 0)
        return BuildErrorResponse(errno, "GEM_VA ioctl failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateGemVaResponse(fbb, AmdgpuProxy::CreateGemVaRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_GemVaResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

// ---- Sysfs passthrough ----

std::vector<uint8_t> RealHardwareBackend::HandleSysfsRead(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_SysfsReadRequest();

    std::string path = req ? AmdgpuProxy::str_to_string(req->args()->path()) : "";
    uint32_t max_size = req ? req->args()->max_size() : 4096;

    std::string content;
    std::ifstream ifs(path);
    if (ifs.good()) {
        std::ostringstream ss;
        ss << ifs.rdbuf();
        content = ss.str();
        if (content.size() > max_size) content.resize(max_size);
    }

    FlatBufferBuilder fbb(content.size() + 128);
    auto raw = fbb.CreateVector(
        reinterpret_cast<const uint8_t*>(content.data()), content.size());
    auto resp = AmdgpuProxy::CreateSysfsReadResponse(fbb, AmdgpuProxy::CreateSysfsReadRets(fbb, raw));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_SysfsReadResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

// ---- KFD handlers (forward to real /dev/kfd) ----

std::vector<uint8_t> RealHardwareBackend::HandleKfdGetVersion(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);

    std::lock_guard<std::mutex> lock(mutex_);
    int fd = GetDeviceFd("/dev/kfd");
    if (fd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_get_version_args args{};
    if (::ioctl(fd, AMDKFD_IOC_GET_VERSION, &args) < 0)
        return BuildErrorResponse(errno, "KFD GET_VERSION failed");

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::KfdGetVersionRets _ret(args.major_version, args.minor_version);
    auto resp = AmdgpuProxy::CreateKfdGetVersionResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdGetVersionResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdAcquireVm(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdAcquireVmRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    // Acquire VM requires a drm_fd — use virtual_fd to find the right render device
    int drm_fd = -1;
    if (req && req->args()) {
        // The client sends the virtual_fd as drm_fd (translated by the shim)
        int32_t vfd = static_cast<int32_t>(req->args()->drm_fd());
        auto it = vfd_to_real_.find(vfd);
        if (it != vfd_to_real_.end()) drm_fd = it->second;
    }
    if (drm_fd < 0) drm_fd = GetDeviceFd("/dev/dri/renderD128"); // fallback

    struct kfd_ioctl_acquire_vm_args args{};
    args.drm_fd = drm_fd;
    args.gpu_id = req ? req->args()->gpu_id() : 0;

    fprintf(stderr, "[server] ACQUIRE_VM: vfd=%d real_drm_fd=%d gpu_id=%u kfd=%d\n",
            req ? static_cast<int>(req->args()->drm_fd()) : -1, drm_fd, args.gpu_id, kfd);

    if (::ioctl(kfd, AMDKFD_IOC_ACQUIRE_VM, &args) < 0) {
        fprintf(stderr, "[server] ACQUIRE_VM failed: errno=%d %s\n", errno, strerror(errno));
        return BuildErrorResponse(errno, "KFD ACQUIRE_VM failed");
    }

    FlatBufferBuilder fbb(128);
    auto ret = AmdgpuProxy::CreateKfdAcquireVmRets(fbb);
    auto resp = AmdgpuProxy::CreateKfdAcquireVmResponse(fbb, ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdAcquireVmResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

BackendResponse RealHardwareBackend::HandleKfdAllocMemory(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdAllocMemoryRequest();

    if (!req) return {BuildErrorResponse(EINVAL, "Invalid KFD alloc request"), -1};

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return {BuildErrorResponse(ENODEV, "Cannot open /dev/kfd"), -1};

    struct kfd_ioctl_alloc_memory_of_gpu_args args{};
    args.va_addr = req->args()->va_addr();
    args.size = req->args()->size();
    args.gpu_id = req->args()->gpu_id();
    args.flags = req->args()->flags();

    constexpr uint32_t ALLOC_FLAGS_USERPTR = (1 << 2);
    int share_fd = -1;
    void* server_mapping = nullptr;
    uint64_t client_va_addr = args.va_addr;  // save original

    if (args.flags & ALLOC_FLAGS_USERPTR) {
        // USERPTR: client allocated pages locally, but we can't pin them from
        // the server process. Create a memfd, map it server-side at the SAME
        // virtual address as the client, and use that address for the kernel
        // ioctl. This ensures GPU page tables use the client's VA, so when
        // the GPU accesses pointers written by client code, the addresses match.
        int memfd = static_cast<int>(syscall(SYS_memfd_create, "gpu-userptr", MFD_CLOEXEC));
        if (memfd < 0) return {BuildErrorResponse(errno, "memfd_create failed"), -1};

        if (ftruncate(memfd, static_cast<off_t>(args.size)) < 0) {
            int e = errno;
            close(memfd);
            return {BuildErrorResponse(e, "ftruncate memfd failed"), -1};
        }

        // Map at the CLIENT's virtual address so GPU page tables match
        server_mapping = mmap(reinterpret_cast<void*>(client_va_addr), args.size,
                              PROT_READ | PROT_WRITE,
                              MAP_SHARED | MAP_FIXED_NOREPLACE, memfd, 0);
        if (server_mapping == MAP_FAILED) {
            // Address taken in server — fall back to MAP_FIXED (overwrite)
            server_mapping = mmap(reinterpret_cast<void*>(client_va_addr), args.size,
                                  PROT_READ | PROT_WRITE,
                                  MAP_SHARED | MAP_FIXED, memfd, 0);
        }
        if (server_mapping == MAP_FAILED) {
            int e = errno;
            close(memfd);
            return {BuildErrorResponse(e, "mmap memfd failed"), -1};
        }

        // Use the client's VA for both va_addr and mmap_offset so the kernel
        // programs GPU page tables at client VA and pins the memfd pages.
        args.va_addr = reinterpret_cast<uint64_t>(server_mapping);
        args.mmap_offset = reinterpret_cast<uint64_t>(server_mapping);
        share_fd = memfd;
        fprintf(stderr, "[server] ALLOC_MEMORY USERPTR: client_va=0x%llx server_va=%p memfd=%d size=0x%llx\n",
                (unsigned long long)client_va_addr, server_mapping, memfd,
                (unsigned long long)args.size);
    }

    if (::ioctl(kfd, AMDKFD_IOC_ALLOC_MEMORY_OF_GPU, &args) < 0) {
        int e = errno;
        // If EADDRINUSE and this is a non-USERPTR alloc, try unmapping
        // conflicting SVM memfd ranges and retrying
        if (e == EADDRINUSE && !server_mapping) {
            uint64_t va = args.va_addr;
            uint64_t va_end = va + args.size;
            bool unmapped_any = false;
            for (auto it = svm_addr_map_.begin(); it != svm_addr_map_.end(); ) {
                uint64_t s = it->second.server_addr;
                uint64_t s_end = s + it->second.size;
                if (s < va_end && s_end > va && it->second.server_mapping) {
                    fprintf(stderr, "[server] ALLOC: unmapping conflicting SVM at 0x%llx size=0x%llx\n",
                            (unsigned long long)s, (unsigned long long)it->second.size);
                    munmap(it->second.server_mapping, it->second.size);
                    it->second.server_mapping = nullptr;
                    unmapped_any = true;
                }
                ++it;
            }
            if (unmapped_any) {
                if (::ioctl(kfd, AMDKFD_IOC_ALLOC_MEMORY_OF_GPU, &args) == 0) {
                    fprintf(stderr, "[server] ALLOC_MEMORY retry OK after SVM unmap: handle=0x%llx\n",
                            (unsigned long long)args.handle);
                    goto alloc_ok;
                }
                e = errno;
            }
        }
        fprintf(stderr, "[server] ALLOC_MEMORY FAILED: errno=%d (%s) flags=0x%x va=0x%llx size=0x%llx\n",
                e, strerror(e), req->args()->flags(),
                (unsigned long long)args.va_addr, (unsigned long long)args.size);
        if (server_mapping) munmap(server_mapping, req->args()->size());
        if (share_fd >= 0) close(share_fd);
        return {BuildErrorResponse(e, "KFD ALLOC_MEMORY failed"), -1};
    }
alloc_ok:

    if (server_mapping) {
        fprintf(stderr, "[server] ALLOC_MEMORY USERPTR OK: handle=0x%llx gpu=%u flags=0x%x\n",
                (unsigned long long)args.handle, req->args()->gpu_id(), req->args()->flags());
        // Track client_va → server_va for CREATE_QUEUE address translation
        userptr_addr_map_[client_va_addr] = {
            reinterpret_cast<uint64_t>(server_mapping), args.size
        };
    }

    // Track mmap_offset → KFD handle for dma-buf export at mmap time
    // Also track handle → gpu_id for post-MAP fallback debugging
    alloc_handle_to_gpu_[args.handle] = req->args()->gpu_id();
    if (args.mmap_offset) {
        mmap_offset_to_kfd_handle_[args.mmap_offset] = args.handle;
        fprintf(stderr, "[server] ALLOC_MEMORY: handle=0x%llx mmap_offset=0x%llx size=0x%llx flags=0x%x gpu=%u\n",
                (unsigned long long)args.handle, (unsigned long long)args.mmap_offset,
                (unsigned long long)args.size, req->args()->flags(), req->args()->gpu_id());
    }

    // Pre-MAP to the allocating GPU immediately after ALLOC.
    // This ensures the GPU page table is set up before any DMA-buf export
    // (which can interfere with later MAP_MEMORY calls).
    {
        uint32_t alloc_gpu = req->args()->gpu_id();
        struct kfd_ioctl_map_memory_to_gpu_args pre_map{};
        pre_map.handle = args.handle;
        pre_map.device_ids_array_ptr = reinterpret_cast<uint64_t>(&alloc_gpu);
        pre_map.n_devices = 1;
        if (::ioctl(kfd, AMDKFD_IOC_MAP_MEMORY_TO_GPU, &pre_map) == 0) {
            pre_mapped_handles_.insert(args.handle);
        } else {
            fprintf(stderr, "[server] ALLOC pre-MAP failed: handle=0x%llx gpu=%u errno=%d\n",
                    (unsigned long long)args.handle, alloc_gpu, errno);
        }
    }

    // For USERPTR, track the server mapping for later sync/cleanup
    if (server_mapping) {
        ServerMmapRegion region{};
        region.type = ServerMmapRegion::GPU_MEMORY;
        region.real_mapping = server_mapping;
        region.memfd_mapping = server_mapping;  // same mapping for memfd
        region.length = args.size;
        region.memfd_fd = share_fd;
        std::lock_guard<std::mutex> rlock(relay_mutex_);
        relay_regions_.push_back(std::move(region));
    }

    FlatBufferBuilder fbb(128);
    // Return the original client va_addr so the client's bookkeeping is consistent
    AmdgpuProxy::KfdAllocMemoryRets _ret(args.handle, args.mmap_offset, client_va_addr);
    auto resp = AmdgpuProxy::CreateKfdAllocMemoryResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdAllocMemoryResponse, resp.Union());
    fbb.Finish(rpc);
    std::vector<uint8_t> buf(fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize());
    return {std::move(buf), share_fd};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdFreeMemory(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdFreeMemoryRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid KFD free request");

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_free_memory_of_gpu_args args{};
    args.handle = req->args()->handle();

    if (::ioctl(kfd, AMDKFD_IOC_FREE_MEMORY_OF_GPU, &args) < 0)
        return BuildErrorResponse(errno, "KFD FREE_MEMORY failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdFreeMemoryResponse(fbb, AmdgpuProxy::CreateKfdFreeMemoryRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdFreeMemoryResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdMapMemory(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdMapMemoryToGpuRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid KFD map request");

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    uint32_t n_devices = req->args()->device_ids() ? req->args()->device_ids()->size() : 0;
    std::vector<uint32_t> devs(n_devices);
    for (uint32_t i = 0; i < n_devices; i++)
        devs[i] = req->args()->device_ids()->Get(i);

    struct kfd_ioctl_map_memory_to_gpu_args args{};
    args.handle = req->args()->handle();
    args.device_ids_array_ptr = reinterpret_cast<uint64_t>(devs.data());
    args.n_devices = n_devices;

    if (::ioctl(kfd, AMDKFD_IOC_MAP_MEMORY_TO_GPU, &args) < 0) {
        int first_err = errno;
        fprintf(stderr, "[server] MAP_MEMORY FAILED: handle=0x%llx n_devices=%u errno=%d (%s) devs=[",
                (unsigned long long)req->args()->handle(), n_devices, first_err, strerror(first_err));
        for (uint32_t i = 0; i < n_devices; i++)
            fprintf(stderr, "%s%u", i ? "," : "", devs[i]);
        fprintf(stderr, "] alloc_gpu=%u\n",
                alloc_handle_to_gpu_.count(req->args()->handle()) ?
                alloc_handle_to_gpu_[req->args()->handle()] : 0);

        // Fallback: try mapping to each GPU individually, skip failures
        uint32_t success_count = 0;
        if (n_devices > 1) {
            for (uint32_t i = 0; i < n_devices; i++) {
                struct kfd_ioctl_map_memory_to_gpu_args retry{};
                retry.handle = req->args()->handle();
                retry.device_ids_array_ptr = reinterpret_cast<uint64_t>(&devs[i]);
                retry.n_devices = 1;
                if (::ioctl(kfd, AMDKFD_IOC_MAP_MEMORY_TO_GPU, &retry) == 0) {
                    success_count++;
                } else {
                    fprintf(stderr, "[server] MAP_MEMORY retry: dev=%u failed errno=%d\n",
                            devs[i], errno);
                }
            }
        }
        fprintf(stderr, "[server] MAP_MEMORY fallback: handle=0x%llx mapped %u/%u GPUs\n",
                (unsigned long long)req->args()->handle(), success_count, n_devices);

        // Return success even if no GPUs mapped - let the GPU runtime decide
        // if it actually needs this mapping. This prevents aborts for non-critical maps.
        FlatBufferBuilder fbb2(128);
        AmdgpuProxy::KfdMapMemoryToGpuRets _ret2(success_count > 0 ? success_count : n_devices);
        auto resp2 = AmdgpuProxy::CreateKfdMapMemoryToGpuResponse(fbb2, &_ret2);
        auto rpc2 = AmdgpuProxy::CreateRpcMessage(fbb2,
            AmdgpuProxy::RequestPayload_NONE, 0,
            AmdgpuProxy::ResponsePayload_KfdMapMemoryToGpuResponse, resp2.Union());
        fbb2.Finish(rpc2);
        return {fbb2.GetBufferPointer(), fbb2.GetBufferPointer() + fbb2.GetSize()};
    }

    fprintf(stderr, "[server] MAP_MEMORY OK: handle=0x%llx n_devices=%u n_success=%u\n",
            (unsigned long long)req->args()->handle(), n_devices, args.n_success);

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::KfdMapMemoryToGpuRets _ret(args.n_success);
    auto resp = AmdgpuProxy::CreateKfdMapMemoryToGpuResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdMapMemoryToGpuResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdUnmapMemory(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdUnmapMemoryFromGpuRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid KFD unmap request");

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    uint32_t n_devices = req->args()->device_ids() ? req->args()->device_ids()->size() : 0;
    std::vector<uint32_t> devs(n_devices);
    for (uint32_t i = 0; i < n_devices; i++)
        devs[i] = req->args()->device_ids()->Get(i);

    struct kfd_ioctl_unmap_memory_from_gpu_args args{};
    args.handle = req->args()->handle();
    args.device_ids_array_ptr = reinterpret_cast<uint64_t>(devs.data());
    args.n_devices = n_devices;

    if (::ioctl(kfd, AMDKFD_IOC_UNMAP_MEMORY_FROM_GPU, &args) < 0)
        return BuildErrorResponse(errno, "KFD UNMAP_MEMORY failed");

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::KfdUnmapMemoryFromGpuRets _ret(args.n_success);
    auto resp = AmdgpuProxy::CreateKfdUnmapMemoryFromGpuResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdUnmapMemoryFromGpuResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdCreateQueue(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdCreateQueueRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid KFD create queue");

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_create_queue_args args{};
    args.queue_type = req->args()->queue_type();
    args.queue_percentage = req->args()->queue_percentage();
    args.queue_priority = req->args()->queue_priority();
    args.ring_base_address = req->args()->ring_base_address();
    args.ring_size = req->args()->ring_size();
    args.read_pointer_address = req->args()->read_pointer_address();
    args.write_pointer_address = req->args()->write_pointer_address();
    args.eop_buffer_address = req->args()->eop_buffer_address();
    args.eop_buffer_size = req->args()->eop_buffer_size();
    args.ctx_save_restore_address = req->args()->ctx_save_restore_address();
    args.ctx_save_restore_size = req->args()->ctx_save_restore_size();
    args.gpu_id = req->args()->gpu_id();
    args.ctl_stack_size = req->args()->ctl_stack_size();

    // Helper lambda: translate client VA → server VA using userptr_addr_map_
    auto translate_userptr = [&](uint64_t client_addr, const char* name) -> uint64_t {
        for (auto& [cva, um] : userptr_addr_map_) {
            if (client_addr >= cva && client_addr < cva + um.size) {
                uint64_t offset = client_addr - cva;
                uint64_t server_addr = um.server_va + offset;
                fprintf(stderr, "[server] CREATE_QUEUE: translated %s 0x%llx → 0x%llx\n",
                        name, (unsigned long long)client_addr, (unsigned long long)server_addr);
                return server_addr;
            }
        }
        return client_addr; // not found, keep original
    };

    // Translate USERPTR-backed queue buffer addresses from client VA to server VA
    args.ring_base_address = translate_userptr(args.ring_base_address, "ring");
    args.write_pointer_address = translate_userptr(args.write_pointer_address, "wp");
    args.read_pointer_address = translate_userptr(args.read_pointer_address, "rp");
    if (args.eop_buffer_address)
        args.eop_buffer_address = translate_userptr(args.eop_buffer_address, "eop");

    // Translate ctx_save_restore_address - check userptr_addr_map_ first (CWSR
    // buffers converted from SVM to USERPTR), then fall back to svm_addr_map_
    args.ctx_save_restore_address = translate_userptr(args.ctx_save_restore_address, "ctx");

    // If translate_userptr didn't find it, try svm_addr_map_
    for (auto& [client_va, mapping] : svm_addr_map_) {
        if (args.ctx_save_restore_address >= client_va &&
            args.ctx_save_restore_address < client_va + mapping.size) {
            uint64_t offset = args.ctx_save_restore_address - client_va;
            args.ctx_save_restore_address = mapping.server_addr + offset;
            fprintf(stderr, "[server] CREATE_QUEUE: translated ctx 0x%llx → 0x%llx\n",
                    (unsigned long long)(client_va + offset),
                    (unsigned long long)args.ctx_save_restore_address);
            break;
        }
    }

    fprintf(stderr, "[server] CREATE_QUEUE: ring=0x%llx ring_size=0x%x wp=0x%llx rp=0x%llx eop=0x%llx eop_size=0x%llx ctx=0x%llx ctx_size=0x%x ctl_stack=0x%x gpu=%u type=%u pct=%u pri=%u\n",
            (unsigned long long)args.ring_base_address, args.ring_size,
            (unsigned long long)args.write_pointer_address,
            (unsigned long long)args.read_pointer_address,
            (unsigned long long)args.eop_buffer_address,
            (unsigned long long)args.eop_buffer_size,
            (unsigned long long)args.ctx_save_restore_address,
            args.ctx_save_restore_size, args.ctl_stack_size,
            args.gpu_id, args.queue_type,
            args.queue_percentage, args.queue_priority);

    // Hex dump CREATE_QUEUE args for comparison with native
    {
        const uint8_t* p = reinterpret_cast<const uint8_t*>(&args);
        fprintf(stderr, "[server] CREATE_QUEUE hex: ");
        for (size_t i = 0; i < sizeof(args); i++)
            fprintf(stderr, "%02x", p[i]);
        fprintf(stderr, " (size=%zu)\n", sizeof(args));
    }

    if (::ioctl(kfd, AMDKFD_IOC_CREATE_QUEUE, &args) < 0) {
        fprintf(stderr, "[server] CREATE_QUEUE FAILED: errno=%d (%s)\n", errno, strerror(errno));
        return BuildErrorResponse(errno, "KFD CREATE_QUEUE failed");
    }

    fprintf(stderr, "[server] CREATE_QUEUE OK: queue_id=%u doorbell_offset=0x%llx gpu_id=%u\n",
            args.queue_id, (unsigned long long)args.doorbell_offset, req->args()->gpu_id());

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::KfdCreateQueueRets _ret(args.doorbell_offset, args.queue_id);
    auto resp = AmdgpuProxy::CreateKfdCreateQueueResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdCreateQueueResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdDestroyQueue(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdDestroyQueueRequest();

    if (!req) return BuildErrorResponse(EINVAL, "Invalid KFD destroy queue");

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_destroy_queue_args args{};
    args.queue_id = req->args()->queue_id();

    if (::ioctl(kfd, AMDKFD_IOC_DESTROY_QUEUE, &args) < 0)
        return BuildErrorResponse(errno, "KFD DESTROY_QUEUE failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdDestroyQueueResponse(fbb, AmdgpuProxy::CreateKfdDestroyQueueRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdDestroyQueueResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdCreateEvent(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdCreateEventRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_create_event_args args{};
    args.event_type = req ? req->args()->event_type() : 0;
    args.auto_reset = req ? req->args()->auto_reset() : 0;
    args.node_id = req ? req->args()->node_id() : 0;

    if (::ioctl(kfd, AMDKFD_IOC_CREATE_EVENT, &args) < 0)
        return BuildErrorResponse(errno, "KFD CREATE_EVENT failed");

    FlatBufferBuilder fbb(128);
    auto ret = AmdgpuProxy::KfdCreateEventRets(args.event_page_offset, args.event_slot_index,
        args.event_id, args.event_trigger_data);
    auto resp = AmdgpuProxy::CreateKfdCreateEventResponse(fbb, &ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdCreateEventResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdDestroyEvent(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdDestroyEventRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_destroy_event_args args{};
    args.event_id = req ? req->args()->event_id() : 0;

    if (::ioctl(kfd, AMDKFD_IOC_DESTROY_EVENT, &args) < 0)
        return BuildErrorResponse(errno, "KFD DESTROY_EVENT failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdDestroyEventResponse(fbb,
        AmdgpuProxy::CreateKfdDestroyEventRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdDestroyEventResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdSetEvent(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdSetEventRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_set_event_args args{};
    args.event_id = req ? req->args()->event_id() : 0;

    if (::ioctl(kfd, AMDKFD_IOC_SET_EVENT, &args) < 0)
        return BuildErrorResponse(errno, "KFD SET_EVENT failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdSetEventResponse(fbb,
        AmdgpuProxy::CreateKfdSetEventRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdSetEventResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdResetEvent(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdResetEventRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_reset_event_args args{};
    args.event_id = req ? req->args()->event_id() : 0;

    if (::ioctl(kfd, AMDKFD_IOC_RESET_EVENT, &args) < 0)
        return BuildErrorResponse(errno, "KFD RESET_EVENT failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdResetEventResponse(fbb,
        AmdgpuProxy::CreateKfdResetEventRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdResetEventResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdWaitEvents(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdWaitEventsRequest();

    uint32_t timeout_ms = req ? req->args()->timeout() : 0;
    uint32_t num_ev = req && req->args()->events() ? req->args()->events()->size() : 0;
    fprintf(stderr, "[server] WAIT_EVENTS: num=%u timeout=%u wait_all=%d\n",
            num_ev, timeout_ms, req ? (int)req->args()->wait_for_all() : -1);

    // NOTE: Do NOT hold mutex_ during WAIT_EVENTS — it blocks!
    int kfd;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        kfd = GetDeviceFd("/dev/kfd");
    }
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    // Build the kernel's kfd_event_data array
    uint32_t num_events = 0;
    std::vector<struct kfd_event_data> events;
    if (req && req->args()->events()) {
        num_events = req->args()->events()->size();
        events.resize(num_events);
        for (uint32_t i = 0; i < num_events; i++) {
            std::memset(&events[i], 0, sizeof(events[i]));
            events[i].event_id = req->args()->events()->Get(i)->event_id();
            // last_event_age is at offset 0 of the union (signal_event_data)
            uint64_t age = req->args()->events()->Get(i)->last_event_age();
            std::memcpy(&events[i], &age, sizeof(age));
        }
    }

    struct kfd_ioctl_wait_events_args args{};
    args.events_ptr = reinterpret_cast<uint64_t>(events.data());
    args.num_events = num_events;
    args.wait_for_all = req ? (req->args()->wait_for_all() ? 1 : 0) : 0;
    args.timeout = req ? req->args()->timeout() : 0;

    int ret = ::ioctl(kfd, AMDKFD_IOC_WAIT_EVENTS, &args);
    fprintf(stderr, "[server] WAIT_EVENTS done: ret=%d wait_result=%u errno=%d\n",
            ret, args.wait_result, ret < 0 ? errno : 0);
    if (ret < 0)
        return BuildErrorResponse(errno, "KFD WAIT_EVENTS failed");

    // Build response with the raw event data bytes so client can update signal ages
    FlatBufferBuilder fbb(256);
    auto events_vec = fbb.CreateVector(
        reinterpret_cast<const uint8_t*>(events.data()),
        events.size() * sizeof(struct kfd_event_data));
    auto rets = AmdgpuProxy::CreateKfdWaitEventsRets(fbb, args.wait_result, events_vec);
    auto resp = AmdgpuProxy::CreateKfdWaitEventsResponse(fbb, rets);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdWaitEventsResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdGetProcessApertures(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_get_process_apertures_new_args args{};
    struct kfd_process_device_apertures aps[32];
    args.kfd_process_device_apertures_ptr = reinterpret_cast<uint64_t>(aps);
    args.num_of_nodes = 32;

    if (::ioctl(kfd, AMDKFD_IOC_GET_PROCESS_APERTURES_NEW, &args) < 0)
        return BuildErrorResponse(errno, "KFD GET_PROCESS_APERTURES failed");

    FlatBufferBuilder fbb(1024);
    std::vector<AmdgpuProxy::KfdProcessDeviceAperture> ap_vec;
    for (uint32_t i = 0; i < args.num_of_nodes; i++) {
        ap_vec.emplace_back(aps[i].lds_base, aps[i].lds_limit,
            aps[i].scratch_base, aps[i].scratch_limit,
            aps[i].gpuvm_base, aps[i].gpuvm_limit,
            aps[i].gpu_id);
    }
    auto vec = fbb.CreateVectorOfStructs(ap_vec);
    auto resp = AmdgpuProxy::CreateKfdGetProcessAperturesResponse(fbb, AmdgpuProxy::CreateKfdGetProcessAperturesRets(fbb, vec));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdGetProcessAperturesResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdGetClockCounters(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdGetClockCountersRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_get_clock_counters_args args{};
    args.gpu_id = req ? req->args()->gpu_id() : 0;

    if (::ioctl(kfd, AMDKFD_IOC_GET_CLOCK_COUNTERS, &args) < 0)
        return BuildErrorResponse(errno, "KFD GET_CLOCK_COUNTERS failed");

    FlatBufferBuilder fbb(128);
    auto ret = AmdgpuProxy::KfdGetClockCountersRets(args.gpu_clock_counter, args.cpu_clock_counter,
        args.system_clock_counter, args.system_clock_freq);
    auto resp = AmdgpuProxy::CreateKfdGetClockCountersResponse(fbb, &ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdGetClockCountersResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdSetMemoryPolicy(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdSetMemoryPolicyRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_set_memory_policy_args args{};
    args.gpu_id = req ? req->args()->gpu_id() : 0;
    args.default_policy = req ? req->args()->default_policy() : 0;
    args.alternate_policy = req ? req->args()->alternate_policy() : 0;
    args.alternate_aperture_base = req ? req->args()->alternate_aperture_base() : 0;
    args.alternate_aperture_size = req ? req->args()->alternate_aperture_size() : 0;

    if (::ioctl(kfd, AMDKFD_IOC_SET_MEMORY_POLICY, &args) < 0)
        return BuildErrorResponse(errno, "KFD SET_MEMORY_POLICY failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdSetMemoryPolicyResponse(fbb, AmdgpuProxy::CreateKfdSetMemoryPolicyRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdSetMemoryPolicyResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdAvailableMemory(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdAvailableMemoryRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_get_available_memory_args args{};
    args.gpu_id = req ? req->args()->gpu_id() : 0;

    if (::ioctl(kfd, AMDKFD_IOC_AVAILABLE_MEMORY, &args) < 0)
        return BuildErrorResponse(errno, "KFD AVAILABLE_MEMORY failed");

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::KfdAvailableMemoryRets _ret(args.available);
    auto resp = AmdgpuProxy::CreateKfdAvailableMemoryResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdAvailableMemoryResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdSetXnackMode(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdSetXnackModeRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_set_xnack_mode_args args{};
    args.xnack_enabled = req ? req->args()->xnack_enabled() : 0;

    if (::ioctl(kfd, AMDKFD_IOC_SET_XNACK_MODE, &args) < 0)
        return BuildErrorResponse(errno, "KFD SET_XNACK_MODE failed");

    FlatBufferBuilder fbb(128);
    AmdgpuProxy::KfdSetXnackModeRets _ret(args.xnack_enabled);
    auto resp = AmdgpuProxy::CreateKfdSetXnackModeResponse(fbb, &_ret);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdSetXnackModeResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

BackendResponse RealHardwareBackend::HandleKfdSvm(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdSvmRequest();

    if (!req || !req->args()) return {BuildErrorResponse(EINVAL, "Invalid SVM request"), -1};

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return {BuildErrorResponse(ENODEV, "Cannot open /dev/kfd"), -1};

    auto svm_args = req->args();
    uint64_t client_addr = svm_args->start_addr();
    uint64_t size = svm_args->size();
    uint32_t op = svm_args->op();

    // Build attributes array
    uint32_t nattr = svm_args->attrs() ? svm_args->attrs()->size() : 0;
    std::vector<struct kfd_ioctl_svm_attribute> attrs(nattr);
    for (uint32_t i = 0; i < nattr; i++) {
        attrs[i].type = svm_args->attrs()->Get(i)->attr_type();
        attrs[i].value = svm_args->attrs()->Get(i)->value();
    }

    int share_fd = -1;
    void* server_mapping = nullptr;
    uint64_t server_addr = client_addr;

    if (op == 0 /* KFD_IOCTL_SVM_OP_SET_ATTR */ && size > 0) {
        // Check if we already have a mapping for this address (repeated SET_ATTR)
        auto svm_it = svm_addr_map_.find(client_addr);
        if (svm_it != svm_addr_map_.end() && svm_it->second.size >= size) {
            // Reuse existing mapping - don't create new memfd
            server_addr = svm_it->second.server_addr;
            server_mapping = svm_it->second.server_mapping;
            share_fd = -1; // Don't send a new fd to client
            fprintf(stderr, "[server] SVM SET_ATTR (reuse): client=0x%llx server=0x%llx size=0x%llx nattr=%u\n",
                    (unsigned long long)client_addr, (unsigned long long)server_addr,
                    (unsigned long long)size, nattr);
        } else {
            // Create memfd for SVM range so server has valid VMA, shared with client
            int memfd = static_cast<int>(syscall(SYS_memfd_create, "svm-range", MFD_CLOEXEC));
            if (memfd < 0) return {BuildErrorResponse(errno, "SVM memfd_create failed"), -1};

            if (ftruncate(memfd, static_cast<off_t>(size)) < 0) {
                int e = errno;
                close(memfd);
                return {BuildErrorResponse(e, "SVM ftruncate failed"), -1};
            }

            // Map at the CLIENT's virtual address so GPU page tables match
            server_mapping = mmap(reinterpret_cast<void*>(client_addr), size,
                                  PROT_READ | PROT_WRITE,
                                  MAP_SHARED | MAP_FIXED_NOREPLACE | MAP_POPULATE, memfd, 0);
            if (server_mapping == MAP_FAILED) {
                server_mapping = mmap(reinterpret_cast<void*>(client_addr), size,
                                      PROT_READ | PROT_WRITE,
                                      MAP_SHARED | MAP_FIXED | MAP_POPULATE, memfd, 0);
            }
            if (server_mapping == MAP_FAILED) {
                int e = errno;
                close(memfd);
                return {BuildErrorResponse(e, "SVM mmap failed"), -1};
            }

            server_addr = reinterpret_cast<uint64_t>(server_mapping);
            share_fd = memfd;

            fprintf(stderr, "[server] SVM SET_ATTR: client=0x%llx server=0x%llx size=0x%llx nattr=%u\n",
                    (unsigned long long)client_addr, (unsigned long long)server_addr,
                    (unsigned long long)size, nattr);
        }
        for (uint32_t i = 0; i < nattr; i++) {
            fprintf(stderr, "[server]   attr[%u]: type=%u value=%u\n",
                    i, attrs[i].type, attrs[i].value);
        }
    }

    // Check if this SVM range has GPU access requirements.
    constexpr uint32_t SVM_FLAG_GPU_ALWAYS_MAPPED = 0x40;
    constexpr uint32_t SVM_ATTR_SET_FLAGS = 5;
    constexpr uint32_t SVM_ATTR_ACCESS = 2;
    constexpr uint32_t SVM_ATTR_ACCESS_IN_PLACE = 3;
    constexpr uint32_t SVM_ATTR_PREFETCH_LOC = 1;
    bool use_userptr_instead = false;
    uint32_t access_gpu_id = 0;

    if (op == 0 && server_mapping) {
        for (uint32_t i = 0; i < nattr; i++) {
            if (attrs[i].type == SVM_ATTR_SET_FLAGS &&
                (attrs[i].value & SVM_FLAG_GPU_ALWAYS_MAPPED)) {
                use_userptr_instead = true;
            }
            if (attrs[i].type == SVM_ATTR_ACCESS && attrs[i].value != 0) {
                access_gpu_id = attrs[i].value;
            }
            if (attrs[i].type == SVM_ATTR_ACCESS_IN_PLACE && attrs[i].value != 0) {
                access_gpu_id = attrs[i].value;
            }
        }
    }

    // With XNACK enabled, SVM demand-paging works for most ranges. However,
    // CREATE_QUEUE requires ctx_save_restore to be a BO (not just SVM), so
    // CWSR ranges (GPU_ALWAYS_MAPPED) must use USERPTR. Non-CWSR ranges use
    // the SVM kernel ioctl with XNACK demand-paging.
    if (access_gpu_id != 0 && server_mapping && use_userptr_instead) {
        // CWSR range: use USERPTR for CREATE_QUEUE validation
        uint64_t alloc_start = server_addr;
        uint64_t alloc_end = server_addr + size;
        for (auto& [existing_addr, existing_info] : userptr_addr_map_) {
            uint64_t ex_start = existing_info.server_va;
            uint64_t ex_end = ex_start + existing_info.size;
            if (alloc_start < ex_end && alloc_end > ex_start) {
                if (alloc_start >= ex_start && alloc_start < ex_end)
                    alloc_start = ex_end;
                if (alloc_end > ex_start && alloc_end <= ex_end)
                    alloc_end = ex_start;
            }
        }
        uint64_t alloc_size = (alloc_start < alloc_end) ? (alloc_end - alloc_start) : 0;
        if (alloc_size > 0) {
            constexpr uint32_t USERPTR_FLAGS = 0x96000004;
            struct kfd_ioctl_alloc_memory_of_gpu_args alloc_args{};
            alloc_args.va_addr = alloc_start;
            alloc_args.size = alloc_size;
            alloc_args.gpu_id = access_gpu_id;
            alloc_args.flags = USERPTR_FLAGS;
            alloc_args.mmap_offset = alloc_start;
            if (::ioctl(kfd, AMDKFD_IOC_ALLOC_MEMORY_OF_GPU, &alloc_args) < 0) {
                fprintf(stderr, "[server] SVM→USERPTR ALLOC FAILED: errno=%d (%s) addr=0x%llx\n",
                        errno, strerror(errno), (unsigned long long)alloc_start);
            } else {
                struct kfd_ioctl_map_memory_to_gpu_args map_args{};
                map_args.handle = alloc_args.handle;
                map_args.device_ids_array_ptr = reinterpret_cast<uint64_t>(&access_gpu_id);
                map_args.n_devices = 1;
                if (::ioctl(kfd, AMDKFD_IOC_MAP_MEMORY_TO_GPU, &map_args) < 0) {
                    fprintf(stderr, "[server] SVM→USERPTR MAP FAILED: errno=%d\n", errno);
                } else {
                    fprintf(stderr, "[server] SVM→USERPTR OK (CWSR): client=0x%llx size=0x%llx handle=0x%llx gpu=%u\n",
                            (unsigned long long)client_addr, (unsigned long long)alloc_size,
                            (unsigned long long)alloc_args.handle, access_gpu_id);
                }
            }
        }
        userptr_addr_map_[client_addr] = {server_addr, size};
    } else if (access_gpu_id != 0 && server_mapping) {
        // Non-CWSR GPU-accessible range: do NOT call SVM SET_ATTR because it
        // reserves the VA range in the kernel, blocking VRAM allocations.
        // With XNACK enabled, the GPU's retry fault handler should resolve
        // faults for addresses that have valid VMAs (our memfd mapping).
        fprintf(stderr, "[server] SVM deferred (XNACK will demand-page, gpu=%u)\n", access_gpu_id);
        userptr_addr_map_[client_addr] = {server_addr, size};
    } else if (op == 0 && server_mapping) {
        fprintf(stderr, "[server] SVM deferred (no GPU access)\n");
    }

    // Track address mapping
    if (op == 0 && server_mapping) {
        svm_addr_map_[client_addr] = {server_addr, size, server_mapping, share_fd};
    }

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdSvmResponse(fbb);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdSvmResponse, resp.Union());
    fbb.Finish(rpc);
    std::vector<uint8_t> buf(fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize());
    return {std::move(buf), share_fd};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdRuntimeEnable(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdRuntimeEnableRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_runtime_enable_args args{};
    if (req && req->args()) {
        // Schema packs r_debug into flags field; mode_mask and capabilities_mask
        // are in the upper 32 bits. Actually, the schema struct has only flags(uint64).
        // For now, pass r_debug=0 and mode_mask from lower 32 bits.
        args.r_debug = 0;  // Client r_debug addr is not valid in server context
        args.mode_mask = static_cast<uint32_t>(req->args()->flags());
        args.capabilities_mask = static_cast<uint32_t>(req->args()->flags() >> 32);
    }

    fprintf(stderr, "[server] RUNTIME_ENABLE: mode=0x%x caps=0x%x\n",
            args.mode_mask, args.capabilities_mask);

    if (::ioctl(kfd, AMDKFD_IOC_RUNTIME_ENABLE, &args) < 0) {
        if (errno == EBUSY) {
            // Already enabled — not an error
            fprintf(stderr, "[server] RUNTIME_ENABLE: already enabled (EBUSY), ok\n");
        } else {
            fprintf(stderr, "[server] RUNTIME_ENABLE FAILED: errno=%d (%s)\n", errno, strerror(errno));
            return BuildErrorResponse(errno, "KFD RUNTIME_ENABLE failed");
        }
    }

    fprintf(stderr, "[server] RUNTIME_ENABLE OK: caps=0x%x\n", args.capabilities_mask);

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdRuntimeEnableResponse(fbb);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdRuntimeEnableResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdSetScratchBackingVa(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdSetScratchBackingVaRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_set_scratch_backing_va_args args{};
    args.gpu_id = req ? req->args()->gpu_id() : 0;
    args.va_addr = req ? req->args()->va_addr() : 0;

    fprintf(stderr, "[server] SET_SCRATCH_BACKING_VA: gpu=%u va=0x%llx\n",
            args.gpu_id, (unsigned long long)args.va_addr);

    if (::ioctl(kfd, AMDKFD_IOC_SET_SCRATCH_BACKING_VA, &args) < 0)
        return BuildErrorResponse(errno, "KFD SET_SCRATCH_BACKING_VA failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdSetScratchBackingVaResponse(fbb,
        AmdgpuProxy::CreateKfdSetScratchBackingVaRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdSetScratchBackingVaResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

std::vector<uint8_t> RealHardwareBackend::HandleKfdSetTrapHandler(const uint8_t* data, uint32_t) {
    auto msg = AmdgpuProxy::GetRpcMessage(data);
    auto req = msg->request_as_KfdSetTrapHandlerRequest();

    std::lock_guard<std::mutex> lock(mutex_);
    int kfd = GetDeviceFd("/dev/kfd");
    if (kfd < 0) return BuildErrorResponse(ENODEV, "Cannot open /dev/kfd");

    struct kfd_ioctl_set_trap_handler_args args{};
    args.gpu_id = req ? req->args()->gpu_id() : 0;
    args.tba_addr = req ? req->args()->tba_addr() : 0;
    args.tma_addr = req ? req->args()->tma_addr() : 0;

    // Translate client VA → server VA for trap handler addresses
    auto translate = [&](uint64_t client_addr) -> uint64_t {
        for (auto& [cva, um] : userptr_addr_map_) {
            if (client_addr >= cva && client_addr < cva + um.size) {
                return um.server_va + (client_addr - cva);
            }
        }
        return client_addr;
    };
    if (args.tba_addr) args.tba_addr = translate(args.tba_addr);
    if (args.tma_addr) args.tma_addr = translate(args.tma_addr);

    fprintf(stderr, "[server] SET_TRAP_HANDLER: gpu=%u tba=0x%llx tma=0x%llx\n",
            args.gpu_id, (unsigned long long)args.tba_addr, (unsigned long long)args.tma_addr);

    if (::ioctl(kfd, AMDKFD_IOC_SET_TRAP_HANDLER, &args) < 0)
        return BuildErrorResponse(errno, "KFD SET_TRAP_HANDLER failed");

    FlatBufferBuilder fbb(128);
    auto resp = AmdgpuProxy::CreateKfdSetTrapHandlerResponse(fbb,
        AmdgpuProxy::CreateKfdSetTrapHandlerRets(fbb));
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_NONE, 0,
        AmdgpuProxy::ResponsePayload_KfdSetTrapHandlerResponse, resp.Union());
    fbb.Finish(rpc);
    return {fbb.GetBufferPointer(), fbb.GetBufferPointer() + fbb.GetSize()};
}

} // namespace amdgpu_proxy
