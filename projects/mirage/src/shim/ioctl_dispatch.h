// ioctl_dispatch.h — Maps ioctl numbers to FlatBuffer serialization
#pragma once

#include <cstdint>
#include <vector>

namespace amdgpu_proxy {

class ProxyClient;

/// Dispatch a DRM ioctl to the proxy server.
/// Serializes the ioctl args into a FlatBuffer RpcMessage, sends to server,
/// deserializes response, and writes results back to the ioctl arg struct.
///
/// Returns 0 on success, -errno on failure.
int DispatchDrmIoctl(ProxyClient& client, unsigned long request, void* arg, int32_t virtual_fd = -1);

/// Dispatch a KFD ioctl to the proxy server.
/// Returns 0 on success, -errno on failure.
int DispatchKfdIoctl(ProxyClient& client, unsigned long request, void* arg);

/// Dispatch a sysfs file read through the proxy.
/// Fills buffer with up to max_size bytes. Returns bytes read or -errno.
int DispatchSysfsRead(ProxyClient& client, const char* path,
                      void* buffer, uint32_t max_size);

} // namespace amdgpu_proxy
