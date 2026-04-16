// fd_table.h — Virtual file descriptor management for the LD_PRELOAD shim
#pragma once

#include "../rpc/client.h"

#include <cstdint>
#include <memory>
#include <mutex>
#include <shared_mutex>
#include <string>
#include <unordered_map>
#include <unordered_set>

namespace amdgpu_proxy {

/// Type of virtual device
enum class DeviceType {
    DRM_RENDER,   // /dev/dri/renderD*
    DRM_CARD,     // /dev/dri/card*
    KFD,          // /dev/kfd
};

/// Entry in the virtual FD table
struct FdEntry {
    int real_fd;                        // A real kernel fd (memfd) for fstat compat
    int virtual_fd;                     // Server-assigned virtual fd
    int passed_fd = -1;                 // Real device fd received via SCM_RIGHTS for mmap
    DeviceType type;
    std::shared_ptr<ProxyClient> client;
    std::string device_path;
};

/// Tracks mmap regions belonging to virtual fds for proper munmap handling.
struct MmapRegion {
    void* addr;
    size_t length;
    int fd;
    std::string shm_name;
};

/// Singleton FD table mapping intercepted fds to proxy client connections.
/// Thread-safe with a read-write lock.
class FdTable {
public:
    static FdTable& Instance();

    /// Register a new virtual fd, creating a real kernel fd placeholder.
    /// If passed_fd >= 0, it is the real device fd from SCM_RIGHTS for mmap.
    /// Returns the real fd to return to the caller, or -1 on failure.
    int Add(int virtual_fd, DeviceType type,
            std::shared_ptr<ProxyClient> client,
            const std::string& device_path,
            int passed_fd = -1);

    /// Look up an entry by real fd. Returns nullptr if not tracked.
    FdEntry* Lookup(int fd);

    /// Remove and close an entry by real fd. Returns true if found.
    bool Remove(int fd);

    /// Check if a real fd is a virtual GPU fd.
    bool IsVirtual(int fd);

    /// Duplicate a virtual fd entry for a new real fd (for dup/dup2/dup3).
    /// Returns true if old_fd was virtual and was successfully duplicated.
    bool Dup(int old_fd, int new_fd);

    /// Track an mmap region.
    void AddMmap(void* addr, size_t length, int fd, const std::string& shm_name);

    /// Find and remove an mmap region by address. Returns true if found.
    bool RemoveMmap(void* addr, MmapRegion& out);

    /// Clear all entries and mmaps.  Called in child after fork() so that
    /// inherited proxy connections (shared socket fds) are abandoned and
    /// fresh ones are created on the next open().
    void ClearForFork();

private:
    FdTable() = default;

    mutable std::shared_mutex mutex_;
    std::unordered_map<int, FdEntry> entries_;        // real_fd → entry
    std::unordered_map<uintptr_t, MmapRegion> mmaps_; // addr → region
};

} // namespace amdgpu_proxy
