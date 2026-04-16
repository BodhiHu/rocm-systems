// main.cpp — Proxy server entry point
#include "../rpc/server.h"
#include "../rpc/transport.h"
#include "../rpc/backend/real_hardware.h"

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <csignal>
#include <memory>

static amdgpu_proxy::ProxyServer* g_server = nullptr;

static void signal_handler(int) {
    if (g_server) g_server->Stop();
}

static void usage(const char* prog) {
    std::fprintf(stderr,
        "Usage: %s [options]\n"
        "Options:\n"
        "  --socket <path>  Socket path (default: $AMDGPU_PROXY_SOCK or /tmp/amdgpu-proxy.sock)\n"
        "  --help           Show this message\n",
        prog);
}

int main(int argc, char* argv[]) {
    std::string socket_path;

    for (int i = 1; i < argc; i++) {
        if (std::strcmp(argv[i], "--socket") == 0 && i + 1 < argc) {
            socket_path = argv[++i];
        } else if (std::strcmp(argv[i], "--help") == 0) {
            usage(argv[0]);
            return 0;
        } else {
            std::fprintf(stderr, "Unknown option: %s\n", argv[i]);
            usage(argv[0]);
            return 1;
        }
    }

    auto backend = std::make_unique<amdgpu_proxy::RealHardwareBackend>();
    std::fprintf(stderr, "[amdgpu-proxy] Starting with real hardware backend\n");

    amdgpu_proxy::ProxyServer server(*backend);
    g_server = &server;

    std::signal(SIGINT, signal_handler);
    std::signal(SIGTERM, signal_handler);

    std::fprintf(stderr, "[amdgpu-proxy] Listening on %s\n",
        socket_path.empty() ? amdgpu_proxy::GetSocketPath().c_str() : socket_path.c_str());

    if (!server.Start(socket_path)) {
        std::fprintf(stderr, "[amdgpu-proxy] Failed to start server\n");
        return 1;
    }

    std::fprintf(stderr, "[amdgpu-proxy] Server stopped\n");
    g_server = nullptr;
    return 0;
}
