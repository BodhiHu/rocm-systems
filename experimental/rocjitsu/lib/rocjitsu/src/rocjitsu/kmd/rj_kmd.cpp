// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

#include "rocjitsu/kmd/rj_kmd.h"
#include "rocjitsu/kmd/linux/simulated_driver.h"

#include <memory>
#include <string>

struct rj_kmd_t {
  std::unique_ptr<rocjitsu::SimulatedDriver> driver;
  std::string topology_path_cache;
};

extern "C" {

rj_status_t rj_kmd_create_default(rj_kmd_t **out) {
  if (!out)
    return ROCJITSU_STATUS_INVALID_ARGUMENT;
  try {
    auto handle = std::make_unique<rj_kmd_t>();
    handle->driver = rocjitsu::SimulatedDriver::create_default();
    if (!handle->driver)
      return ROCJITSU_STATUS_ERROR;
    handle->topology_path_cache = handle->driver->topology_path();
    *out = handle.release();
    return ROCJITSU_STATUS_SUCCESS;
  } catch (...) {
    *out = nullptr;
    return ROCJITSU_STATUS_ERROR;
  }
}

rj_status_t rj_kmd_open(rj_kmd_t *driver, int *fd) {
  if (!driver || !fd)
    return ROCJITSU_STATUS_INVALID_ARGUMENT;
  int result = driver->driver->open();
  if (result < 0)
    return ROCJITSU_STATUS_ERROR;
  *fd = result;
  return ROCJITSU_STATUS_SUCCESS;
}

rj_status_t rj_kmd_close(rj_kmd_t *driver) {
  if (!driver)
    return ROCJITSU_STATUS_INVALID_ARGUMENT;
  driver->driver->close();
  return ROCJITSU_STATUS_SUCCESS;
}

rj_status_t rj_kmd_ioctl(rj_kmd_t *driver, unsigned long request, void *arg,
                         int *result) {
  if (!driver)
    return ROCJITSU_STATUS_INVALID_ARGUMENT;
  int rc = driver->driver->ioctl(request, arg);
  if (result)
    *result = rc;
  return ROCJITSU_STATUS_SUCCESS;
}

rj_status_t rj_kmd_mmap(rj_kmd_t *driver, void *addr, size_t length, int prot,
                        int flags, int64_t offset, void **result) {
  if (!driver || !result)
    return ROCJITSU_STATUS_INVALID_ARGUMENT;
  *result = driver->driver->mmap(addr, length, prot, flags,
                                 static_cast<off_t>(offset));
  return ROCJITSU_STATUS_SUCCESS;
}

rj_status_t rj_kmd_munmap(rj_kmd_t *driver, void *addr, size_t length,
                          int *result) {
  if (!driver)
    return ROCJITSU_STATUS_INVALID_ARGUMENT;
  int rc = driver->driver->munmap(addr, length);
  if (result)
    *result = rc;
  return ROCJITSU_STATUS_SUCCESS;
}

int rj_kmd_fd(const rj_kmd_t *driver) {
  if (!driver)
    return -1;
  // The SimulatedDriver stores the fd internally; return -1 for now
  // since there is no public accessor. The fd is set during open().
  // Access the static kfd_fd() which reads the atomic.
  return rocjitsu::SimulatedDriver::kfd_fd();
}

const char *rj_kmd_topology_path(const rj_kmd_t *driver) {
  if (!driver)
    return nullptr;
  return driver->topology_path_cache.c_str();
}

void rj_kmd_destroy(rj_kmd_t *driver) { delete driver; }

} // extern "C"
