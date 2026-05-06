// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

/// @file main.cpp
/// @brief rocjitsu daemon — hosts the GPU simulator and serves KFD ioctls to
///        client processes connected via a Unix domain socket.

#include "rocjitsu/kmd/linux/rpc.h"
#include "rocjitsu/kmd/linux/simulated_driver.h"

#include "rocjitsu/base/rj_compiler.h"
RJ_DIAGNOSTIC_PUSH
RJ_DIAGNOSTIC_IGNORE_PEDANTIC
#include "linux/uapi/kfd_ioctl.h"
RJ_DIAGNOSTIC_POP

#include <algorithm>
#include <cerrno>
#include <csignal>
#include <cstring>
#include <filesystem>
#include <format>
#include <string>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <thread>
#include <unistd.h>
#include <vector>

using namespace rocjitsu;

static std::atomic<bool> g_running{true};

/// @brief Handle a single client connection on a dedicated thread.
/// @details Reads RPC requests from the client socket, dispatches them to
///          the SimulatedDriver, and sends results back. Runs until the client
///          disconnects or the daemon shuts down.
static void handle_client(int client_fd, SimulatedDriver *driver) {
  while (g_running) {
    RpcHeader hdr = {};
    if (!rpc_recv_exact(client_fd, &hdr, sizeof(hdr)))
      break;

    switch (hdr.opcode) {
    case RPC_HANDSHAKE: {
      RpcHeader resp = {};
      resp.request_id = hdr.request_id;
      resp.result = 0;

      RpcHandshakeResponse hs = {};
      hs.gpu_id = 0;
      hs.kfd_fd = SimulatedDriver::kfd_fd();
      auto topo = driver->topology_path();
      hs.topology_path_len = static_cast<uint32_t>(topo.size());

      resp.payload_bytes = sizeof(hs) + hs.topology_path_len;
      rpc_send_exact(client_fd, &resp, sizeof(resp));
      rpc_send_exact(client_fd, &hs, sizeof(hs));
      if (hs.topology_path_len > 0)
        rpc_send_exact(client_fd, topo.data(), topo.size());
      break;
    }

    case RPC_CLOSE: {
      RpcHeader resp = {};
      resp.request_id = hdr.request_id;
      resp.result = 0;
      resp.payload_bytes = 0;
      rpc_send_exact(client_fd, &resp, sizeof(resp));
      goto done;
    }

    case RPC_MMAP: {
      RpcMmapRequest mreq = {};
      if (!rpc_recv_exact(client_fd, &mreq, sizeof(mreq)))
        goto done;

      void *result = driver->mmap(reinterpret_cast<void *>(mreq.addr), mreq.length, mreq.prot,
                                  mreq.flags, static_cast<off_t>(mreq.offset));

      RpcHeader resp = {};
      resp.request_id = hdr.request_id;
      resp.result = (result == MAP_FAILED) ? -errno : 0;
      resp.payload_bytes = sizeof(RpcMmapResponse);

      RpcMmapResponse mresp = {};
      mresp.mapped_addr = reinterpret_cast<uint64_t>(result);

      uint8_t buf[sizeof(resp) + sizeof(mresp)];
      std::memcpy(buf, &resp, sizeof(resp));
      std::memcpy(buf + sizeof(resp), &mresp, sizeof(mresp));
      // TODO: send the backing memfd via SCM_RIGHTS for shared access.
      rpc_send_exact(client_fd, buf, sizeof(buf));
      break;
    }

    case RPC_MUNMAP: {
      RpcMunmapRequest mreq = {};
      if (!rpc_recv_exact(client_fd, &mreq, sizeof(mreq)))
        goto done;

      int rc = driver->munmap(reinterpret_cast<void *>(mreq.addr), mreq.length);
      RpcHeader resp = {};
      resp.request_id = hdr.request_id;
      resp.result = rc;
      resp.payload_bytes = 0;
      rpc_send_exact(client_fd, &resp, sizeof(resp));
      break;
    }

    case RPC_IOCTL: {
      std::vector<uint8_t> payload(hdr.payload_bytes);
      if (!rpc_recv_exact(client_fd, payload.data(), hdr.payload_bytes))
        goto done;

      auto *ireq = reinterpret_cast<RpcIoctlRequest *>(payload.data());
      size_t arg_size = _IOC_SIZE(ireq->ioctl_cmd);
      void *arg = payload.data() + sizeof(RpcIoctlRequest);
      size_t args_total = ireq->args_bytes;

      // Reconstruct embedded pointers to point at inlined arrays.
      switch (ireq->ioctl_cmd) {
      case AMDKFD_IOC_WAIT_EVENTS: {
        auto *a = reinterpret_cast<kfd_ioctl_wait_events_args *>(arg);
        if (args_total > arg_size)
          a->events_ptr = reinterpret_cast<uint64_t>(static_cast<uint8_t *>(arg) + arg_size);
        break;
      }
      case AMDKFD_IOC_MAP_MEMORY_TO_GPU:
      case AMDKFD_IOC_UNMAP_MEMORY_FROM_GPU: {
        auto *a = reinterpret_cast<kfd_ioctl_map_memory_to_gpu_args *>(arg);
        if (args_total > arg_size)
          a->device_ids_array_ptr =
              reinterpret_cast<uint64_t>(static_cast<uint8_t *>(arg) + arg_size);
        break;
      }
      case AMDKFD_IOC_GET_PROCESS_APERTURES_NEW: {
        auto *a = reinterpret_cast<kfd_ioctl_get_process_apertures_new_args *>(arg);
        if (args_total > arg_size)
          a->kfd_process_device_apertures_ptr =
              reinterpret_cast<uint64_t>(static_cast<uint8_t *>(arg) + arg_size);
        break;
      }
      default:
        break;
      }

      int rc = driver->ioctl(ireq->ioctl_cmd, arg);

      RpcHeader resp = {};
      resp.opcode = RPC_IOCTL;
      resp.request_id = hdr.request_id;
      resp.result = rc;
      resp.payload_bytes = static_cast<uint32_t>(args_total);
      rpc_send_exact(client_fd, &resp, sizeof(resp));
      if (args_total > 0)
        rpc_send_exact(client_fd, arg, args_total);
      break;
    }

    default:
      goto done;
    }
  }

done:
  ::close(client_fd);
}

int main(int argc, char *argv[]) {
  (void)argc;
  (void)argv;

  std::signal(SIGINT, [](int) { g_running = false; });
  std::signal(SIGTERM, [](int) { g_running = false; });
  std::signal(SIGPIPE, SIG_IGN);

  auto *driver = SimulatedDriver::get_or_create();
  if (!driver)
    return 1;

  auto sock_path = rpc_default_socket_path();
  std::filesystem::create_directories(std::filesystem::path(sock_path).parent_path());
  unlink(sock_path.c_str());

  int listen_fd = socket(AF_UNIX, SOCK_STREAM, 0);
  if (listen_fd < 0)
    return 1;

  struct sockaddr_un addr = {};
  addr.sun_family = AF_UNIX;
  std::strncpy(addr.sun_path, sock_path.c_str(), sizeof(addr.sun_path) - 1);

  if (bind(listen_fd, reinterpret_cast<struct sockaddr *>(&addr), sizeof(addr)) != 0 ||
      listen(listen_fd, 16) != 0) {
    ::close(listen_fd);
    return 1;
  }

  while (g_running) {
    int client = accept(listen_fd, nullptr, nullptr);
    if (client < 0) {
      if (errno == EINTR)
        continue;
      break;
    }
    std::thread(handle_client, client, driver).detach();
  }

  ::close(listen_fd);
  unlink(sock_path.c_str());
  return 0;
}
