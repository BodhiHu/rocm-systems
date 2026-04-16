// client.cpp — Proxy client implementation
#include "client.h"
#include "transport.h"

#include <unistd.h>
#include <sys/syscall.h>

// Use raw syscall to close socket fds, bypassing the LD_PRELOAD shimmed close()
// which would re-enter FdTable locking and potentially deadlock.
static inline int raw_close(int fd) {
    return static_cast<int>(syscall(SYS_close, fd));
}

namespace amdgpu_proxy {

ProxyClient::ProxyClient() = default;

ProxyClient::~ProxyClient() {
    Disconnect();
}

bool ProxyClient::Connect(const std::string& path) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (fd_ >= 0) return true; // already connected

    std::string sock_path = path.empty() ? GetSocketPath() : path;
    fd_ = ConnectToServer(sock_path);
    return fd_ >= 0;
}

void ProxyClient::Disconnect() {
    std::lock_guard<std::mutex> lock(mutex_);
    if (fd_ >= 0) {
        raw_close(fd_);
        fd_ = -1;
    }
}

bool ProxyClient::IsConnected() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return fd_ >= 0;
}

std::vector<uint8_t> ProxyClient::SendRequest(const uint8_t* data, uint32_t size) {
    return SendRequestReceiveFd(data, size, nullptr);
}

std::vector<uint8_t> ProxyClient::SendRequestReceiveFd(const uint8_t* data, uint32_t size, int* received_fd) {
    if (received_fd) *received_fd = -1;

    std::lock_guard<std::mutex> lock(mutex_);
    if (fd_ < 0) return {};

    if (!SendMessage(fd_, data, size)) {
        raw_close(fd_);
        fd_ = -1;
        return {};
    }

    auto response = RecvMessageWithFd(fd_, received_fd);
    if (response.empty()) {
        raw_close(fd_);
        fd_ = -1;
    }
    return response;
}

} // namespace amdgpu_proxy
