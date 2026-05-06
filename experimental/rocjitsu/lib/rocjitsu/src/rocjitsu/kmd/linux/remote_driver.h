// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#ifndef ROCJITSU_KMD_LINUX_REMOTE_DRIVER_H_
#define ROCJITSU_KMD_LINUX_REMOTE_DRIVER_H_

/// @file remote_driver.h
/// @brief Client-side RPC stub that forwards KFD ioctls to the rocjitsu daemon.
///
/// @details Implements the Driver interface by serializing ioctl requests over
/// a Unix domain socket to the daemon process. GPU memory is shared via memfds
/// passed through SCM_RIGHTS. The client mmaps these memfds locally at the
/// addresses ROCR's FMM expects.

#include "rocjitsu/vm/driver.h"

#include <atomic>
#include <cstdint>
#include <string>

namespace rocjitsu {

/// @brief Client-side driver that forwards ioctls to the rocjitsu daemon.
///
/// @details Manages a singleton connection to the daemon process. The
/// interposer calls get_or_create() on the first open("/dev/kfd") to establish
/// the connection. Subsequent ioctl/mmap/munmap calls are serialized over the
/// socket using the wire protocol defined in wire_protocol.h.
class RemoteDriver : public Driver {
public:
  /// @brief Get or lazily create the daemon connection singleton.
  /// @details Connects to the daemon socket, performs the RPC handshake, and
  /// stores the singleton. Thread-safe via atomic pointer.
  /// @returns Pointer to the remote driver, or nullptr if no daemon is running.
  static RemoteDriver *get_or_create();

  /// @brief Look up the remote driver by its KFD file descriptor.
  /// @param fd The file descriptor to check.
  /// @returns Pointer to the driver if fd matches the remote KFD fd, else nullptr.
  static RemoteDriver *lookup(int fd);

  /// @brief Get the KFD fd for the remote driver (-1 if not connected).
  static int kfd_fd();

  /// @brief Get the daemon's sysfs topology path (empty if not connected).
  static std::string topology_path();

  ~RemoteDriver() override;

  int open() override;
  int close() override;
  int ioctl(unsigned long request, void *arg) override;
  void *mmap(void *addr, size_t length, int prot, int flags, off_t offset) override;
  int munmap(void *addr, size_t length) override;

private:
  /// @brief Construct from an already-connected Unix socket fd.
  explicit RemoteDriver(int sock_fd);

  /// @brief Serialize and send an ioctl request, receive the response.
  int send_ioctl(unsigned long request, void *arg);

  /// @brief Send an mmap request and receive the backing memfd via SCM_RIGHTS.
  int send_mmap(void *addr, size_t length, int prot, int flags, off_t offset, int *memfd_out);

  static std::atomic<RemoteDriver *> instance_; ///< Singleton instance.
  static std::atomic<int> kfd_fd_;              ///< Synthetic KFD fd returned to ROCR.

  int sock_ = -1;             ///< Unix socket connection to the daemon.
  uint32_t next_id_ = 1;      ///< Monotonic request ID counter.
  std::string topology_path_; ///< Daemon's sysfs topology directory path.
};

} // namespace rocjitsu

#endif // ROCJITSU_KMD_LINUX_REMOTE_DRIVER_H_
