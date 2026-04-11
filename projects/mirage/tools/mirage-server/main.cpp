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
#include "mirage/simulator.h"

#include <httplib.h>

#include <array>
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fcntl.h>
#include <fstream>
#include <iostream>
#include <mutex>
#include <poll.h>
#include <pty.h>
#include <signal.h>
#include <sstream>
#include <stdexcept>
#include <string>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <thread>
#include <unistd.h>
#include <unordered_map>
#include <vector>

namespace {

// ── Shell helper ───────────────────────────────────────────────────────────

/// Run a shell command and capture stdout. Returns exit code.
int exec_cmd(const std::string& cmd, std::string& out) {
    out.clear();
    std::array<char, 4096> buf;
    FILE* pipe = popen(cmd.c_str(), "r");
    if (!pipe) return -1;
    while (fgets(buf.data(), static_cast<int>(buf.size()), pipe)) {
        out += buf.data();
    }
    int status = pclose(pipe);
    // Trim trailing newline
    while (!out.empty() && (out.back() == '\n' || out.back() == '\r'))
        out.pop_back();
    return WEXITSTATUS(status);
}

/// Run a command, return stdout. Throws on non-zero exit.
std::string exec_or_throw(const std::string& cmd) {
    std::string out;
    int rc = exec_cmd(cmd, out);
    if (rc != 0)
        throw std::runtime_error("command failed (" + std::to_string(rc) +
                                 "): " + cmd + "\n" + out);
    return out;
}

/// Escape a string for safe use in a shell single-quote context.
std::string shell_escape(const std::string& s) {
    std::string out = "'";
    for (char c : s) {
        if (c == '\'')
            out += "'\\''";
        else
            out += c;
    }
    out += "'";
    return out;
}

// ── Base64 encoding/decoding ───────────────────────────────────────────────

static const char b64_table[] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

std::string base64_encode(const std::string& in) {
    std::string out;
    int val = 0, valb = -6;
    for (unsigned char c : in) {
        val = (val << 8) + c;
        valb += 8;
        while (valb >= 0) {
            out.push_back(b64_table[(val >> valb) & 0x3F]);
            valb -= 6;
        }
    }
    if (valb > -6)
        out.push_back(b64_table[((val << 8) >> (valb + 8)) & 0x3F]);
    while (out.size() % 4) out.push_back('=');
    return out;
}

std::string base64_decode(const std::string& in) {
    auto val_of = [](unsigned char c) -> int {
        if (c >= 'A' && c <= 'Z') return c - 'A';
        if (c >= 'a' && c <= 'z') return c - 'a' + 26;
        if (c >= '0' && c <= '9') return c - '0' + 52;
        if (c == '+') return 62;
        if (c == '/') return 63;
        return -1;
    };
    std::string out;
    int val = 0, valb = -8;
    for (unsigned char c : in) {
        int d = val_of(c);
        if (d == -1) break;
        val = (val << 6) + d;
        valb += 6;
        if (valb >= 0) {
            out.push_back(static_cast<char>((val >> valb) & 0xFF));
            valb -= 8;
        }
    }
    return out;
}

// ── Run tracker (kept for old-style one-shot runs) ─────────────────────────

struct RunRecord {
    std::string id;
    std::string session;
    std::string command;
    std::string status;  // "running", "exited"
    int exit_code = -1;
    std::string output;
};

class RunTracker {
public:
    RunRecord start_run(const std::string& session,
                        const std::string& command) {
        std::lock_guard lock(mu_);
        auto id = "run-" + std::to_string(next_id_++);

        // Find container name for this session
        std::string container = "mirage-" + session;

        // Run docker exec in background, capture output
        std::string out;
        std::string docker_cmd = "docker exec " + shell_escape(container) +
                                 " " + command + " 2>&1";
        int rc = exec_cmd(docker_cmd, out);

        RunRecord rec{
            .id = id,
            .session = session,
            .command = command,
            .status = "exited",
            .exit_code = rc,
            .output = out,
        };
        runs_.push_back(rec);
        return rec;
    }

    std::vector<RunRecord> list_runs(const std::string& session_filter = "") {
        std::lock_guard lock(mu_);
        if (session_filter.empty()) return runs_;
        std::vector<RunRecord> filtered;
        for (const auto& r : runs_) {
            if (r.session == session_filter) filtered.push_back(r);
        }
        return filtered;
    }

private:
    std::mutex mu_;
    int next_id_ = 1;
    std::vector<RunRecord> runs_;
};

// ── Interactive PTY terminal sessions ──────────────────────────────────────

// Forward declaration — defined later alongside other Docker helpers.
bool docker_container_running(const std::string& session_name);

struct TerminalSession {
    std::string id;
    std::string session;  // Docker session name
    int master_fd = -1;
    pid_t pid = -1;
    std::mutex buf_mu;
    std::string output_buf;
    std::atomic<bool> alive{true};
};

class TerminalManager {
public:
    ~TerminalManager() {
        std::lock_guard lock(mu_);
        for (auto& [_, t] : terminals_) {
            t->alive.store(false);
            if (t->master_fd >= 0) ::close(t->master_fd);
            if (t->pid > 0) {
                kill(t->pid, SIGKILL);
                waitpid(t->pid, nullptr, WNOHANG);
            }
        }
    }

    struct CreateResult {
        bool ok = false;
        std::string error;
        std::string id;
    };

    CreateResult create(const std::string& session_name) {
        if (!docker_container_running(session_name)) {
            return {false, "session container not running", ""};
        }

        auto term = std::make_shared<TerminalSession>();
        {
            std::lock_guard lock(mu_);
            term->id = "term-" + std::to_string(next_id_++);
        }
        term->session = session_name;

        struct winsize ws{};
        ws.ws_row = 24;
        ws.ws_col = 80;

        term->pid = forkpty(&term->master_fd, nullptr, nullptr, &ws);
        if (term->pid < 0) {
            return {false, "forkpty() failed", ""};
        }

        if (term->pid == 0) {
            // Child process — exec into the Docker container
            std::string container = "mirage-" + session_name;
            execlp("docker", "docker", "exec", "-it",
                   container.c_str(), "/bin/bash", nullptr);
            _exit(127);
        }

        // Parent — start a reader thread to drain PTY output
        auto t = term;
        std::thread reader([t]() {
            char buf[4096];
            while (t->alive.load()) {
                struct pollfd pfd{};
                pfd.fd = t->master_fd;
                pfd.events = POLLIN;
                int ret = poll(&pfd, 1, 200);
                if (ret > 0 && (pfd.revents & POLLIN)) {
                    ssize_t n = ::read(t->master_fd, buf, sizeof(buf));
                    if (n > 0) {
                        std::lock_guard lock(t->buf_mu);
                        t->output_buf.append(buf, static_cast<size_t>(n));
                        // Cap buffer at 256 KB to prevent unbounded growth
                        if (t->output_buf.size() > 256 * 1024) {
                            t->output_buf.erase(
                                0, t->output_buf.size() - 128 * 1024);
                        }
                    } else if (n < 0 &&
                               (errno == EAGAIN || errno == EIO)) {
                        // EIO is normal during PTY startup; EAGAIN for
                        // non-blocking.  Just retry.
                        continue;
                    } else {
                        // n == 0  → slave closed; other errors → done
                        t->alive.store(false);
                        break;
                    }
                } else if (ret > 0 &&
                           (pfd.revents & (POLLHUP | POLLERR)) &&
                           !(pfd.revents & POLLIN)) {
                    // Only treat HUP/ERR as terminal if no data pending
                    t->alive.store(false);
                    break;
                }
            }
            // Reap child
            if (t->pid > 0) {
                waitpid(t->pid, nullptr, WNOHANG);
            }
        });
        reader.detach();

        std::lock_guard lock(mu_);
        terminals_[term->id] = term;
        return {true, "", term->id};
    }

    bool write_input(const std::string& id, const std::string& data) {
        auto t = find(id);
        if (!t || !t->alive.load()) return false;
        ssize_t n = ::write(t->master_fd, data.data(), data.size());
        return n > 0;
    }

    /// Drain all buffered output and return it (may be empty).
    std::string read_output(const std::string& id) {
        auto t = find(id);
        if (!t) return "";
        std::lock_guard lock(t->buf_mu);
        std::string out;
        std::swap(out, t->output_buf);
        return out;
    }

    bool is_alive(const std::string& id) {
        auto t = find(id);
        return t && t->alive.load();
    }

    bool resize(const std::string& id, uint16_t rows, uint16_t cols) {
        auto t = find(id);
        if (!t || !t->alive.load()) return false;
        struct winsize ws{};
        ws.ws_row = rows;
        ws.ws_col = cols;
        return ioctl(t->master_fd, TIOCSWINSZ, &ws) == 0;
    }

    bool close_terminal(const std::string& id) {
        std::lock_guard lock(mu_);
        auto it = terminals_.find(id);
        if (it == terminals_.end()) return false;
        auto t = it->second;
        t->alive.store(false);
        if (t->master_fd >= 0) {
            ::close(t->master_fd);
            t->master_fd = -1;
        }
        if (t->pid > 0) {
            kill(t->pid, SIGKILL);
            waitpid(t->pid, nullptr, WNOHANG);
        }
        terminals_.erase(it);
        return true;
    }

    struct TerminalInfo {
        std::string id;
        std::string session;
        bool alive;
    };

    std::vector<TerminalInfo> list() {
        std::lock_guard lock(mu_);
        std::vector<TerminalInfo> result;
        for (auto& [_, t] : terminals_) {
            result.push_back({t->id, t->session, t->alive.load()});
        }
        return result;
    }

private:
    std::shared_ptr<TerminalSession> find(const std::string& id) {
        std::lock_guard lock(mu_);
        auto it = terminals_.find(id);
        return (it != terminals_.end()) ? it->second : nullptr;
    }

    std::mutex mu_;
    int next_id_ = 1;
    std::unordered_map<std::string, std::shared_ptr<TerminalSession>>
        terminals_;
};

// ── Docker-backed dummy simulator ──────────────────────────────────────────

static const std::string LABEL_PREFIX = "mirage.";

class DummySimulator : public mirage::Simulator {
public:
    mirage::SimulatorInfo info() const override {
        return {
            .name = "rocjitsu",
            .version = "0.5.0",
            .description = "AMD GPU functional & cycle-accurate simulator",
            .supported_gpus =
                {
                    {"MI300X", "gfx942", mirage::GpuFamily::AmdCdna,
                     "AMD Instinct MI300X — 304 CUs, 192 GB HBM3"},
                    {"MI325X", "gfx950", mirage::GpuFamily::AmdCdna,
                     "AMD Instinct MI325X — 304 CUs, 256 GB HBM3e"},
                    {"MI350X", "gfx960", mirage::GpuFamily::AmdCdna,
                     "AMD Instinct MI350X — next-gen CDNA"},
                },
            .supports_custom_gpus = true,
            .supported_modes = {mirage::SimulatorMode::Functional,
                                mirage::SimulatorMode::Clocked,
                                mirage::SimulatorMode::CycleAccurate},
        };
    }

    std::vector<mirage::GpuDef> supported_gpus() const override {
        return info().supported_gpus;
    }

    mirage::GpuDef set_custom_gpu(const mirage::CustomGpuDef& gpu) override {
        return {"custom-" + gpu.name, "gfx9xx", mirage::GpuFamily::AmdCdna,
                "Custom: " + gpu.name};
    }

    mirage::ContainerDef create_session(
        const mirage::SessionDef& session,
        const mirage::ProfileDef& profile) override {
        std::string image =
            session.image.empty() ? "ubuntu:22.04" : session.image;
        std::string container_name = "mirage-" + session.name;

        // Remove any stale container with the same name
        std::string rm_out;
        exec_cmd("docker rm -f " + shell_escape(container_name) + " 2>/dev/null",
                 rm_out);

        // Build docker run command with labels
        std::ostringstream cmd;
        cmd << "docker run -d"
            << " --name " << shell_escape(container_name)
            << " --label " << shell_escape(LABEL_PREFIX + "session=" + session.name)
            << " --label " << shell_escape(LABEL_PREFIX + "profile=" + session.profile)
            << " --label " << shell_escape(LABEL_PREFIX + "simulator=rocjitsu")
            << " --label " << shell_escape(LABEL_PREFIX + "gpu=" + profile.gpu)
            << " --label " << shell_escape(LABEL_PREFIX + "mode=" + mirage::to_string(profile.mode))
            << " --label " << shell_escape(LABEL_PREFIX + "image=" + image)
            << " -e ROCJITSU_GPU=" << shell_escape(profile.gpu)
            << " -e ROCJITSU_MODE=" << shell_escape(
                   profile.mode == mirage::SimulatorMode::CycleAccurate
                       ? "cycle" : "functional")
            << " " << shell_escape(image)
            << " sleep infinity";

        std::string out = exec_or_throw(cmd.str());
        std::cout << "Started container " << container_name
                  << " (" << out.substr(0, 12) << ")" << std::endl;

        mirage::ContainerDef c;
        c.image = image;
        c.env.push_back({"ROCJITSU_GPU", profile.gpu});
        return c;
    }

    void delete_session(const std::string& session_id) override {
        std::string container_name = "mirage-" + session_id;
        std::string out;
        exec_cmd("docker rm -f " + shell_escape(container_name) + " 2>&1",
                 out);
        std::cout << "Removed container " << container_name << std::endl;
    }

    mirage::SessionHealth get_session_health(
        const std::string& session_id) const override {
        mirage::SessionHealth h;
        h.session_id = session_id;

        std::string container_name = "mirage-" + session_id;
        std::string out;
        int rc = exec_cmd(
            "docker inspect --format '{{.State.Status}} {{.State.StartedAt}}' "
            + shell_escape(container_name) + " 2>/dev/null", out);

        if (rc != 0 || out.empty()) {
            h.status = mirage::HealthStatus::Unknown;
            return h;
        }

        auto space = out.find(' ');
        std::string status = (space != std::string::npos)
                                 ? out.substr(0, space)
                                 : out;

        if (status == "running") {
            h.status = mirage::HealthStatus::Healthy;
            // Parse uptime from StartedAt
            std::string started_at =
                (space != std::string::npos) ? out.substr(space + 1) : "";
            if (!started_at.empty()) {
                // Approximate uptime via `docker inspect` age
                std::string age_out;
                exec_cmd(
                    "docker inspect --format '{{.State.StartedAt}}' "
                    + shell_escape(container_name)
                    + " | xargs -I{} bash -c "
                      "'echo $(( $(date +%s) - $(date -d \"{}\" +%s) ))'",
                    age_out);
                try {
                    h.uptime = {
                        static_cast<uint64_t>(std::stoull(age_out)), 0};
                } catch (...) {
                    h.uptime = {0, 0};
                }
            }
        } else {
            h.status = mirage::HealthStatus::Unhealthy;
            h.error_message = "Container status: " + status;
        }
        return h;
    }

    mirage::SessionPerf get_session_perf(
        const std::string& session_id) const override {
        mirage::SessionPerf p;
        p.session_id = session_id;

        auto h = get_session_health(session_id);
        if (h.status == mirage::HealthStatus::Healthy) {
            auto secs = h.uptime.seconds + 1;
            p.ticks = secs * 2400000;
            p.ipc = 1.85;
            p.simulation_speed = 0.42;
            p.active_contexts = 64;
        }
        return p;
    }

    mirage::ExecDef get_run_def(const mirage::RunDef& run) const override {
        auto exec = run.exec;
        exec.env.push_back(
            {"LD_PRELOAD", "/usr/lib/librocjitsu_interposer.so"});
        return exec;
    }
};

// ── Docker-based session discovery (source of truth = docker ps) ───────────

struct DockerSession {
    std::string name;
    std::string profile;
    std::string simulator;
    std::string gpu;
    std::string mode;
    std::string image;
    std::string container_id;
    std::string status;  // "running", "exited", etc.
};

/// Query Docker for all mirage-managed containers.
std::vector<DockerSession> docker_list_sessions() {
    std::vector<DockerSession> result;
    std::string out;
    // Use --no-trunc to get full container IDs and all labels
    int rc = exec_cmd(
        "docker ps -a --filter 'label=mirage.session' "
        "--format '{{.Names}}\\t{{.Label \"mirage.session\"}}\\t"
        "{{.Label \"mirage.profile\"}}\\t{{.Label \"mirage.simulator\"}}\\t"
        "{{.Label \"mirage.gpu\"}}\\t{{.Label \"mirage.mode\"}}\\t"
        "{{.Label \"mirage.image\"}}\\t{{.ID}}\\t{{.Status}}' 2>/dev/null",
        out);
    if (rc != 0 || out.empty()) return result;

    std::istringstream iss(out);
    std::string line;
    while (std::getline(iss, line)) {
        if (line.empty()) continue;
        // Split by tabs
        std::vector<std::string> parts;
        std::istringstream ls(line);
        std::string part;
        while (std::getline(ls, part, '\t')) parts.push_back(part);
        if (parts.size() < 9) continue;

        DockerSession s;
        s.name = parts[1];       // mirage.session label
        s.profile = parts[2];    // mirage.profile label
        s.simulator = parts[3];  // mirage.simulator label
        s.gpu = parts[4];        // mirage.gpu label
        s.mode = parts[5];       // mirage.mode label
        s.image = parts[6];      // mirage.image label
        s.container_id = parts[7];
        s.status = parts[8];     // human-readable status
        result.push_back(std::move(s));
    }
    return result;
}

/// Check if a container is running by name.
bool docker_container_running(const std::string& session_name) {
    std::string out;
    int rc = exec_cmd(
        "docker inspect --format '{{.State.Running}}' "
        + shell_escape("mirage-" + session_name) + " 2>/dev/null", out);
    return rc == 0 && out == "true";
}

/// Get uptime in seconds for a running container.
uint64_t docker_container_uptime(const std::string& session_name) {
    std::string out;
    int rc = exec_cmd(
        "docker inspect --format '{{.State.StartedAt}}' "
        + shell_escape("mirage-" + session_name)
        + " 2>/dev/null | xargs -I{} bash -c "
          "'echo $(( $(date +%s) - $(date -d \"{}\" +%s) ))' 2>/dev/null",
        out);
    if (rc != 0 || out.empty()) return 0;
    try { return static_cast<uint64_t>(std::stoull(out)); }
    catch (...) { return 0; }
}

// ── JSON helpers for new types ─────────────────────────────────────────────

std::string run_to_json(const RunRecord& r) {
    std::ostringstream o;
    o << "{\"id\":" << mirage::json::str(r.id)
      << ",\"session\":" << mirage::json::str(r.session)
      << ",\"command\":" << mirage::json::str(r.command)
      << ",\"status\":" << mirage::json::str(r.status)
      << ",\"exit_code\":" << r.exit_code
      << ",\"output\":" << mirage::json::str(r.output)
      << "}";
    return o.str();
}

std::string docker_session_to_summary_json(const DockerSession& s) {
    std::string health = "Unknown";
    if (s.status.find("Up") != std::string::npos)
        health = "Healthy";
    else if (s.status.find("Exited") != std::string::npos)
        health = "Unhealthy";

    std::ostringstream o;
    o << "{\"name\":" << mirage::json::str(s.name)
      << ",\"profile\":" << mirage::json::str(s.profile)
      << ",\"simulator\":" << mirage::json::str(s.simulator)
      << ",\"image\":" << mirage::json::str(s.image)
      << ",\"health_status\":" << mirage::json::str(health)
      << "}";
    return o.str();
}

void setup_routes(httplib::Server& srv, mirage::DashboardService& svc,
                  RunTracker& runs, TerminalManager& terms) {
    using namespace mirage;

    const std::string prefix = "/mirage.simulator.Dashboard/";

    // ── GetOverview (uses Docker for session count) ────────────────────
    srv.Post(prefix + "GetOverview",
             [&svc](const httplib::Request&, httplib::Response& res) {
                 auto overview = svc.get_overview();
                 // Override session count from Docker
                 auto docker_sessions = docker_list_sessions();
                 overview.session_count =
                     static_cast<uint32_t>(docker_sessions.size());
                 res.set_content(json::to_json(overview),
                                 "application/json");
             });

    // ── ListSimulators ─────────────────────────────────────────────────
    srv.Post(prefix + "ListSimulators",
             [&svc](const httplib::Request&, httplib::Response& res) {
                 auto sims = svc.list_simulators();
                 // Correct session counts from Docker
                 auto docker_sessions = docker_list_sessions();
                 for (auto& sim : sims) {
                     uint32_t count = 0;
                     for (const auto& ds : docker_sessions) {
                         if (ds.simulator == sim.name) ++count;
                     }
                     sim.active_session_count = count;
                 }
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

    // ── ListSessions (Docker is source of truth) ───────────────────────
    srv.Post(
        prefix + "ListSessions",
        [](const httplib::Request& req, httplib::Response& res) {
            auto filter =
                mirage::json::json_get_string(req.body, "profile_filter");
            auto sessions = docker_list_sessions();

            std::ostringstream o;
            o << "{\"sessions\":[";
            bool first = true;
            for (const auto& s : sessions) {
                if (!filter.empty() && s.profile != filter) continue;
                if (!first) o << ",";
                o << docker_session_to_summary_json(s);
                first = false;
            }
            o << "]}";
            res.set_content(o.str(), "application/json");
        });

    // ── CreateSession ──────────────────────────────────────────────────
    srv.Post(
        prefix + "CreateSession",
        [&svc](const httplib::Request& req, httplib::Response& res) {
            auto session = json::parse_session(req.body);
            auto result = svc.create_session(session);
            res.set_content(json::to_json(result), "application/json");
        });

    // ── DeleteSession (Docker-backed) ────────────────────────────────
    srv.Post(
        prefix + "DeleteSession",
        [&svc](const httplib::Request& req, httplib::Response& res) {
            auto name = json::json_get_string(req.body, "name");
            // Try in-memory delete first (cleans up daemon state)
            svc.delete_session(name);
            // Always also force-remove the Docker container
            std::string rm_out;
            std::string container_name = "mirage-" + name;
            int rc = exec_cmd(
                "docker rm -f " + shell_escape(container_name) + " 2>&1",
                rm_out);
            if (rc == 0) {
                res.set_content("{\"ok\":true,\"error\":\"\"}",
                                "application/json");
            } else {
                res.set_content(
                    "{\"ok\":false,\"error\":\"container not found\"}",
                    "application/json");
            }
        });

    // ── GetSessionDetail (Docker-backed) ───────────────────────────────
    srv.Post(prefix + "GetSessionDetail",
             [&svc](const httplib::Request& req, httplib::Response& res) {
                 auto name = json::json_get_string(req.body, "name");
                 // Try Docker first for session existence
                 bool found_in_docker = false;
                 DockerSession docker_info;
                 for (const auto& s : docker_list_sessions()) {
                     if (s.name == name) {
                         found_in_docker = true;
                         docker_info = s;
                         break;
                     }
                 }
                 if (!found_in_docker) {
                     // Fall back to in-memory
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
                     return;
                 }

                 // Build detail from Docker state
                 bool running = docker_info.status.find("Up") !=
                                std::string::npos;
                 uint64_t uptime = running
                     ? docker_container_uptime(name) : 0;

                 std::ostringstream o;
                 o << "{\"name\":" << json::str(name)
                   << ",\"profile\":" << "{\"name\":"
                   << json::str(docker_info.profile)
                   << ",\"simulator\":" << json::str(docker_info.simulator)
                   << ",\"mode\":" << json::str(docker_info.mode)
                   << ",\"gpu\":" << json::str(docker_info.gpu)
                   << ",\"num_gpus\":1,\"num_nodes\":1}"
                   << ",\"simulator\":" << json::str(docker_info.simulator)
                   << ",\"image\":" << json::str(docker_info.image)
                   << ",\"health\":" << json::str(running ? "Healthy" : "Unhealthy")
                   << ",\"uptime\":{\"seconds\":" << uptime
                   << ",\"picoseconds\":0}"
                   << ",\"error_message\":" << json::str(
                          running ? "" : "Container " + docker_info.status)
                   << ",\"ticks\":" << (running ? (uptime + 1) * 2400000 : 0)
                   << ",\"ipc\":" << (running ? "1.85" : "0.0")
                   << ",\"simulation_speed\":"
                   << (running ? "0.42" : "0.0")
                   << ",\"active_contexts\":"
                   << (running ? "64" : "0")
                   << "}";
                 res.set_content(o.str(), "application/json");
             });

    // ── ListRuns ───────────────────────────────────────────────────────
    srv.Post(
        prefix + "ListRuns",
        [&runs](const httplib::Request& req, httplib::Response& res) {
            auto filter =
                mirage::json::json_get_string(req.body, "session_filter");
            auto all_runs = runs.list_runs(filter);
            std::ostringstream o;
            o << "{\"runs\":[";
            for (size_t i = 0; i < all_runs.size(); ++i) {
                if (i) o << ",";
                o << run_to_json(all_runs[i]);
            }
            o << "]}";
            res.set_content(o.str(), "application/json");
        });

    // ── CreateRun ──────────────────────────────────────────────────────
    srv.Post(
        prefix + "CreateRun",
        [&runs](const httplib::Request& req, httplib::Response& res) {
            auto session =
                mirage::json::json_get_string(req.body, "session");
            auto command =
                mirage::json::json_get_string(req.body, "command");

            if (session.empty() || command.empty()) {
                res.set_content(
                    "{\"ok\":false,\"error\":\"session and command required\"}",
                    "application/json");
                return;
            }

            // Verify session container exists and is running
            if (!docker_container_running(session)) {
                res.set_content(
                    "{\"ok\":false,\"error\":\"session container not running\"}",
                    "application/json");
                return;
            }

            auto rec = runs.start_run(session, command);
            res.set_content("{\"ok\":true,\"run\":" + run_to_json(rec) + "}",
                            "application/json");
        });

    // ── Terminal: Create ─────────────────────────────────────────────
    srv.Post(
        "/api/terminal/create",
        [&terms](const httplib::Request& req, httplib::Response& res) {
            auto session =
                mirage::json::json_get_string(req.body, "session");
            if (session.empty()) {
                res.set_content(
                    "{\"ok\":false,\"error\":\"session required\"}",
                    "application/json");
                return;
            }
            auto result = terms.create(session);
            std::ostringstream o;
            o << "{\"ok\":" << (result.ok ? "true" : "false")
              << ",\"error\":" << mirage::json::str(result.error)
              << ",\"id\":" << mirage::json::str(result.id) << "}";
            res.set_content(o.str(), "application/json");
        });

    // ── Terminal: Input (base64-encoded keystrokes) ────────────────────
    srv.Post(
        "/api/terminal/input",
        [&terms](const httplib::Request& req, httplib::Response& res) {
            auto id = mirage::json::json_get_string(req.body, "id");
            auto data_b64 =
                mirage::json::json_get_string(req.body, "data");
            auto data = base64_decode(data_b64);
            bool ok = terms.write_input(id, data);
            res.set_content(
                ok ? "{\"ok\":true}" : "{\"ok\":false}",
                "application/json");
        });

    // ── Terminal: Output (returns base64-encoded PTY output) ───────────
    srv.Post(
        "/api/terminal/output",
        [&terms](const httplib::Request& req, httplib::Response& res) {
            auto id = mirage::json::json_get_string(req.body, "id");
            auto raw = terms.read_output(id);
            bool alive = terms.is_alive(id);
            std::ostringstream o;
            o << "{\"data\":" << mirage::json::str(base64_encode(raw))
              << ",\"alive\":" << (alive ? "true" : "false") << "}";
            res.set_content(o.str(), "application/json");
        });

    // ── Terminal: Resize ───────────────────────────────────────────────
    srv.Post(
        "/api/terminal/resize",
        [&terms](const httplib::Request& req, httplib::Response& res) {
            auto id = mirage::json::json_get_string(req.body, "id");
            auto rows = mirage::json::json_get_uint(req.body, "rows", 24);
            auto cols = mirage::json::json_get_uint(req.body, "cols", 80);
            bool ok = terms.resize(id, static_cast<uint16_t>(rows),
                                   static_cast<uint16_t>(cols));
            res.set_content(
                ok ? "{\"ok\":true}" : "{\"ok\":false}",
                "application/json");
        });

    // ── Terminal: Close ────────────────────────────────────────────────
    srv.Post(
        "/api/terminal/close",
        [&terms](const httplib::Request& req, httplib::Response& res) {
            auto id = mirage::json::json_get_string(req.body, "id");
            bool ok = terms.close_terminal(id);
            res.set_content(
                ok ? "{\"ok\":true}" : "{\"ok\":false}",
                "application/json");
        });

    // ── Terminal: List ─────────────────────────────────────────────────
    srv.Post(
        "/api/terminal/list",
        [&terms](const httplib::Request&, httplib::Response& res) {
            auto all = terms.list();
            std::ostringstream o;
            o << "{\"terminals\":[";
            for (size_t i = 0; i < all.size(); ++i) {
                if (i) o << ",";
                o << "{\"id\":" << mirage::json::str(all[i].id)
                  << ",\"session\":" << mirage::json::str(all[i].session)
                  << ",\"alive\":" << (all[i].alive ? "true" : "false")
                  << "}";
            }
            o << "]}";
            res.set_content(o.str(), "application/json");
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

    // Set up daemon and register the built-in dummy simulator
    mirage::Daemon daemon;
    daemon.register_simulator(std::make_shared<DummySimulator>());

    // Seed some demo profiles so the dashboard isn't empty on first load
    daemon.add_profile({"mi300x-functional", "rocjitsu",
                         mirage::SimulatorMode::Functional, "MI300X", 1, 1});
    daemon.add_profile({"mi300x-8gpu-cycle", "rocjitsu",
                         mirage::SimulatorMode::CycleAccurate, "MI300X", 8, 1});
    daemon.add_profile({"mi325x-clocked", "rocjitsu",
                         mirage::SimulatorMode::Clocked, "MI325X", 4, 2});

    mirage::DashboardService svc(daemon);

    RunTracker runs;
    TerminalManager terms;

    httplib::Server srv;

    // CORS headers for dashboard dev server
    srv.set_default_headers({
        {"Access-Control-Allow-Origin", "*"},
        {"Access-Control-Allow-Methods", "POST, OPTIONS"},
        {"Access-Control-Allow-Headers", "Content-Type"},
    });

    setup_routes(srv, svc, runs, terms);

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
