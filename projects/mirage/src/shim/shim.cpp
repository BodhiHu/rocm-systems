// shim.cpp — LD_PRELOAD shared library that intercepts GPU device access
//
// Interposes: open, open64, openat, close, ioctl, mmap, mmap64, munmap, fstat
//
// When the intercepted path matches a GPU device (/dev/dri/renderD*,
// /dev/dri/card*, /dev/kfd) or sysfs topology path, the call is
// redirected to the proxy server via a FlatBuffer RPC over Unix socket.
//
// Usage: LD_PRELOAD=libamdgpu_shim.so some_program

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "fd_table.h"
#include "ioctl_dispatch.h"
#include "../rpc/client.h"
#include "../rpc/transport.h"
#include "rpc_generated.h"
#include "rpc/str_util.h"

#include <dlfcn.h>
#include <cerrno>
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/sysmacros.h>
#include <sys/types.h>
#include <unistd.h>

using namespace amdgpu_proxy;

// Debug logging controlled by AMDGPU_SHIM_DEBUG env var
static int shim_debug_level() {
    static int level = -1;
    if (level < 0) {
        const char* e = getenv("AMDGPU_SHIM_DEBUG");
        level = e ? std::atoi(e) : 0;
    }
    return level;
}
#define SHIM_DBG(fmt, ...) do { if (shim_debug_level() > 0) std::fprintf(stderr, "[shim] " fmt "\n", ##__VA_ARGS__); } while(0)

// Called in child process after fork() to abandon inherited proxy
// connections and force HIP to re-open devices with fresh sockets.
static void atfork_child() {
    FdTable::Instance().ClearForFork();
}

// Register atfork handler via constructor attribute (runs before main).
__attribute__((constructor))
static void register_atfork() {
    pthread_atfork(nullptr, nullptr, atfork_child);
}

// Original function pointers
static int (*real_open)(const char*, int, ...) = nullptr;
static int (*real_open64)(const char*, int, ...) = nullptr;
static int (*real_openat)(int, const char*, int, ...) = nullptr;
static int (*real_close)(int) = nullptr;
static int (*real_ioctl)(int, unsigned long, ...) = nullptr;
static void* (*real_mmap)(void*, size_t, int, int, int, off_t) = nullptr;
static void* (*real_mmap64)(void*, size_t, int, int, int, off64_t) = nullptr;
static int (*real_munmap)(void*, size_t) = nullptr;
static int (*real_fstat)(int, struct stat*) = nullptr;
static ssize_t (*real_read)(int, void*, size_t) = nullptr;
static int (*real_access)(const char*, int) = nullptr;
static int (*real_faccessat)(int, const char*, int, int) = nullptr;
static int (*real___xstat)(int, const char*, struct stat*) = nullptr;
static int (*real_stat)(const char*, struct stat*) = nullptr;
static int (*real___fxstatat)(int, int, const char*, struct stat*, int) = nullptr;
static int (*real_fstatat)(int, const char*, struct stat*, int) = nullptr;
static int (*real_stat64)(const char*, struct stat64*) = nullptr;
static int (*real_lstat64)(const char*, struct stat64*) = nullptr;
static int (*real_dup)(int) = nullptr;
static int (*real_dup2)(int, int) = nullptr;
static int (*real_dup3)(int, int, int) = nullptr;
static ssize_t (*real_readlink)(const char*, char*, size_t) = nullptr;
static int (*real_fcntl)(int, int, ...) = nullptr;

static void init_real_funcs() {
    static bool initialized = false;
    if (initialized) return;
    initialized = true;

    real_open = reinterpret_cast<decltype(real_open)>(dlsym(RTLD_NEXT, "open"));
    real_open64 = reinterpret_cast<decltype(real_open64)>(dlsym(RTLD_NEXT, "open64"));
    real_openat = reinterpret_cast<decltype(real_openat)>(dlsym(RTLD_NEXT, "openat"));
    real_close = reinterpret_cast<decltype(real_close)>(dlsym(RTLD_NEXT, "close"));
    real_ioctl = reinterpret_cast<decltype(real_ioctl)>(dlsym(RTLD_NEXT, "ioctl"));
    real_mmap = reinterpret_cast<decltype(real_mmap)>(dlsym(RTLD_NEXT, "mmap"));
    real_mmap64 = reinterpret_cast<decltype(real_mmap64)>(dlsym(RTLD_NEXT, "mmap64"));
    real_munmap = reinterpret_cast<decltype(real_munmap)>(dlsym(RTLD_NEXT, "munmap"));
    // Try "fstat" first (glibc 2.33+), fall back to "__fxstat" (legacy glibc)
    real_fstat = reinterpret_cast<decltype(real_fstat)>(dlsym(RTLD_NEXT, "fstat"));
    if (!real_fstat)
        real_fstat = reinterpret_cast<decltype(real_fstat)>(dlsym(RTLD_NEXT, "__fxstat"));
    real_read = reinterpret_cast<decltype(real_read)>(dlsym(RTLD_NEXT, "read"));
    real_access = reinterpret_cast<decltype(real_access)>(dlsym(RTLD_NEXT, "access"));
    real_faccessat = reinterpret_cast<decltype(real_faccessat)>(dlsym(RTLD_NEXT, "faccessat"));
    // Try "stat" first (glibc 2.33+), fall back to "__xstat" (legacy glibc)
    real___xstat = reinterpret_cast<decltype(real___xstat)>(dlsym(RTLD_NEXT, "stat"));
    if (!real___xstat)
        real___xstat = reinterpret_cast<decltype(real___xstat)>(dlsym(RTLD_NEXT, "__xstat"));
    real_stat = reinterpret_cast<decltype(real_stat)>(dlsym(RTLD_NEXT, "stat"));
    // Try "fstatat" first (glibc 2.33+), fall back to "__fxstatat" (legacy glibc)
    real___fxstatat = reinterpret_cast<decltype(real___fxstatat)>(dlsym(RTLD_NEXT, "fstatat"));
    if (!real___fxstatat)
        real___fxstatat = reinterpret_cast<decltype(real___fxstatat)>(dlsym(RTLD_NEXT, "__fxstatat"));
    real_fstatat = reinterpret_cast<decltype(real_fstatat)>(dlsym(RTLD_NEXT, "fstatat"));
    real_stat64 = reinterpret_cast<decltype(real_stat64)>(dlsym(RTLD_NEXT, "stat64"));
    real_lstat64 = reinterpret_cast<decltype(real_lstat64)>(dlsym(RTLD_NEXT, "lstat64"));
    real_dup = reinterpret_cast<decltype(real_dup)>(dlsym(RTLD_NEXT, "dup"));
    real_dup2 = reinterpret_cast<decltype(real_dup2)>(dlsym(RTLD_NEXT, "dup2"));
    real_dup3 = reinterpret_cast<decltype(real_dup3)>(dlsym(RTLD_NEXT, "dup3"));
    real_readlink = reinterpret_cast<decltype(real_readlink)>(dlsym(RTLD_NEXT, "readlink"));
    real_fcntl = reinterpret_cast<decltype(real_fcntl)>(dlsym(RTLD_NEXT, "fcntl"));
}

// Check if path is a GPU device we should intercept
static bool is_gpu_device(const char* path) {
    if (!path) return false;
    return (std::strstr(path, "/dev/dri/renderD") != nullptr ||
            std::strstr(path, "/dev/dri/card") != nullptr ||
            std::strcmp(path, "/dev/kfd") == 0);
}

// Check if path is a sysfs topology path
static bool is_sysfs_topology(const char* path) {
    if (!path) return false;
    return std::strstr(path, "/sys/class/kfd/kfd/topology") != nullptr;
}

static DeviceType path_to_device_type(const char* path) {
    if (std::strstr(path, "/dev/dri/renderD")) return DeviceType::DRM_RENDER;
    if (std::strstr(path, "/dev/dri/card")) return DeviceType::DRM_CARD;
    return DeviceType::KFD;
}

// Fill a stat buffer to look like a character device so ROCm's stat()
// checks succeed even when the device files don't exist in the container.
static void fill_fake_device_stat(struct stat* buf, const char* path) {
    std::memset(buf, 0, sizeof(*buf));
    buf->st_mode = S_IFCHR | 0666;
    buf->st_nlink = 1;
    // Use stable fake dev/rdev numbers based on path
    if (std::strcmp(path, "/dev/kfd") == 0) {
        buf->st_rdev = makedev(245, 0);
    } else if (std::strstr(path, "/dev/dri/renderD")) {
        int minor = 128;
        const char* p = std::strstr(path, "renderD");
        if (p) minor = std::atoi(p + 7);
        buf->st_rdev = makedev(226, minor);
    } else if (std::strstr(path, "/dev/dri/card")) {
        int minor = 0;
        const char* p = std::strstr(path, "card");
        if (p) minor = std::atoi(p + 4);
        buf->st_rdev = makedev(226, minor);
    }
}

static void fill_fake_device_stat64(struct stat64* buf, const char* path) {
    std::memset(buf, 0, sizeof(*buf));
    buf->st_mode = S_IFCHR | 0666;
    buf->st_nlink = 1;
    if (std::strstr(path, "/dev/kfd")) {
        buf->st_rdev = makedev(245, 0);
    } else if (const char* p = std::strstr(path, "renderD")) {
        int minor = std::atoi(p + 7);
        buf->st_rdev = makedev(226, minor);
    } else if (const char* c = std::strstr(path, "card")) {
        int minor = std::atoi(c + 4);
        buf->st_rdev = makedev(226, minor);
    }
}

// Open a sysfs topology file by proxying its content from the server.
// Returns a memfd containing the file content, or -1 on failure.
static int do_sysfs_proxy_open(const char* path) {
    init_real_funcs();

    auto client = std::make_shared<ProxyClient>();
    if (!client->Connect()) {
        errno = ENOENT;
        return -1;
    }

    char buf[8192];
    int ret = DispatchSysfsRead(*client, path, buf, sizeof(buf));
    if (ret <= 0) {
        errno = ENOENT;
        return -1;
    }

    // Create a memfd with the content
    int mfd = static_cast<int>(syscall(SYS_memfd_create, "proxy_sysfs", 0));
    if (mfd < 0) {
        errno = ENOMEM;
        return -1;
    }
    if (::write(mfd, buf, ret) != ret) {
        real_close(mfd);
        errno = EIO;
        return -1;
    }
    lseek(mfd, 0, SEEK_SET);
    return mfd;
}

static int do_proxy_open(const char* path, int flags) {
    init_real_funcs();

    auto client = std::make_shared<ProxyClient>();
    if (!client->Connect()) {
        errno = ENODEV;
        return -1;
    }

    // Send open request
    flatbuffers::FlatBufferBuilder fbb(256);
    AmdgpuProxy::OpenArgs _args(AmdgpuProxy::make_str<AmdgpuProxy::Str255>(path), flags, 0);
    auto req = AmdgpuProxy::CreateOpenRequest(fbb, nullptr, &_args);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_OpenRequest, req.Union());
    fbb.Finish(rpc);

    auto resp_buf = client->SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
    if (resp_buf.empty()) {
        errno = ENODEV;
        return -1;
    }

    auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
    if (!msg || msg->error()) {
        errno = msg && msg->error() ? msg->error()->code() : ENODEV;
        return -1;
    }

    auto resp = msg->response_as_OpenResponse();
    int virtual_fd = resp ? resp->ret()->virtual_fd() : -1;
    if (virtual_fd < 0) {
        errno = ENODEV;
        return -1;
    }

    DeviceType type = path_to_device_type(path);
    int real_fd = FdTable::Instance().Add(virtual_fd, type, client, path);
    if (real_fd < 0) {
        errno = EMFILE;
        return -1;
    }

    SHIM_DBG("open(%s) → local_fd=%d virtual_fd=%d type=%d", path, real_fd, virtual_fd, (int)type);
    return real_fd;
}

// --- Interposed functions ---

extern "C" {

int open(const char* path, int flags, ...) {
    init_real_funcs();

    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap;
        va_start(ap, flags);
        mode = static_cast<mode_t>(va_arg(ap, int));
        va_end(ap);
    }

    if (is_gpu_device(path)) {
        return do_proxy_open(path, flags);
    }

    if (is_sysfs_topology(path)) {
        fprintf(stderr, "[shim] sysfs open: %s\n", path);
        int fd = do_sysfs_proxy_open(path);
        if (fd >= 0) return fd;
        fprintf(stderr, "[shim] sysfs open FAILED: %s\n", path);
        // Fall through to real open on failure
    }

    return real_open(path, flags, mode);
}

int open64(const char* path, int flags, ...) {
    init_real_funcs();

    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap;
        va_start(ap, flags);
        mode = static_cast<mode_t>(va_arg(ap, int));
        va_end(ap);
    }

    if (is_gpu_device(path)) {
        return do_proxy_open(path, flags);
    }

    if (is_sysfs_topology(path)) {
        fprintf(stderr, "[shim] sysfs open64: %s\n", path);
        int fd = do_sysfs_proxy_open(path);
        if (fd >= 0) return fd;
    }

    if (real_open64) return real_open64(path, flags, mode);
    return real_open(path, flags, mode);
}

int openat(int dirfd, const char* path, int flags, ...) {
    init_real_funcs();

    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap;
        va_start(ap, flags);
        mode = static_cast<mode_t>(va_arg(ap, int));
        va_end(ap);
    }

    if (is_gpu_device(path)) {
        return do_proxy_open(path, flags);
    }

    if (is_sysfs_topology(path)) {
        int fd = do_sysfs_proxy_open(path);
        if (fd >= 0) return fd;
    }

    return real_openat(dirfd, path, flags, mode);
}

int close(int fd) {
    init_real_funcs();

    if (FdTable::Instance().IsVirtual(fd)) {
        SHIM_DBG("close fd=%d (virtual)", fd);
        auto* entry = FdTable::Instance().Lookup(fd);
        if (entry && entry->client) {
            // Send close notification to server
            flatbuffers::FlatBufferBuilder fbb(128);
            AmdgpuProxy::CloseArgs _args(entry->virtual_fd);
            auto req = AmdgpuProxy::CreateCloseRequest(fbb, nullptr, &_args);
            auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
                AmdgpuProxy::RequestPayload_CloseRequest, req.Union());
            fbb.Finish(rpc);
            entry->client->SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
        }
        FdTable::Instance().Remove(fd);
        return 0;
    }

    return real_close(fd);
}

// dup/dup2/dup3 — track duplicated virtual GPU fds
int dup(int oldfd) {
    init_real_funcs();
    int newfd = real_dup(oldfd);
    if (newfd >= 0 && FdTable::Instance().IsVirtual(oldfd)) {
        FdTable::Instance().Dup(oldfd, newfd);
        SHIM_DBG("dup(%d) → %d", oldfd, newfd);
    }
    return newfd;
}

int dup2(int oldfd, int newfd) {
    init_real_funcs();
    int ret = real_dup2(oldfd, newfd);
    if (ret >= 0 && FdTable::Instance().IsVirtual(oldfd)) {
        FdTable::Instance().Dup(oldfd, ret);
        SHIM_DBG("dup2(%d, %d) → %d", oldfd, newfd, ret);
    }
    return ret;
}

int dup3(int oldfd, int newfd, int flags) {
    init_real_funcs();
    int ret = real_dup3(oldfd, newfd, flags);
    if (ret >= 0 && FdTable::Instance().IsVirtual(oldfd)) {
        FdTable::Instance().Dup(oldfd, ret);
        SHIM_DBG("dup3(%d, %d) → %d", oldfd, newfd, ret);
    }
    return ret;
}

// readlink — for /proc/self/fd/N, return the virtual device path 
ssize_t readlink(const char* path, char* buf, size_t bufsiz) {
    init_real_funcs();
    // Check if /proc/self/fd/<N>
    if (path && std::strstr(path, "/proc/self/fd/")) {
        const char* num = path + std::strlen("/proc/self/fd/");
        char* end;
        long fd = std::strtol(num, &end, 10);
        if (end != num && *end == '\0') {
            auto* entry = FdTable::Instance().Lookup(static_cast<int>(fd));
            if (entry) {
                size_t len = std::min(entry->device_path.size(), bufsiz);
                std::memcpy(buf, entry->device_path.c_str(), len);
                return static_cast<ssize_t>(len);
            }
        }
    }
    return real_readlink(path, buf, bufsiz);
}

// fcntl — track F_DUPFD and F_DUPFD_CLOEXEC on virtual GPU fds
int fcntl(int fd, int cmd, ...) {
    init_real_funcs();

    va_list ap;
    va_start(ap, cmd);
    // fcntl's third argument type depends on cmd
    long arg = va_arg(ap, long);
    va_end(ap);

    int ret = real_fcntl(fd, cmd, arg);

    // Track dup operations on virtual GPU fds
    if (ret >= 0 && (cmd == F_DUPFD || cmd == F_DUPFD_CLOEXEC)) {
        if (FdTable::Instance().IsVirtual(fd)) {
            FdTable::Instance().Dup(fd, ret);
            SHIM_DBG("fcntl(%d, F_DUPFD%s) → %d", fd,
                      cmd == F_DUPFD_CLOEXEC ? "_CLOEXEC" : "", ret);
        }
    }

    return ret;
}

// Also intercept fcntl64 (used by libdrm_amdgpu compiled against GLIBC_2.28+)
int fcntl64(int fd, int cmd, ...) {
    init_real_funcs();

    va_list ap;
    va_start(ap, cmd);
    long arg = va_arg(ap, long);
    va_end(ap);

    int ret = real_fcntl(fd, cmd, arg);

    if (ret >= 0 && (cmd == F_DUPFD || cmd == F_DUPFD_CLOEXEC)) {
        if (FdTable::Instance().IsVirtual(fd)) {
            FdTable::Instance().Dup(fd, ret);
            SHIM_DBG("fcntl64(%d, F_DUPFD%s) → %d", fd,
                      cmd == F_DUPFD_CLOEXEC ? "_CLOEXEC" : "", ret);
        }
    }

    return ret;
}

// fstat on virtual GPU fds should report character device
int fstat(int fd, struct stat* buf) {
    init_real_funcs();
    auto* entry = FdTable::Instance().Lookup(fd);
    if (entry && buf) {
        fill_fake_device_stat(buf, entry->device_path.c_str());
        return 0;
    }
    if (real_fstat) return real_fstat(fd, buf);
    return syscall(SYS_fstat, fd, buf);
}

int fstat64(int fd, struct stat64* buf) {
    init_real_funcs();
    auto* entry = FdTable::Instance().Lookup(fd);
    if (entry && buf) {
        fill_fake_device_stat64(buf, entry->device_path.c_str());
        return 0;
    }
    // On glibc 2.33+ with _FILE_OFFSET_BITS=64, stat and stat64 are the same type
    if (real_fstat) return real_fstat(fd, reinterpret_cast<struct stat*>(buf));
    return syscall(SYS_fstat, fd, buf);
}

int __fxstat(int ver, int fd, struct stat* buf) {
    init_real_funcs();
    auto* entry = FdTable::Instance().Lookup(fd);
    if (entry && buf) {
        fill_fake_device_stat(buf, entry->device_path.c_str());
        return 0;
    }
    if (real_fstat) return real_fstat(fd, buf);
    return syscall(SYS_fstat, fd, buf);
}

int ioctl(int fd, unsigned long request, ...) {
    init_real_funcs();

    va_list ap;
    va_start(ap, request);
    void* arg = va_arg(ap, void*);
    va_end(ap);

    SHIM_DBG("ioctl enter fd=%d nr=0x%02lx type=0x%02lx", fd, request & 0xff, (request >> 8) & 0xff);

    auto* entry = FdTable::Instance().Lookup(fd);
    if (entry && entry->client) {
        // All ioctls go through the proxy server. The server has real device
        // access and executes ioctls in its own process context (required for
        // KFD which binds mm_struct at open() time).
        if (entry->type == DeviceType::KFD) {
            // For ACQUIRE_VM, translate the client-side fd to the virtual_fd
            // so the server can look up the correct DRM device.
            unsigned int kfd_nr = request & 0xff;
            if (kfd_nr == 0x15 && arg) {  // ACQUIRE_VM
                struct { uint32_t drm_fd; uint32_t gpu_id; }* av =
                    static_cast<decltype(av)>(arg);
                auto* drm_entry = FdTable::Instance().Lookup(static_cast<int>(av->drm_fd));
                if (drm_entry) {
                    SHIM_DBG("ACQUIRE_VM: translating drm_fd=%u → virtual_fd=%d", av->drm_fd, drm_entry->virtual_fd);
                    av->drm_fd = static_cast<uint32_t>(drm_entry->virtual_fd);
                }
            }
            SHIM_DBG("ioctl KFD fd=%d cmd=0x%02lx", fd, (request >> 8) & 0xff);
            int ret = DispatchKfdIoctl(*entry->client, request, arg);
            SHIM_DBG("ioctl KFD fd=%d cmd=0x%02lx → %d", fd, (request >> 8) & 0xff, ret);
            if (ret < 0) { errno = -ret; return -1; }
            return 0;
        } else {
            SHIM_DBG("ioctl DRM fd=%d nr=0x%02lx vfd=%d", fd, request & 0xff, entry->virtual_fd);
            int ret = DispatchDrmIoctl(*entry->client, request, arg, entry->virtual_fd);
            SHIM_DBG("ioctl DRM fd=%d nr=0x%02lx → %d", fd, request & 0xff, ret);
            if (ret < 0) { errno = -ret; return -1; }
            return 0;
        }
    }

    return real_ioctl(fd, request, arg);
}

// Helper: send mmap request to server & receive memfd/dma-buf fd via SCM_RIGHTS
static void* proxy_mmap(void* addr, size_t length, int prot, int flags,
                        FdEntry* entry, uint64_t offset) {
    // Build MmapRequest
    flatbuffers::FlatBufferBuilder fbb(256);
    AmdgpuProxy::MmapArgs _args(entry->virtual_fd, offset, length, prot, flags);
    auto req = AmdgpuProxy::CreateMmapRequest(fbb, nullptr, &_args);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_MmapRequest, req.Union());
    fbb.Finish(rpc);

    int received_fd = -1;
    auto resp_buf = entry->client->SendRequestReceiveFd(
        fbb.GetBufferPointer(), fbb.GetSize(), &received_fd);

    SHIM_DBG("proxy_mmap: vfd=%d offset=0x%llx len=%zu received_fd=%d resp_size=%zu",
             entry->virtual_fd, (unsigned long long)offset, length, received_fd, resp_buf.size());

    if (received_fd >= 0) {
        // Map the received fd (dma-buf or memfd) into our address space
        // Preserve MAP_FIXED from original flags so HSA runtime gets mappings at expected addresses
        int mmap_flags = MAP_SHARED;
        if (flags & MAP_FIXED)
            mmap_flags |= MAP_FIXED;
        void* ptr = real_mmap(addr, length, prot, mmap_flags, received_fd, 0);
        if (ptr == MAP_FAILED) {
            SHIM_DBG("proxy_mmap: mmap of received fd=%d failed: %d (flags=0x%x)", received_fd, errno, mmap_flags);
            syscall(SYS_close, received_fd);
            // Fallback to anonymous
            ptr = real_mmap(addr, length, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (ptr != MAP_FAILED) std::memset(ptr, 0, length);
        } else {
            // Keep the received fd open (needed for the mapping to persist)
            SHIM_DBG("proxy_mmap: mapped received_fd=%d at %p (flags=0x%x)", received_fd, ptr, mmap_flags);
        }
        return ptr;
    }

    // No fd received — fall back to anonymous mapping
    SHIM_DBG("proxy_mmap: no fd received, using anonymous mapping");
    void* ptr = real_mmap(addr, length, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (ptr != MAP_FAILED) std::memset(ptr, 0, length);
    return ptr;
}

void* mmap(void* addr, size_t length, int prot, int flags, int fd, off_t offset) {
    init_real_funcs();

    auto* entry = FdTable::Instance().Lookup(fd);
    if (entry && entry->client) {
        SHIM_DBG("mmap fd=%d len=%zu prot=0x%x flags=0x%x off=0x%lx vfd=%d",
                 fd, length, prot, flags, (long)offset, entry->virtual_fd);
        void* ptr = proxy_mmap(addr, length, prot, flags, entry, static_cast<uint64_t>(offset));
        if (ptr != MAP_FAILED) {
            FdTable::Instance().AddMmap(ptr, length, fd, "gpu");
        }
        SHIM_DBG("mmap fd=%d → %p", fd, ptr);
        return ptr;
    }

    return real_mmap(addr, length, prot, flags, fd, offset);
}

void* mmap64(void* addr, size_t length, int prot, int flags, int fd, off64_t offset) {
    init_real_funcs();

    auto* entry = FdTable::Instance().Lookup(fd);
    if (entry && entry->client) {
        SHIM_DBG("mmap64 fd=%d len=%zu prot=0x%x flags=0x%x off=0x%llx vfd=%d",
                 fd, length, prot, flags, (long long)offset, entry->virtual_fd);
        void* ptr = proxy_mmap(addr, length, prot, flags, entry, static_cast<uint64_t>(offset));
        if (ptr != MAP_FAILED) {
            FdTable::Instance().AddMmap(ptr, length, fd, "gpu");
        }
        SHIM_DBG("mmap64 fd=%d → %p", fd, ptr);
        return ptr;
    }

    if (real_mmap64) return real_mmap64(addr, length, prot, flags, fd, offset);
    return real_mmap(addr, length, prot, flags, fd, static_cast<off_t>(offset));
}

int munmap(void* addr, size_t length) {
    init_real_funcs();

    MmapRegion region;
    if (FdTable::Instance().RemoveMmap(addr, region)) {
        return real_munmap(addr, length);
    }

    return real_munmap(addr, length);
}

ssize_t read(int fd, void* buf, size_t count) {
    init_real_funcs();

    // For sysfs topology reads on virtual fds, we can intercept here
    // But typically sysfs is read via regular file open, not virtual fd
    // The shim handles sysfs differently — see open() for sysfs paths

    auto* entry = FdTable::Instance().Lookup(fd);
    if (entry && entry->client) {
        SHIM_DBG("read fd=%d count=%zu", fd, count);
        // DRM event reads — return 0 (no events in simulator)
        return 0;
    }

    return real_read(fd, buf, count);
}

// --- access / faccessat — report GPU devices as accessible ---

int access(const char* path, int mode) {
    init_real_funcs();
    if (is_gpu_device(path)) return 0;
    return real_access(path, mode);
}

int faccessat(int dirfd, const char* path, int mode, int flags) {
    init_real_funcs();
    if (is_gpu_device(path)) return 0;
    if (real_faccessat) return real_faccessat(dirfd, path, mode, flags);
    // Fallback: resolve path and use access()
    return real_access(path, mode);
}

// --- stat / __xstat — report GPU devices as character devices ---

int __xstat(int ver, const char* path, struct stat* buf) {
    init_real_funcs();
    if (is_gpu_device(path) && buf) {
        fill_fake_device_stat(buf, path);
        return 0;
    }
    if (real___xstat) return real___xstat(ver, path, buf);
    if (real_stat) return real_stat(path, buf);
    return syscall(SYS_newfstatat, AT_FDCWD, path, buf, 0);
}

int stat(const char* path, struct stat* buf) {
    init_real_funcs();
    if (is_gpu_device(path) && buf) {
        fill_fake_device_stat(buf, path);
        return 0;
    }
    if (real_stat) return real_stat(path, buf);
    if (real___xstat) return real___xstat(1, path, buf);
    return syscall(SYS_newfstatat, AT_FDCWD, path, buf, 0);
}

// Modern glibc uses fstatat(AT_FDCWD, path, buf, 0) for stat().
// Also called as __fxstatat on older glibc. We need both.

int fstatat(int dirfd, const char* path, struct stat* buf, int flags) {
    init_real_funcs();
    if (is_gpu_device(path) && buf) {
        fill_fake_device_stat(buf, path);
        return 0;
    }
    if (real_fstatat) return real_fstatat(dirfd, path, buf, flags);
    if (real___fxstatat) return real___fxstatat(1, dirfd, path, buf, flags);
    return syscall(SYS_newfstatat, dirfd, path, buf, flags);
}

// CPython 3.12+ calls stat64/lstat64/fstatat64 (linked against GLIBC_2.33).

int stat64(const char* path, struct stat64* buf) {
    init_real_funcs();
    if (is_gpu_device(path) && buf) {
        fill_fake_device_stat64(buf, path);
        return 0;
    }
    if (real_stat64) return real_stat64(path, buf);
    // Fallback: use raw syscall so we never return ENOSYS for normal files
    return syscall(SYS_newfstatat, AT_FDCWD, path, buf, 0);
}

int lstat64(const char* path, struct stat64* buf) {
    init_real_funcs();
    if (is_gpu_device(path) && buf) {
        fill_fake_device_stat64(buf, path);
        return 0;
    }
    if (real_lstat64) return real_lstat64(path, buf);
    return syscall(SYS_newfstatat, AT_FDCWD, path, buf, AT_SYMLINK_NOFOLLOW);
}

int fstatat64(int dirfd, const char* path, struct stat64* buf, int flags) {
    init_real_funcs();
    if (is_gpu_device(path) && buf) {
        fill_fake_device_stat64(buf, path);
        return 0;
    }
    if (real_fstatat) return real_fstatat(dirfd, path, reinterpret_cast<struct stat*>(buf), flags);
    return syscall(SYS_newfstatat, dirfd, path, buf, flags);
}

int __fxstatat(int ver, int dirfd, const char* path, struct stat* buf, int flags) {
    init_real_funcs();
    if (is_gpu_device(path) && buf) {
        fill_fake_device_stat(buf, path);
        return 0;
    }
    if (real___fxstatat) return real___fxstatat(ver, dirfd, path, buf, flags);
    if (real_fstatat) return real_fstatat(dirfd, path, buf, flags);
    errno = ENOSYS;
    return -1;
}

} // extern "C"
