// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

/// @file remote_driver.cpp
/// @brief Client-side RPC stub for the rocjitsu daemon.

#include "rocjitsu/kmd/linux/remote_driver.h"
#include "rocjitsu/kmd/linux/rpc.h"

#include "rocjitsu/base/rj_compiler.h"
RJ_DIAGNOSTIC_PUSH
RJ_DIAGNOSTIC_IGNORE_PEDANTIC
#include "linux/uapi/kfd_ioctl.h"
RJ_DIAGNOSTIC_POP

#include <cerrno>
#include <cstring>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/un.h>
#include <unistd.h>
#include <vector>

namespace rocjitsu {

std::atomic<RemoteDriver *> RemoteDriver::instance_{nullptr};
std::atomic<int> RemoteDriver::kfd_fd_{-1};

RemoteDriver *RemoteDriver::get_or_create() {
  if (auto *inst = instance_.load(std::memory_order_acquire))
    return inst;

  auto path = rpc_default_socket_path();
  int sock = socket(AF_UNIX, SOCK_STREAM, 0);
  if (sock < 0)
    return nullptr;

  struct sockaddr_un addr = {};
  addr.sun_family = AF_UNIX;
  std::strncpy(addr.sun_path, path.c_str(), sizeof(addr.sun_path) - 1);

  if (connect(sock, reinterpret_cast<struct sockaddr *>(&addr), sizeof(addr)) != 0) {
    ::close(sock);
    return nullptr;
  }

  auto *driver = new RemoteDriver(sock);
  int fd = driver->open();
  if (fd < 0) {
    delete driver;
    return nullptr;
  }
  kfd_fd_.store(fd, std::memory_order_release);
  instance_.store(driver, std::memory_order_release);
  return driver;
}

RemoteDriver *RemoteDriver::lookup(int fd) {
  auto *inst = instance_.load(std::memory_order_acquire);
  return (fd >= 0 && fd == kfd_fd_.load(std::memory_order_acquire) && inst) ? inst : nullptr;
}

int RemoteDriver::kfd_fd() { return kfd_fd_.load(std::memory_order_acquire); }

std::string RemoteDriver::topology_path() {
  auto *inst = instance_.load(std::memory_order_acquire);
  return inst ? inst->topology_path_ : std::string{};
}

RemoteDriver::RemoteDriver(int sock_fd) : sock_(sock_fd) {}

RemoteDriver::~RemoteDriver() {
  if (sock_ >= 0)
    ::close(sock_);
}

int RemoteDriver::open() {
  // Send handshake to get topology path and gpu_id from daemon.
  RpcHeader hdr = {};
  hdr.opcode = RPC_HANDSHAKE;
  hdr.request_id = next_id_++;
  hdr.payload_bytes = 0;

  if (!rpc_send_exact(sock_, &hdr, sizeof(hdr)))
    return -1;

  // Receive response header.
  RpcHeader resp = {};
  if (!rpc_recv_exact(sock_, &resp, sizeof(resp)))
    return -1;

  if (resp.result != 0)
    return resp.result;

  // Receive handshake payload.
  RpcHandshakeResponse hs = {};
  if (!rpc_recv_exact(sock_, &hs, sizeof(hs)))
    return -1;

  // Receive topology path string.
  if (hs.topology_path_len > 0) {
    topology_path_.resize(hs.topology_path_len);
    if (!rpc_recv_exact(sock_, topology_path_.data(), hs.topology_path_len))
      return -1;
  }

  int fd = static_cast<int>(syscall(SYS_memfd_create, "rocjitsu_remote_kfd", 0));
  return fd;
}

int RemoteDriver::close() {
  RpcHeader hdr = {};
  hdr.opcode = RPC_CLOSE;
  hdr.request_id = next_id_++;
  hdr.payload_bytes = 0;

  if (!rpc_send_exact(sock_, &hdr, sizeof(hdr)))
    return -1;

  RpcHeader resp = {};
  if (!rpc_recv_exact(sock_, &resp, sizeof(resp)))
    return -1;

  return resp.result;
}

/// @brief Determine the ioctl struct size from the encoded request number.
static size_t ioctl_arg_size(unsigned long request) { return _IOC_SIZE(request); }

/// @brief Check if an ioctl has embedded pointer fields that need inline serialization.
static bool has_embedded_pointers(unsigned long request) {
  switch (request) {
  case AMDKFD_IOC_WAIT_EVENTS:
  case AMDKFD_IOC_MAP_MEMORY_TO_GPU:
  case AMDKFD_IOC_UNMAP_MEMORY_FROM_GPU:
  case AMDKFD_IOC_SVM:
  case AMDKFD_IOC_GET_PROCESS_APERTURES_NEW:
    return true;
  default:
    return false;
  }
}

int RemoteDriver::ioctl(unsigned long request, void *arg) { return send_ioctl(request, arg); }

int RemoteDriver::send_ioctl(unsigned long request, void *arg) {
  size_t arg_size = ioctl_arg_size(request);

  // Build: [RpcHeader] [RpcIoctlRequest] [ioctl args] [optional inlined arrays]
  constexpr size_t prefix = sizeof(RpcHeader) + sizeof(RpcIoctlRequest);
  std::vector<uint8_t> buf(prefix + arg_size);

  std::memcpy(buf.data() + prefix, arg, arg_size);

  // Inline embedded pointer arrays after the ioctl args.
  if (has_embedded_pointers(request)) {
    auto *args_base = buf.data() + prefix;
    switch (request) {
    case AMDKFD_IOC_WAIT_EVENTS: {
      auto *a = reinterpret_cast<kfd_ioctl_wait_events_args *>(args_base);
      size_t n = a->num_events * sizeof(kfd_event_data);
      size_t off = buf.size();
      buf.resize(off + n);
      std::memcpy(buf.data() + off, reinterpret_cast<const void *>(a->events_ptr), n);
      break;
    }
    case AMDKFD_IOC_MAP_MEMORY_TO_GPU:
    case AMDKFD_IOC_UNMAP_MEMORY_FROM_GPU: {
      auto *a = reinterpret_cast<kfd_ioctl_map_memory_to_gpu_args *>(args_base);
      size_t n = a->n_devices * sizeof(uint32_t);
      size_t off = buf.size();
      buf.resize(off + n);
      std::memcpy(buf.data() + off, reinterpret_cast<const void *>(a->device_ids_array_ptr), n);
      break;
    }
    case AMDKFD_IOC_SVM: {
      auto *a = reinterpret_cast<kfd_ioctl_svm_args *>(args_base);
      size_t n = a->nattr * sizeof(kfd_ioctl_svm_attribute);
      size_t off = buf.size();
      buf.resize(off + n);
      std::memcpy(buf.data() + off, a + 1, n);
      break;
    }
    case AMDKFD_IOC_GET_PROCESS_APERTURES_NEW: {
      auto *a = reinterpret_cast<kfd_ioctl_get_process_apertures_new_args *>(args_base);
      buf.resize(buf.size() + a->num_of_nodes * sizeof(kfd_process_device_apertures));
      break;
    }
    default:
      break;
    }
  }

  auto *hdr = reinterpret_cast<RpcHeader *>(buf.data());
  hdr->opcode = RPC_IOCTL;
  hdr->request_id = next_id_++;
  hdr->payload_bytes = static_cast<uint32_t>(buf.size() - sizeof(RpcHeader));
  hdr->result = 0;

  auto *ireq = reinterpret_cast<RpcIoctlRequest *>(buf.data() + sizeof(RpcHeader));
  ireq->ioctl_cmd = static_cast<uint32_t>(request);
  ireq->args_bytes = static_cast<uint32_t>(buf.size() - prefix);

  if (!rpc_send_exact(sock_, buf.data(), buf.size()))
    return -1;

  // Receive response.
  RpcHeader resp = {};
  if (!rpc_recv_exact(sock_, &resp, sizeof(resp)))
    return -1;

  if (resp.payload_bytes > 0) {
    std::vector<uint8_t> payload(resp.payload_bytes);
    if (!rpc_recv_exact(sock_, payload.data(), resp.payload_bytes))
      return -1;

    size_t copy_size = std::min(arg_size, static_cast<size_t>(resp.payload_bytes));
    std::memcpy(arg, payload.data(), copy_size);

    // Copy inlined arrays back to the original embedded pointers.
    if (has_embedded_pointers(request) && resp.payload_bytes > arg_size) {
      size_t extra = resp.payload_bytes - arg_size;
      switch (request) {
      case AMDKFD_IOC_WAIT_EVENTS: {
        auto *a = static_cast<kfd_ioctl_wait_events_args *>(arg);
        std::memcpy(reinterpret_cast<void *>(a->events_ptr), payload.data() + arg_size,
                    std::min(a->num_events * sizeof(kfd_event_data), extra));
        break;
      }
      case AMDKFD_IOC_GET_PROCESS_APERTURES_NEW: {
        auto *a = static_cast<kfd_ioctl_get_process_apertures_new_args *>(arg);
        std::memcpy(reinterpret_cast<void *>(a->kfd_process_device_apertures_ptr),
                    payload.data() + arg_size,
                    std::min(a->num_of_nodes * sizeof(kfd_process_device_apertures), extra));
        break;
      }
      default:
        break;
      }
    }
  }

  return resp.result;
}

void *RemoteDriver::mmap(void *addr, size_t length, int prot, int flags, off_t offset) {
  int memfd = -1;
  int rc = send_mmap(addr, length, prot, flags, offset, &memfd);
  if (rc != 0 || memfd < 0)
    return MAP_FAILED;

  // Map the daemon's memfd locally at the address FMM expects.
  int mflags = MAP_SHARED;
  if (flags & MAP_FIXED)
    mflags |= MAP_FIXED;

  long raw = syscall(SYS_mmap, addr, length, PROT_READ | PROT_WRITE, mflags, memfd, 0);
  void *ptr = (raw < 0) ? MAP_FAILED : reinterpret_cast<void *>(static_cast<uintptr_t>(raw));

  ::close(memfd);
  return ptr;
}

int RemoteDriver::munmap(void *addr, size_t length) {
  RpcMunmapRequest req = {};
  req.addr = reinterpret_cast<uint64_t>(addr);
  req.length = length;

  RpcHeader hdr = {};
  hdr.opcode = RPC_MUNMAP;
  hdr.request_id = next_id_++;
  hdr.payload_bytes = sizeof(req);

  uint8_t buf[sizeof(hdr) + sizeof(req)];
  std::memcpy(buf, &hdr, sizeof(hdr));
  std::memcpy(buf + sizeof(hdr), &req, sizeof(req));

  if (!rpc_send_exact(sock_, buf, sizeof(buf)))
    return -1;

  RpcHeader resp = {};
  if (!rpc_recv_exact(sock_, &resp, sizeof(resp)))
    return -1;

  if (resp.result == 0)
    syscall(SYS_munmap, addr, length);

  return resp.result;
}

int RemoteDriver::send_mmap(void *addr, size_t length, int prot, int flags, off_t offset,
                            int *memfd_out) {
  RpcMmapRequest req = {};
  req.addr = reinterpret_cast<uint64_t>(addr);
  req.length = length;
  req.prot = prot;
  req.flags = flags;
  req.offset = offset;

  RpcHeader hdr = {};
  hdr.opcode = RPC_MMAP;
  hdr.request_id = next_id_++;
  hdr.payload_bytes = sizeof(req);

  uint8_t buf[sizeof(hdr) + sizeof(req)];
  std::memcpy(buf, &hdr, sizeof(hdr));
  std::memcpy(buf + sizeof(hdr), &req, sizeof(req));

  if (!rpc_send_exact(sock_, buf, sizeof(buf)))
    return -1;

  // Receive response with memfd via SCM_RIGHTS.
  uint8_t resp_buf[sizeof(RpcHeader) + sizeof(RpcMmapResponse)];
  int fds[1] = {-1};
  size_t nfds = 1;
  ssize_t n = rpc_recv_msg(sock_, resp_buf, sizeof(resp_buf), fds, &nfds);
  if (n <= 0)
    return -1;

  auto *resp = reinterpret_cast<RpcHeader *>(resp_buf);
  *memfd_out = (nfds > 0) ? fds[0] : -1;
  return resp->result;
}

} // namespace rocjitsu
