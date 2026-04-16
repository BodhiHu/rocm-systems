// client.h — Proxy client for sending RPC requests to the server
#pragma once

#include <cstdint>
#include <mutex>
#include <string>
#include <vector>
#include <atomic>

namespace amdgpu_proxy {

/// ProxyClient connects to the proxy server over a Unix domain socket
/// and sends FlatBuffer-serialized RPC requests, receiving responses.
/// Thread-safe: all public methods are synchronized.
class ProxyClient {
public:
    ProxyClient();
    ~ProxyClient();

    // Non-copyable
    ProxyClient(const ProxyClient&) = delete;
    ProxyClient& operator=(const ProxyClient&) = delete;

    /// Connect to the proxy server at the given socket path.
    /// If path is empty, uses GetSocketPath().
    bool Connect(const std::string& path = "");

    /// Disconnect from the server.
    void Disconnect();

    /// Returns true if connected.
    bool IsConnected() const;

    /// Send a raw FlatBuffer request and receive the raw response.
    /// Returns the response bytes, or empty vector on error.
    std::vector<uint8_t> SendRequest(const uint8_t* data, uint32_t size);

    /// Send a request and receive response, optionally receiving a file descriptor
    /// passed via SCM_RIGHTS. If received_fd is non-null, *received_fd will be set
    /// to the received fd or -1 if none was passed.
    std::vector<uint8_t> SendRequestReceiveFd(const uint8_t* data, uint32_t size, int* received_fd);

    /// Get the socket file descriptor (for poll/select).
    int GetFd() const { return fd_; }

private:
    int fd_ = -1;
    mutable std::mutex mutex_;
    std::atomic<uint64_t> next_request_id_{1};
};

} // namespace amdgpu_proxy
