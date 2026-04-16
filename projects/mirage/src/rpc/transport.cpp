// transport.cpp — Unix domain socket transport with length-prefixed framing
#include "transport.h"

#include <cerrno>
#include <cstdlib>
#include <cstring>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/un.h>

namespace amdgpu_proxy {

std::string GetSocketPath() {
    const char* env = std::getenv("AMDGPU_PROXY_SOCK");
    return env ? std::string(env) : std::string(kDefaultSocketPath);
}

static bool SendAll(int fd, const void* buf, size_t len) {
    const uint8_t* p = static_cast<const uint8_t*>(buf);
    while (len > 0) {
        ssize_t n = ::send(fd, p, len, MSG_NOSIGNAL);
        if (n <= 0) {
            if (n < 0 && errno == EINTR) continue;
            return false;
        }
        p += n;
        len -= static_cast<size_t>(n);
    }
    return true;
}

static bool RecvAll(int fd, void* buf, size_t len) {
    uint8_t* p = static_cast<uint8_t*>(buf);
    while (len > 0) {
        ssize_t n = ::recv(fd, p, len, 0);
        if (n <= 0) {
            if (n < 0 && errno == EINTR) continue;
            return false;
        }
        p += n;
        len -= static_cast<size_t>(n);
    }
    return true;
}

bool SendMessage(int fd, const uint8_t* data, uint32_t size) {
    return SendMessageWithFd(fd, data, size, -1);
}

bool SendMessageWithFd(int fd, const uint8_t* data, uint32_t size, int pass_fd) {
    // Send 4-byte little-endian size prefix
    uint8_t header[4];
    header[0] = static_cast<uint8_t>(size & 0xFF);
    header[1] = static_cast<uint8_t>((size >> 8) & 0xFF);
    header[2] = static_cast<uint8_t>((size >> 16) & 0xFF);
    header[3] = static_cast<uint8_t>((size >> 24) & 0xFF);

    if (pass_fd >= 0) {
        // Send header + ancillary fd in one sendmsg
        struct iovec iov{};
        iov.iov_base = header;
        iov.iov_len = 4;

        union {
            struct cmsghdr cm;
            char buf[CMSG_SPACE(sizeof(int))];
        } cmsg_buf{};

        struct msghdr msg{};
        msg.msg_iov = &iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg_buf.buf;
        msg.msg_controllen = sizeof(cmsg_buf.buf);

        struct cmsghdr* cmsg = CMSG_FIRSTHDR(&msg);
        cmsg->cmsg_level = SOL_SOCKET;
        cmsg->cmsg_type = SCM_RIGHTS;
        cmsg->cmsg_len = CMSG_LEN(sizeof(int));
        std::memcpy(CMSG_DATA(cmsg), &pass_fd, sizeof(int));

        ssize_t n = ::sendmsg(fd, &msg, MSG_NOSIGNAL);
        if (n < 0) return false;
    } else {
        if (!SendAll(fd, header, 4)) return false;
    }

    if (size > 0 && !SendAll(fd, data, size)) return false;
    return true;
}

std::vector<uint8_t> RecvMessage(int fd) {
    return RecvMessageWithFd(fd, nullptr);
}

std::vector<uint8_t> RecvMessageWithFd(int fd, int* received_fd) {
    if (received_fd) *received_fd = -1;

    // Receive 4-byte header, possibly with ancillary fd
    uint8_t header[4];

    if (received_fd) {
        struct iovec iov{};
        iov.iov_base = header;
        iov.iov_len = 4;

        union {
            struct cmsghdr cm;
            char buf[CMSG_SPACE(sizeof(int))];
        } cmsg_buf{};

        struct msghdr msg{};
        msg.msg_iov = &iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg_buf.buf;
        msg.msg_controllen = sizeof(cmsg_buf.buf);

        ssize_t n;
        do {
            n = ::recvmsg(fd, &msg, MSG_CMSG_CLOEXEC);
        } while (n < 0 && errno == EINTR);

        if (n <= 0) return {};

        // Extract ancillary fd if present
        for (struct cmsghdr* cmsg = CMSG_FIRSTHDR(&msg); cmsg; cmsg = CMSG_NXTHDR(&msg, cmsg)) {
            if (cmsg->cmsg_level == SOL_SOCKET && cmsg->cmsg_type == SCM_RIGHTS) {
                std::memcpy(received_fd, CMSG_DATA(cmsg), sizeof(int));
            }
        }

        // If recvmsg didn't deliver all 4 header bytes, recv the rest
        if (n < 4) {
            if (!RecvAll(fd, header + n, 4 - static_cast<size_t>(n))) return {};
        }
    } else {
        if (!RecvAll(fd, header, 4)) return {};
    }

    uint32_t size = static_cast<uint32_t>(header[0])
                  | (static_cast<uint32_t>(header[1]) << 8)
                  | (static_cast<uint32_t>(header[2]) << 16)
                  | (static_cast<uint32_t>(header[3]) << 24);

    if (size == 0) return {};
    // Sanity limit: 64 MiB
    if (size > 64 * 1024 * 1024) return {};

    std::vector<uint8_t> buf(size);
    if (!RecvAll(fd, buf.data(), size)) return {};
    return buf;
}

int CreateServer(const std::string& path) {
    int fd = ::socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return -1;

    // Remove stale socket file
    ::unlink(path.c_str());

    struct sockaddr_un addr{};
    addr.sun_family = AF_UNIX;
    if (path.size() >= sizeof(addr.sun_path)) {
        ::close(fd);
        return -1;
    }
    std::strncpy(addr.sun_path, path.c_str(), sizeof(addr.sun_path) - 1);

    if (::bind(fd, reinterpret_cast<struct sockaddr*>(&addr), sizeof(addr)) < 0) {
        ::close(fd);
        return -1;
    }

    if (::listen(fd, 16) < 0) {
        ::close(fd);
        return -1;
    }

    return fd;
}

int AcceptClient(int server_fd) {
    int fd = ::accept(server_fd, nullptr, nullptr);
    return fd;
}

int ConnectToServer(const std::string& path) {
    int fd = ::socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) return -1;

    struct sockaddr_un addr{};
    addr.sun_family = AF_UNIX;
    if (path.size() >= sizeof(addr.sun_path)) {
        ::close(fd);
        return -1;
    }
    std::strncpy(addr.sun_path, path.c_str(), sizeof(addr.sun_path) - 1);

    if (::connect(fd, reinterpret_cast<struct sockaddr*>(&addr), sizeof(addr)) < 0) {
        ::close(fd);
        return -1;
    }

    return fd;
}

} // namespace amdgpu_proxy
