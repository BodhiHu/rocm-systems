// server.cpp — Proxy server implementation
#include "server.h"
#include "transport.h"

#include <unistd.h>
#include <sys/socket.h>
#include <poll.h>
#include <cstring>
#include <algorithm>

namespace amdgpu_proxy {

ProxyServer::ProxyServer(IBackend& backend) : backend_(backend) {}

ProxyServer::~ProxyServer() {
    Stop();
}

bool ProxyServer::Start(const std::string& path) {
    std::string sock_path = path.empty() ? GetSocketPath() : path;
    server_fd_ = CreateServer(sock_path);
    if (server_fd_ < 0) return false;

    running_.store(true);

    while (running_.load()) {
        // Poll with timeout so we can check running_ flag
        struct pollfd pfd{};
        pfd.fd = server_fd_;
        pfd.events = POLLIN;

        int ret = ::poll(&pfd, 1, 500); // 500ms timeout
        if (ret <= 0) continue;

        int client_fd = AcceptClient(server_fd_);
        if (client_fd < 0) continue;

        ReapFinishedThreads();

        try {
            auto* done = new std::atomic<bool>(false);
            std::lock_guard<std::mutex> lock(threads_mutex_);
            client_done_.push_back(done);
            client_threads_.emplace_back(&ProxyServer::HandleClient, this, client_fd, done);
        } catch (const std::system_error& e) {
            fprintf(stderr, "[server] Failed to create client thread: %s\n", e.what());
            ::close(client_fd);
        }
    }

    ::close(server_fd_);
    server_fd_ = -1;

    // Join all client threads
    {
        std::lock_guard<std::mutex> lock(threads_mutex_);
        for (auto& t : client_threads_) {
            if (t.joinable()) t.join();
        }
        for (auto* d : client_done_) delete d;
        client_threads_.clear();
        client_done_.clear();
    }

    return true;
}

void ProxyServer::Stop() {
    running_.store(false);
    if (server_fd_ >= 0) {
        ::shutdown(server_fd_, SHUT_RDWR);
    }
    ShutdownAllClients();
}

void ProxyServer::ShutdownAllClients() {
    std::lock_guard<std::mutex> lock(client_fds_mutex_);
    for (int fd : active_client_fds_) {
        ::shutdown(fd, SHUT_RDWR);
    }
}

void ProxyServer::HandleClient(int client_fd, std::atomic<bool>* done) {
    {
        std::lock_guard<std::mutex> lock(client_fds_mutex_);
        active_client_fds_.push_back(client_fd);
    }

    while (running_.load()) {
        auto request = RecvMessage(client_fd);
        if (request.empty()) break; // Client disconnected

        auto response = backend_.HandleRequest(request.data(),
                                                static_cast<uint32_t>(request.size()));

        bool ok = SendMessageWithFd(client_fd, response.data.data(),
                         static_cast<uint32_t>(response.data.size()),
                         response.pass_fd);

        // Close the passed fd on the server side (kernel dups it for the receiver)
        if (response.pass_fd >= 0) {
            ::close(response.pass_fd);
        }

        if (!ok) break; // Send failed
    }

    {
        std::lock_guard<std::mutex> lock(client_fds_mutex_);
        active_client_fds_.erase(
            std::remove(active_client_fds_.begin(), active_client_fds_.end(), client_fd),
            active_client_fds_.end());
    }
    ::close(client_fd);
    done->store(true);
}

void ProxyServer::ReapFinishedThreads() {
    std::lock_guard<std::mutex> lock(threads_mutex_);
    size_t n = client_threads_.size();
    size_t dst = 0;
    for (size_t i = 0; i < n; i++) {
        if (client_done_[i]->load()) {
            client_threads_[i].join();
            delete client_done_[i];
        } else {
            if (dst != i) {
                client_threads_[dst] = std::move(client_threads_[i]);
                client_done_[dst] = client_done_[i];
            }
            dst++;
        }
    }
    client_threads_.resize(dst);
    client_done_.resize(dst);
}

} // namespace amdgpu_proxy
