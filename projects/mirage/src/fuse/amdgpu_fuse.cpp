// amdgpu_fuse.cpp — FUSE filesystem exposing virtual GPU device nodes
//
// Mounts a FUSE filesystem that exposes:
//   - renderD128  (virtual /dev/dri/renderD128)
//   - card0       (virtual /dev/dri/card0)
//   - kfd         (virtual /dev/kfd)
//
// All file operations (open, ioctl, read, poll) are forwarded to
// the proxy server via the RPC transport.
//
// Usage: amdgpu-fuse --mountpoint /tmp/amdgpu-fuse --socket /tmp/amdgpu-proxy.sock

#define FUSE_USE_VERSION 31

#include <fuse3/fuse.h>
#include <fuse3/fuse_lowlevel.h>

#include "../rpc/client.h"
#include "../rpc/transport.h"
#include "rpc_generated.h"
#include "rpc/str_util.h"

#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <sys/sysmacros.h>

using namespace amdgpu_proxy;

// Global state
static std::string g_socket_path;

struct FuseFileHandle {
    std::shared_ptr<ProxyClient> client;
    int virtual_fd;
    std::string device_name;
};

static std::mutex g_handles_mutex;
static std::unordered_map<uint64_t, std::unique_ptr<FuseFileHandle>> g_handles;
static uint64_t g_next_fh = 1;

// File entries in our virtual filesystem
struct VirtualEntry {
    const char* name;
    mode_t mode;
    dev_t rdev;
};

static const VirtualEntry g_entries[] = {
    {"renderD128", S_IFCHR | 0666, makedev(226, 128)},
    {"card0",      S_IFCHR | 0666, makedev(226, 0)},
    {"kfd",        S_IFCHR | 0666, makedev(241, 0)},
};
static const int g_num_entries = sizeof(g_entries) / sizeof(g_entries[0]);

static const VirtualEntry* find_entry(const char* name) {
    for (int i = 0; i < g_num_entries; i++) {
        if (std::strcmp(g_entries[i].name, name) == 0) return &g_entries[i];
    }
    return nullptr;
}

// FUSE operations

static int fuse_getattr(const char* path, struct stat* st, struct fuse_file_info*) {
    std::memset(st, 0, sizeof(*st));

    if (std::strcmp(path, "/") == 0) {
        st->st_mode = S_IFDIR | 0755;
        st->st_nlink = 2 + g_num_entries;
        return 0;
    }

    const char* name = path + 1; // skip leading '/'
    auto* entry = find_entry(name);
    if (!entry) return -ENOENT;

    st->st_mode = entry->mode;
    st->st_nlink = 1;
    st->st_rdev = entry->rdev;
    st->st_size = 0;
    return 0;
}

static int fuse_readdir(const char* path, void* buf, fuse_fill_dir_t filler,
                        off_t, struct fuse_file_info*, enum fuse_readdir_flags) {
    if (std::strcmp(path, "/") != 0) return -ENOENT;

    filler(buf, ".", nullptr, 0, static_cast<fuse_fill_dir_flags>(0));
    filler(buf, "..", nullptr, 0, static_cast<fuse_fill_dir_flags>(0));

    for (int i = 0; i < g_num_entries; i++) {
        filler(buf, g_entries[i].name, nullptr, 0, static_cast<fuse_fill_dir_flags>(0));
    }
    return 0;
}

static int fuse_open(const char* path, struct fuse_file_info* fi) {
    const char* name = path + 1;
    auto* entry = find_entry(name);
    if (!entry) return -ENOENT;

    auto client = std::make_shared<ProxyClient>();
    if (!client->Connect(g_socket_path)) return -EIO;

    // Build device path
    std::string device_path;
    if (std::strcmp(name, "kfd") == 0) {
        device_path = "/dev/kfd";
    } else {
        device_path = std::string("/dev/dri/") + name;
    }

    // Send open request
    flatbuffers::FlatBufferBuilder fbb(256);
    AmdgpuProxy::OpenArgs _args(AmdgpuProxy::make_str<AmdgpuProxy::Str255>(device_path), fi->flags, 0);
    auto req = AmdgpuProxy::CreateOpenRequest(fbb, nullptr, &_args);
    auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
        AmdgpuProxy::RequestPayload_OpenRequest, req.Union());
    fbb.Finish(rpc);

    auto resp_buf = client->SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
    if (resp_buf.empty()) return -EIO;

    auto msg = AmdgpuProxy::GetRpcMessage(resp_buf.data());
    if (!msg || msg->error()) return -EIO;

    auto resp = msg->response_as_OpenResponse();
    int virtual_fd = resp ? resp->ret()->virtual_fd() : -1;
    if (virtual_fd < 0) return -EIO;

    auto fh = std::make_unique<FuseFileHandle>();
    fh->client = std::move(client);
    fh->virtual_fd = virtual_fd;
    fh->device_name = name;

    std::lock_guard<std::mutex> lock(g_handles_mutex);
    uint64_t handle = g_next_fh++;
    fi->fh = handle;
    fi->direct_io = 1;
    g_handles[handle] = std::move(fh);

    return 0;
}

static int fuse_release(const char*, struct fuse_file_info* fi) {
    std::lock_guard<std::mutex> lock(g_handles_mutex);
    auto it = g_handles.find(fi->fh);
    if (it == g_handles.end()) return 0;

    auto& fh = it->second;
    if (fh->client) {
        flatbuffers::FlatBufferBuilder fbb(128);
        AmdgpuProxy::CloseArgs _args(fh->virtual_fd);
        auto req = AmdgpuProxy::CreateCloseRequest(fbb, nullptr, &_args);
        auto rpc = AmdgpuProxy::CreateRpcMessage(fbb,
            AmdgpuProxy::RequestPayload_CloseRequest, req.Union());
        fbb.Finish(rpc);
        fh->client->SendRequest(fbb.GetBufferPointer(), fbb.GetSize());
    }

    g_handles.erase(it);
    return 0;
}

static int fuse_read_op(const char*, char* buf, size_t size, off_t,
                        struct fuse_file_info* fi) {
    // DRM event reads — return 0 for simulator
    (void)buf; (void)size; (void)fi;
    return 0;
}

static const struct fuse_operations fuse_ops = {
    .getattr = fuse_getattr,
    .open    = fuse_open,
    .read    = fuse_read_op,
    .release = fuse_release,
    .readdir = fuse_readdir,
};

int main(int argc, char* argv[]) {
    // Parse our custom args before passing to fuse
    g_socket_path = GetSocketPath();

    for (int i = 1; i < argc; i++) {
        if (std::strcmp(argv[i], "--socket") == 0 && i + 1 < argc) {
            g_socket_path = argv[i + 1];
            // Remove these args so FUSE doesn't see them
            for (int j = i; j < argc - 2; j++) argv[j] = argv[j + 2];
            argc -= 2;
            i--;
        }
    }

    std::fprintf(stderr, "[amdgpu-fuse] Using socket: %s\n", g_socket_path.c_str());
    return fuse_main(argc, argv, &fuse_ops, nullptr);
}
