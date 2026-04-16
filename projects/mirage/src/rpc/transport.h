// transport.h — Unix domain socket transport with length-prefixed framing
#pragma once

#include <cstdint>
#include <cstddef>
#include <string>
#include <vector>
#include <mutex>

namespace amdgpu_proxy {

/// Default socket path (overridden by AMDGPU_PROXY_SOCK env var)
constexpr const char* kDefaultSocketPath = "/tmp/amdgpu-proxy.sock";

/// Get the configured socket path from environment or default
std::string GetSocketPath();

/// Send a length-prefixed message: [4-byte LE size][payload]
/// Returns true on success. Thread-safe if protected externally.
bool SendMessage(int fd, const uint8_t* data, uint32_t size);

/// Send a length-prefixed message with an optional file descriptor via SCM_RIGHTS.
/// If pass_fd >= 0, the fd is sent as ancillary data with the first sendmsg.
bool SendMessageWithFd(int fd, const uint8_t* data, uint32_t size, int pass_fd);

/// Receive a length-prefixed message.
/// Returns the payload, or empty vector on error/disconnect.
std::vector<uint8_t> RecvMessage(int fd);

/// Receive a length-prefixed message, optionally receiving a file descriptor.
/// If a fd was passed, *received_fd is set to it; otherwise -1.
std::vector<uint8_t> RecvMessageWithFd(int fd, int* received_fd);

/// Create a Unix domain socket server, bind, and listen.
/// Returns the listening socket fd, or -1 on error.
int CreateServer(const std::string& path);

/// Accept a client connection. Returns client fd or -1.
int AcceptClient(int server_fd);

/// Connect to a Unix domain socket server.
/// Returns the connected fd, or -1 on error.
int ConnectToServer(const std::string& path);

} // namespace amdgpu_proxy
