// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#ifndef ROCJITSU_KMD_LINUX_RPC_H_
#define ROCJITSU_KMD_LINUX_RPC_H_

/// @file rpc.h
/// @brief RPC format for the rocjitsu daemon ↔ client Unix socket protocol.
///
/// @details All messages are length-prefixed. The daemon and client exchange
/// RpcHeader/RpcResponse headers followed by ioctl struct payloads. For
/// ioctls with embedded pointers, the pointed-to arrays are serialized inline
/// after the struct. File descriptors (memfds for GPU memory) are passed via
/// SCM_RIGHTS ancillary messages alongside the response.

#include <cstdint>
#include <cstring>
#include <string>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/un.h>
#include <unistd.h>

namespace rocjitsu {

/// @brief RPC operation codes.
enum RpcOpcode : uint16_t {
  RPC_HANDSHAKE,
  RPC_OPEN,
  RPC_CLOSE,
  RPC_MMAP,
  RPC_MUNMAP,
  RPC_IOCTL,
};

/// @brief Unified RPC message header (16 bytes, fixed size).
/// @details Used for both requests and responses. For requests, `result` is 0.
///          For responses, `result` carries the return code from the daemon.
struct RpcHeader {
  uint16_t opcode;        ///< RpcOpcode value.
  uint16_t reserved;      ///< Reserved for future use (must be 0).
  uint32_t request_id;    ///< Correlates request to response.
  uint32_t payload_bytes; ///< Size of the payload in bytes following this header.
  int32_t result; ///< 0 for requests; return code for responses (negative errno on failure).
};

/// @brief Handshake response payload (sent after RPC_HANDSHAKE).
struct RpcHandshakeResponse {
  uint32_t gpu_id;
  int32_t kfd_fd;
  uint32_t topology_path_len; ///< Length of the topology path string that follows.
};

/// @brief Ioctl request payload (when opcode == RPC_IOCTL).
/// @details Followed by the raw ioctl args as opaque bytes. For ioctls with
/// embedded pointers, the pointed-to arrays are inlined after the args.
struct RpcIoctlRequest {
  uint32_t ioctl_cmd;  ///< AMDKFD_IOC_* ioctl number.
  uint32_t args_bytes; ///< Size of the ioctl args (and any inlined arrays) that follow.
};

/// @brief mmap request payload.
struct RpcMmapRequest {
  uint64_t addr;
  uint64_t length;
  int32_t prot;
  int32_t flags;
  int64_t offset;
};

/// @brief mmap response payload.
struct RpcMmapResponse {
  uint64_t mapped_addr; ///< Address the daemon mapped at (informational).
};

/// @brief munmap request payload.
struct RpcMunmapRequest {
  uint64_t addr;
  uint64_t length;
};

/// @brief Send a message with optional ancillary file descriptors.
/// @returns Number of bytes sent, or -1 on error.
inline ssize_t rpc_send_msg(int sock, const void *data, size_t len, const int *fds = nullptr,
                            size_t nfds = 0) {
  struct iovec iov = {const_cast<void *>(data), len};
  struct msghdr msg = {};
  msg.msg_iov = &iov;
  msg.msg_iovlen = 1;

  union {
    struct cmsghdr align;
    char buf[CMSG_SPACE(sizeof(int) * 4)];
  } cmsg_buf = {};

  if (fds && nfds > 0) {
    msg.msg_control = cmsg_buf.buf;
    msg.msg_controllen = CMSG_SPACE(nfds * sizeof(int));
    auto *cmsg = CMSG_FIRSTHDR(&msg);
    cmsg->cmsg_level = SOL_SOCKET;
    cmsg->cmsg_type = SCM_RIGHTS;
    cmsg->cmsg_len = CMSG_LEN(nfds * sizeof(int));
    std::memcpy(CMSG_DATA(cmsg), fds, nfds * sizeof(int));
  }

  return sendmsg(sock, &msg, MSG_NOSIGNAL);
}

/// @brief Receive a message with optional ancillary file descriptors.
/// @param[out] fds Buffer for received file descriptors.
/// @param[in,out] nfds On input: capacity. On output: number received.
/// @returns Number of bytes received, or -1 on error.
inline ssize_t rpc_recv_msg(int sock, void *data, size_t len, int *fds = nullptr,
                            size_t *nfds = nullptr) {
  struct iovec iov = {data, len};
  struct msghdr msg = {};
  msg.msg_iov = &iov;
  msg.msg_iovlen = 1;

  union {
    struct cmsghdr align;
    char buf[CMSG_SPACE(sizeof(int) * 4)];
  } cmsg_buf = {};

  if (fds && nfds && *nfds > 0) {
    msg.msg_control = cmsg_buf.buf;
    msg.msg_controllen = sizeof(cmsg_buf.buf);
  }

  ssize_t n = recvmsg(sock, &msg, 0);
  if (n <= 0) {
    if (nfds)
      *nfds = 0;
    return n;
  }

  size_t fd_count = 0;
  if (fds && nfds) {
    for (auto *cmsg = CMSG_FIRSTHDR(&msg); cmsg; cmsg = CMSG_NXTHDR(&msg, cmsg)) {
      if (cmsg->cmsg_level == SOL_SOCKET && cmsg->cmsg_type == SCM_RIGHTS) {
        size_t payload = cmsg->cmsg_len - CMSG_LEN(0);
        size_t count = payload / sizeof(int);
        if (fd_count + count <= *nfds)
          std::memcpy(fds + fd_count, CMSG_DATA(cmsg), count * sizeof(int));
        fd_count += count;
      }
    }
    *nfds = fd_count;
  }

  return n;
}

/// @brief Read exactly `len` bytes from a socket, handling partial reads.
inline bool rpc_recv_exact(int sock, void *buf, size_t len) {
  auto *p = static_cast<uint8_t *>(buf);
  while (len > 0) {
    ssize_t n = recv(sock, p, len, 0);
    if (n <= 0)
      return false;
    p += n;
    len -= static_cast<size_t>(n);
  }
  return true;
}

/// @brief Send exactly `len` bytes to a socket, handling partial writes.
inline bool rpc_send_exact(int sock, const void *buf, size_t len) {
  auto *p = static_cast<const uint8_t *>(buf);
  while (len > 0) {
    ssize_t n = send(sock, p, len, MSG_NOSIGNAL);
    if (n <= 0)
      return false;
    p += n;
    len -= static_cast<size_t>(n);
  }
  return true;
}

/// @brief Default daemon socket path for the current user.
/// @details Uses $XDG_RUNTIME_DIR (standard per-user runtime directory) if
/// available, falling back to /tmp/rocjitsu-<uid> otherwise.
inline std::string rpc_default_socket_path() {
  if (const char *xdg = getenv("XDG_RUNTIME_DIR"))
    return std::string(xdg) + "/rocjitsu/daemon.sock";
  return "/tmp/rocjitsu-" + std::to_string(getuid()) + "/daemon.sock";
}

} // namespace rocjitsu

#endif // ROCJITSU_KMD_LINUX_RPC_H_
