// Copyright (c) 2026 Advanced Micro Devices, Inc.
// SPDX-License-Identifier: MIT

/// @file rj_kmd.h
/// @brief Public C API for the rocjitsu simulated kernel-mode driver.
///
/// @details Exposes the SimulatedDriver through a C-linkage interface so that
/// Rust (via FFI) and other languages can create a driver, issue KFD ioctls,
/// and manage mmap/munmap — exactly the same surface the real KFD provides
/// through `/dev/kfd`.

#ifndef ROCJITSU_KMD_RJ_KMD_H_
#define ROCJITSU_KMD_RJ_KMD_H_

#include "rocjitsu/base/rj_compiler.h"
#include "rocjitsu/base/rj_status.h"

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// @addtogroup kmd
/// @{

/// @brief Opaque handle to a simulated kernel-mode driver.
typedef struct rj_kmd_t rj_kmd_t;

/// @brief Create a simulated driver from `RJ_CONFIG` / `RJ_SCHEMA` env vars.
///
/// @details Parses the config, constructs the SoC + simulation engine, starts
/// the engine background thread, and generates the sysfs topology tree. The
/// driver is *not* open yet — call rj_kmd_open() before issuing ioctls.
///
/// @param[out] driver The newly created driver handle.
/// @retval ROCJITSU_STATUS_SUCCESS Driver was created successfully.
/// @retval ROCJITSU_STATUS_INVALID_ARGUMENT @p driver is NULL.
/// @retval ROCJITSU_STATUS_ERROR Construction failed (env vars missing, bad
///         config, etc.). An error message is logged to stderr.
RJ_API_EXPORT rj_status_t rj_kmd_create_default(rj_kmd_t **driver);

/// @brief Create a simulated driver from explicit config and schema paths.
///
/// @details Equivalent to rj_kmd_create_default, but avoids process-wide
/// environment variables for callers that manage multiple simulator instances.
/// The driver is *not* open yet; call rj_kmd_open() before issuing ioctls.
///
/// @param[in]  config_path Path to a rocjitsu simulation config JSON file.
/// @param[in]  schema_path Path to simulation_config.fbs.
/// @param[out] driver      The newly created driver handle.
/// @retval ROCJITSU_STATUS_SUCCESS Driver was created successfully.
/// @retval ROCJITSU_STATUS_INVALID_ARGUMENT Any pointer argument is NULL.
/// @retval ROCJITSU_STATUS_ERROR Construction failed.
RJ_API_EXPORT rj_status_t rj_kmd_create(const char *config_path,
                                        const char *schema_path,
                                        rj_kmd_t **driver);

/// @brief Open the simulated KFD device.
///
/// @details Allocates a synthetic file descriptor (via memfd_create), registers
/// the driver as a primary with the simulation engine, and wires the interrupt
/// callback. Must be called before issuing ioctls.
///
/// @param[in]  driver Driver handle.
/// @param[out] fd     The synthetic KFD file descriptor.
/// @retval ROCJITSU_STATUS_SUCCESS Driver opened successfully.
/// @retval ROCJITSU_STATUS_INVALID_ARGUMENT @p driver or @p fd is NULL.
/// @retval ROCJITSU_STATUS_ERROR open() failed.
RJ_API_EXPORT rj_status_t rj_kmd_open(rj_kmd_t *driver, int *fd);

/// @brief Close the simulated KFD device.
///
/// @details Tears down queues, frees allocations, and deregisters from the
/// engine. The driver handle remains valid — it can be re-opened.
///
/// @param[in] driver Driver handle.
/// @retval ROCJITSU_STATUS_SUCCESS Driver closed successfully.
/// @retval ROCJITSU_STATUS_INVALID_ARGUMENT @p driver is NULL.
RJ_API_EXPORT rj_status_t rj_kmd_close(rj_kmd_t *driver);

/// @brief Dispatch a KFD ioctl to the simulated driver.
///
/// @details Accepts the same ioctl command numbers and argument structs as the
/// real `/dev/kfd` kernel interface. The driver processes the request entirely
/// in userspace through the simulation engine.
///
/// @param[in]     driver  Driver handle.
/// @param[in]     request The ioctl command number (e.g. AMDKFD_IOC_GET_VERSION).
/// @param[in,out] arg     Pointer to the ioctl argument struct (same layout as
///                        the kernel expects).
/// @param[out]    result  The ioctl return value (0 on success, negative errno
///                        on failure). May be NULL if the caller only cares
///                        about the rj_status_t.
/// @retval ROCJITSU_STATUS_SUCCESS The ioctl was dispatched (check @p result
///         for the ioctl-level outcome).
/// @retval ROCJITSU_STATUS_INVALID_ARGUMENT @p driver is NULL.
RJ_API_EXPORT rj_status_t rj_kmd_ioctl(rj_kmd_t *driver, unsigned long request,
                                       void *arg, int *result);

/// @brief Map device memory through the simulated driver.
///
/// @param[in]  driver Driver handle.
/// @param[in]  addr   Requested address (or NULL).
/// @param[in]  length Length of the mapping.
/// @param[in]  prot   Protection flags (PROT_READ, PROT_WRITE, …).
/// @param[in]  flags  Mapping flags (MAP_SHARED, MAP_FIXED, …).
/// @param[in]  offset Mmap offset (encodes doorbell/event page/handle).
/// @param[out] result The mapped address, or MAP_FAILED.
/// @retval ROCJITSU_STATUS_SUCCESS mmap dispatched (check @p result).
/// @retval ROCJITSU_STATUS_INVALID_ARGUMENT @p driver or @p result is NULL.
RJ_API_EXPORT rj_status_t rj_kmd_mmap(rj_kmd_t *driver, void *addr,
                                      size_t length, int prot, int flags,
                                      int64_t offset, void **result);

/// @brief Unmap previously mapped device memory.
///
/// @param[in]  driver Driver handle.
/// @param[in]  addr   Address to unmap.
/// @param[in]  length Length of the mapping.
/// @param[out] result The munmap return value (0 or negative errno). May be NULL.
/// @retval ROCJITSU_STATUS_SUCCESS munmap dispatched.
/// @retval ROCJITSU_STATUS_INVALID_ARGUMENT @p driver is NULL.
RJ_API_EXPORT rj_status_t rj_kmd_munmap(rj_kmd_t *driver, void *addr,
                                        size_t length, int *result);

/// @brief Get the synthetic KFD file descriptor.
///
/// @param[in] driver Driver handle.
/// @returns The fd, or -1 if the driver is not open.
RJ_API_EXPORT int rj_kmd_fd(const rj_kmd_t *driver);

/// @brief Get the generated sysfs topology directory path.
///
/// @details The returned string is owned by the driver and valid until the
/// driver is destroyed.
///
/// @param[in] driver Driver handle.
/// @returns NUL-terminated path, or NULL if the driver has no topology.
RJ_API_EXPORT const char *rj_kmd_topology_path(const rj_kmd_t *driver);

/// @brief Destroy a simulated driver handle.
///
/// @details Closes the driver if still open, then frees all resources. After
/// this call @p driver is invalid.
///
/// @param[in] driver Driver handle (may be NULL — no-op).
RJ_API_EXPORT void rj_kmd_destroy(rj_kmd_t *driver);

/// @}

#ifdef __cplusplus
} // extern "C"
#endif

#endif // ROCJITSU_KMD_RJ_KMD_H_
