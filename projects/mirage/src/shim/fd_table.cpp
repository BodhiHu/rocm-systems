// fd_table.cpp — Virtual FD table implementation
#include "fd_table.h"

#include <unistd.h>
#include <sys/mman.h>
#include <linux/memfd.h>
#include <sys/syscall.h>
#include <fcntl.h>
#include <cstring>

// Raw close that bypasses the LD_PRELOAD-intercepted close().
// Without this, FdTable::Remove() would deadlock: it holds a unique_lock
// on mutex_ and calls ::close(), which re-enters the shimmed close()
// that tries to acquire a shared_lock on the same (non-recursive) mutex.
static inline int raw_close(int fd) {
    return static_cast<int>(syscall(SYS_close, fd));
}

namespace amdgpu_proxy {

FdTable& FdTable::Instance() {
    static FdTable instance;
    return instance;
}

int FdTable::Add(int virtual_fd, DeviceType type,
                 std::shared_ptr<ProxyClient> client,
                 const std::string& device_path,
                 int passed_fd) {
    // Create a real kernel fd using memfd_create so that fstat/poll work
    int real_fd = static_cast<int>(syscall(SYS_memfd_create, "amdgpu-proxy", MFD_CLOEXEC));
    if (real_fd < 0) return -1;

    std::unique_lock lock(mutex_);
    entries_[real_fd] = FdEntry{real_fd, virtual_fd, passed_fd, type, std::move(client), device_path};
    return real_fd;
}

FdEntry* FdTable::Lookup(int fd) {
    std::shared_lock lock(mutex_);
    auto it = entries_.find(fd);
    if (it == entries_.end()) return nullptr;
    return &it->second;
}

bool FdTable::Remove(int fd) {
    FdEntry entry_copy;
    {
        std::unique_lock lock(mutex_);
        auto it = entries_.find(fd);
        if (it == entries_.end()) return false;
        entry_copy = std::move(it->second);
        entries_.erase(it);
    }
    // Close the memfd outside the lock to avoid deadlock
    raw_close(fd);
    // Close the passed device fd if present and not shared by other entries
    // Note: don't close passed_fd here since dup'd entries may share it
    // entry_copy destructs here, outside the lock.
    return true;
}

bool FdTable::IsVirtual(int fd) {
    std::shared_lock lock(mutex_);
    return entries_.count(fd) > 0;
}

bool FdTable::Dup(int old_fd, int new_fd) {
    std::unique_lock lock(mutex_);
    auto it = entries_.find(old_fd);
    if (it == entries_.end()) return false;
    // Share the same virtual_fd, client, device_path, and passed_fd
    entries_[new_fd] = FdEntry{new_fd, it->second.virtual_fd, it->second.passed_fd,
                                it->second.type, it->second.client, it->second.device_path};
    return true;
}

void FdTable::AddMmap(void* addr, size_t length, int fd, const std::string& shm_name) {
    std::unique_lock lock(mutex_);
    mmaps_[reinterpret_cast<uintptr_t>(addr)] = MmapRegion{addr, length, fd, shm_name};
}

bool FdTable::RemoveMmap(void* addr, MmapRegion& out) {
    std::unique_lock lock(mutex_);
    auto it = mmaps_.find(reinterpret_cast<uintptr_t>(addr));
    if (it == mmaps_.end()) return false;
    out = it->second;
    mmaps_.erase(it);
    return true;
}

void FdTable::ClearForFork() {
    // Called in the child process after fork().
    // Close all memfd placeholders and drop ProxyClient references.
    // Do NOT lock: the child is single-threaded at this point and the
    // mutex may be in an inconsistent state if the parent held it.
    for (auto& [real_fd, entry] : entries_) {
        // Disconnect the proxy client (closes the shared socket fd so the
        // child stops sharing it with the parent).
        if (entry.client) entry.client->Disconnect();
        // Close the memfd placeholder.
        raw_close(real_fd);
        // Close passed_fd (DMA-buf or device fd) if any.
        if (entry.passed_fd >= 0) raw_close(entry.passed_fd);
    }
    entries_.clear();
    mmaps_.clear();
}

} // namespace amdgpu_proxy
