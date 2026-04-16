// server.h — Proxy server that dispatches requests to a backend
#pragma once

#include <atomic>
#include <functional>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace amdgpu_proxy {

/// Response from backend, optionally including a file descriptor to pass via SCM_RIGHTS
struct BackendResponse {
    std::vector<uint8_t> data;
    int pass_fd = -1;  // fd to send to client, or -1 if none
};

/// Backend interface — processes RPC requests and returns responses.
/// Implementations: RealHardwareBackend
class IBackend {
public:
    virtual ~IBackend() = default;

    /// Process a raw FlatBuffer RpcMessage request and return a response.
    /// Input: serialized RpcMessage with request payload.
    /// Output: serialized RpcMessage with response payload + optional fd.
    virtual BackendResponse HandleRequest(const uint8_t* data, uint32_t size) = 0;
};

/// ProxyServer listens on a Unix domain socket and dispatches requests
/// to a backend. Uses one thread per client connection.
class ProxyServer {
public:
    explicit ProxyServer(IBackend& backend);
    ~ProxyServer();

    // Non-copyable
    ProxyServer(const ProxyServer&) = delete;
    ProxyServer& operator=(const ProxyServer&) = delete;

    /// Start listening on the given socket path.
    /// Blocks until Stop() is called from another thread.
    bool Start(const std::string& path = "");

    /// Signal the server to stop accepting connections and shut down.
    void Stop();

    /// Returns true if the server is running.
    bool IsRunning() const { return running_.load(); }

private:
    void HandleClient(int client_fd, std::atomic<bool>* done);

    void ReapFinishedThreads();
    void ShutdownAllClients();

    IBackend& backend_;
    int server_fd_ = -1;
    std::atomic<bool> running_{false};
    std::mutex threads_mutex_;
    std::vector<std::thread> client_threads_;
    std::vector<std::atomic<bool>*> client_done_;
    std::mutex client_fds_mutex_;
    std::vector<int> active_client_fds_;
};

} // namespace amdgpu_proxy
