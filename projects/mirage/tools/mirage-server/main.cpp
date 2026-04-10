/// mirage-server — gRPC-Web JSON API server for the mirage dashboard.
///
/// Exposes the Dashboard service (simulator.fbs) over HTTP using
/// gRPC-Web JSON content-type conventions.  Each RPC maps to:
///   POST /mirage.simulator.Dashboard/{MethodName}
///
/// Also serves the dashboard SPA static files when --static is given.
///
/// Usage:
///   mirage-server [--port PORT] [--static DIR]  (default port: 50051)

#include "mirage/daemon.h"
#include "mirage/dashboard_service.h"
#include "mirage/json_helpers.h"

#include <httplib.h>

#include <cstdlib>
#include <fstream>
#include <iostream>
#include <string>

namespace {

void setup_routes(httplib::Server& srv, mirage::DashboardService& svc) {
    using namespace mirage;

    // All gRPC-Web endpoints use POST with JSON bodies.
    const std::string prefix = "/mirage.simulator.Dashboard/";

    // ── GetOverview ────────────────────────────────────────────────────
    srv.Post(prefix + "GetOverview",
             [&svc](const httplib::Request&, httplib::Response& res) {
                 res.set_content(json::to_json(svc.get_overview()),
                                 "application/json");
             });

    // ── ListSimulators ─────────────────────────────────────────────────
    srv.Post(prefix + "ListSimulators",
             [&svc](const httplib::Request&, httplib::Response& res) {
                 auto sims = svc.list_simulators();
                 res.set_content("{\"simulators\":" +
                                     json::array_to_json(sims) + "}",
                                 "application/json");
             });

    // ── GetSimulator ───────────────────────────────────────────────────
    srv.Post(prefix + "GetSimulator",
             [&svc](const httplib::Request& req, httplib::Response& res) {
                 auto name = json::json_get_string(req.body, "name");
                 auto sim = svc.get_simulator(name);
                 if (sim) {
                     res.set_content("{\"simulator\":" +
                                         json::to_json(*sim) + "}",
                                     "application/json");
                 } else {
                     res.status = 404;
                     res.set_content(
                         "{\"error\":\"simulator not found\"}",
                         "application/json");
                 }
             });

    // ── ListProfiles ───────────────────────────────────────────────────
    srv.Post(
        prefix + "ListProfiles",
        [&svc](const httplib::Request& req, httplib::Response& res) {
            auto filter =
                json::json_get_string(req.body, "simulator_filter");
            auto profiles = svc.list_profiles(filter);
            res.set_content(
                "{\"profiles\":" + json::array_to_json(profiles) + "}",
                "application/json");
        });

    // ── CreateProfile ──────────────────────────────────────────────────
    srv.Post(
        prefix + "CreateProfile",
        [&svc](const httplib::Request& req, httplib::Response& res) {
            auto profile = json::parse_profile(req.body);
            auto result = svc.create_profile(profile);
            res.set_content(json::to_json(result), "application/json");
        });

    // ── DeleteProfile ──────────────────────────────────────────────────
    srv.Post(
        prefix + "DeleteProfile",
        [&svc](const httplib::Request& req, httplib::Response& res) {
            auto name = json::json_get_string(req.body, "name");
            auto result = svc.delete_profile(name);
            res.set_content(json::to_json(result), "application/json");
        });

    // ── ListSessions ───────────────────────────────────────────────────
    srv.Post(
        prefix + "ListSessions",
        [&svc](const httplib::Request& req, httplib::Response& res) {
            auto filter =
                json::json_get_string(req.body, "profile_filter");
            auto sessions = svc.list_sessions(filter);
            res.set_content(
                "{\"sessions\":" + json::array_to_json(sessions) + "}",
                "application/json");
        });

    // ── CreateSession ──────────────────────────────────────────────────
    srv.Post(
        prefix + "CreateSession",
        [&svc](const httplib::Request& req, httplib::Response& res) {
            auto session = json::parse_session(req.body);
            auto result = svc.create_session(session);
            res.set_content(json::to_json(result), "application/json");
        });

    // ── DeleteSession ──────────────────────────────────────────────────
    srv.Post(
        prefix + "DeleteSession",
        [&svc](const httplib::Request& req, httplib::Response& res) {
            auto name = json::json_get_string(req.body, "name");
            auto result = svc.delete_session(name);
            res.set_content(json::to_json(result), "application/json");
        });

    // ── GetSessionDetail ───────────────────────────────────────────────
    srv.Post(prefix + "GetSessionDetail",
             [&svc](const httplib::Request& req, httplib::Response& res) {
                 auto name = json::json_get_string(req.body, "name");
                 auto detail = svc.get_session_detail(name);
                 if (detail) {
                     res.set_content(json::to_json(*detail),
                                     "application/json");
                 } else {
                     res.status = 404;
                     res.set_content(
                         "{\"error\":\"session not found\"}",
                         "application/json");
                 }
             });

    // ── CORS preflight ─────────────────────────────────────────────────
    srv.Options("/(.*)", [](const httplib::Request&, httplib::Response& res) {
        res.status = 204;
    });
}

} // namespace

int main(int argc, char* argv[]) {
    int port = 50051;
    std::string static_dir;

    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if ((arg == "--port" || arg == "-p") && i + 1 < argc) {
            port = std::atoi(argv[++i]);
        } else if (arg == "--static" && i + 1 < argc) {
            static_dir = argv[++i];
        }
    }

    // Set up daemon with empty config (simulators register at runtime)
    mirage::Daemon daemon;
    mirage::DashboardService svc(daemon);

    httplib::Server srv;

    // CORS headers for dashboard dev server
    srv.set_default_headers({
        {"Access-Control-Allow-Origin", "*"},
        {"Access-Control-Allow-Methods", "POST, OPTIONS"},
        {"Access-Control-Allow-Headers", "Content-Type"},
    });

    setup_routes(srv, svc);

    // Serve the dashboard SPA static files.
    if (!static_dir.empty()) {
        srv.set_mount_point("/", static_dir);

        // SPA fallback: serve index.html for any GET that didn't match
        // a static file or an API route.
        srv.set_error_handler(
            [&static_dir](const httplib::Request& req,
                          httplib::Response& res) {
                if (res.status == 404 && req.method == "GET") {
                    std::ifstream ifs(static_dir + "/index.html");
                    if (ifs) {
                        std::string html((std::istreambuf_iterator<char>(ifs)),
                                         std::istreambuf_iterator<char>());
                        res.set_content(html, "text/html");
                        res.status = 200;
                    }
                }
            });

        std::cout << "Serving dashboard from " << static_dir << std::endl;
    }

    std::cout << "mirage-server listening on http://0.0.0.0:" << port
              << std::endl;
    if (!srv.listen("0.0.0.0", port)) {
        std::cerr << "Failed to start server on port " << port << std::endl;
        return 1;
    }
    return 0;
}
