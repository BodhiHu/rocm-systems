/// mirage-server — gRPC + gRPC-Web server for the mirage dashboard.
///
/// Exposes the Dashboard service (simulator.fbs) as a real gRPC server
/// on port 50052 and a gRPC-Web binary endpoint on port 50051 (httplib).
///
/// The browser talks binary FlatBuffers via HTTP POST:
///   POST /mirage.simulator.Dashboard/{Method}
///   Content-Type: application/x-flatbuffers
///   Body: raw FlatBuffer bytes
///
/// C++ / Python clients use native gRPC on port 50052.
///
/// Usage:
///   mirage-server [--port PORT] [--grpc-port PORT] [--static DIR]

#include "mirage/daemon.h"
#include "mirage/dashboard_service.h"
#include "mirage/json_helpers.h"
#include "mirage/simulator.h"

#include "simulator_generated.h"
#include "simulator.grpc.fb.h"

#include <flatbuffers/flatbuffers.h>
#include <flatbuffers/grpc.h>
#include <grpcpp/grpcpp.h>
#include <httplib.h>

#include <array>
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fcntl.h>
#include <fstream>
#include <functional>
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

int exec_cmd(const std::string& cmd, std::string& out) {
    out.clear();
    std::array<char, 4096> buf;
    FILE* pipe = popen(cmd.c_str(), "r");
    if (!pipe) return -1;
    while (fgets(buf.data(), static_cast<int>(buf.size()), pipe)) {
        out += buf.data();
    }
    int status = pclose(pipe);
    while (!out.empty() && (out.back() == '\n' || out.back() == '\r'))
        out.pop_back();
    return WEXITSTATUS(status);
}

std::string exec_or_throw(const std::string& cmd) {
    std::string out;
    int rc = exec_cmd(cmd, out);
    if (rc != 0)
        throw std::runtime_error("command failed (" + std::to_string(rc) +
                                 "): " + cmd + "\n" + out);
    return out;
}

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

// ── Run tracker ────────────────────────────────────────────────────────────

struct RunRecord {
    std::string id;
    std::string session;
    std::string command;
    std::string status;
    int exit_code = -1;
    std::string output;
};

class RunTracker {
public:
    RunRecord start_run(const std::string& session,
                        const std::string& command) {
        std::lock_guard lock(mu_);
        auto id = "run-" + std::to_string(next_id_++);
        std::string container = "mirage-" + session;
        std::string out;
        std::string docker_cmd = "docker exec " + shell_escape(container) +
                                 " " + command + " 2>&1";
        int rc = exec_cmd(docker_cmd, out);
        RunRecord rec{
            .id = id, .session = session, .command = command,
            .status = "exited", .exit_code = rc, .output = out,
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

bool docker_container_running(const std::string& session_name);

struct TerminalSession {
    std::string id;
    std::string session;
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

    struct CreateResult { bool ok = false; std::string error; std::string id; };

    CreateResult create(const std::string& session_name) {
        if (!docker_container_running(session_name))
            return {false, "session container not running", ""};
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
        if (term->pid < 0) return {false, "forkpty() failed", ""};
        if (term->pid == 0) {
            std::string container = "mirage-" + session_name;
            execlp("docker", "docker", "exec", "-it",
                   container.c_str(), "/bin/bash", nullptr);
            _exit(127);
        }
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
                        if (t->output_buf.size() > 256 * 1024)
                            t->output_buf.erase(0, t->output_buf.size() - 128 * 1024);
                    } else if (n < 0 && (errno == EAGAIN || errno == EIO)) {
                        continue;
                    } else {
                        t->alive.store(false);
                        break;
                    }
                } else if (ret > 0 &&
                           (pfd.revents & (POLLHUP | POLLERR)) &&
                           !(pfd.revents & POLLIN)) {
                    t->alive.store(false);
                    break;
                }
            }
            if (t->pid > 0) waitpid(t->pid, nullptr, WNOHANG);
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
        if (t->master_fd >= 0) { ::close(t->master_fd); t->master_fd = -1; }
        if (t->pid > 0) { kill(t->pid, SIGKILL); waitpid(t->pid, nullptr, WNOHANG); }
        terminals_.erase(it);
        return true;
    }

    struct TerminalInfo { std::string id; std::string session; bool alive; };

    std::vector<TerminalInfo> list() {
        std::lock_guard lock(mu_);
        std::vector<TerminalInfo> result;
        for (auto& [_, t] : terminals_)
            result.push_back({t->id, t->session, t->alive.load()});
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
    std::unordered_map<std::string, std::shared_ptr<TerminalSession>> terminals_;
};

// ── Session log store ──────────────────────────────────────────────────────

struct SessionLogEntry {
    std::mutex mu;
    std::string log;
    std::string status;
};

class SessionLogStore {
public:
    void create(const std::string& name) {
        std::lock_guard lock(mu_);
        auto& entry = logs_[name];
        entry = std::make_shared<SessionLogEntry>();
        entry->status = "pulling";
    }
    void append(const std::string& name, const std::string& text) {
        auto e = find(name); if (!e) return;
        std::lock_guard lock(e->mu);
        e->log += text;
    }
    void set_status(const std::string& name, const std::string& status) {
        auto e = find(name); if (!e) return;
        std::lock_guard lock(e->mu);
        e->status = status;
    }
    struct Snapshot { std::string log; std::string status; };
    Snapshot get(const std::string& name) {
        auto e = find(name);
        if (!e) return {"", ""};
        std::lock_guard lock(e->mu);
        return {e->log, e->status};
    }
    void remove(const std::string& name) {
        std::lock_guard lock(mu_);
        logs_.erase(name);
    }
private:
    std::shared_ptr<SessionLogEntry> find(const std::string& name) {
        std::lock_guard lock(mu_);
        auto it = logs_.find(name);
        return (it != logs_.end()) ? it->second : nullptr;
    }
    std::mutex mu_;
    std::unordered_map<std::string, std::shared_ptr<SessionLogEntry>> logs_;
};

// ── Docker-backed simulator ────────────────────────────────────────────────

static const std::string LABEL_PREFIX = "mirage.";

class DummySimulator : public mirage::Simulator {
public:
    mirage::SimulatorInfo info() const override {
        return {
            .name = "rocjitsu",
            .version = "0.5.0",
            .description = "AMD GPU functional & cycle-accurate simulator",
            .supported_gpus = {
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
        std::string image = session.image.empty() ? "ubuntu:22.04" : session.image;
        std::string container_name = "mirage-" + session.name;
        std::string rm_out;
        exec_cmd("docker rm -f " + shell_escape(container_name) + " 2>/dev/null", rm_out);
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
            << " " << shell_escape(image) << " sleep infinity";
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
        exec_cmd("docker rm -f " + shell_escape(container_name) + " 2>&1", out);
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
        if (rc != 0 || out.empty()) { h.status = mirage::HealthStatus::Unknown; return h; }
        auto space = out.find(' ');
        std::string status = (space != std::string::npos) ? out.substr(0, space) : out;
        if (status == "running") {
            h.status = mirage::HealthStatus::Healthy;
            std::string age_out;
            exec_cmd(
                "docker inspect --format '{{.State.StartedAt}}' "
                + shell_escape(container_name)
                + " | xargs -I{} bash -c "
                  "'echo $(( $(date +%s) - $(date -d \"{}\" +%s) ))'",
                age_out);
            try { h.uptime = {static_cast<uint64_t>(std::stoull(age_out)), 0}; }
            catch (...) { h.uptime = {0, 0}; }
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
        exec.env.push_back({"LD_PRELOAD", "/usr/lib/librocjitsu_interposer.so"});
        return exec;
    }
};

// ── Docker session discovery ───────────────────────────────────────────────

struct DockerSession {
    std::string name, profile, simulator, gpu, mode, image, container_id, status;
};

std::vector<DockerSession> docker_list_sessions() {
    std::vector<DockerSession> result;
    std::string out;
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
        std::vector<std::string> parts;
        std::istringstream ls(line);
        std::string part;
        while (std::getline(ls, part, '\t')) parts.push_back(part);
        if (parts.size() < 9) continue;
        result.push_back({parts[1], parts[2], parts[3], parts[4],
                          parts[5], parts[6], parts[7], parts[8]});
    }
    return result;
}

bool docker_container_running(const std::string& session_name) {
    std::string out;
    int rc = exec_cmd(
        "docker inspect --format '{{.State.Running}}' "
        + shell_escape("mirage-" + session_name) + " 2>/dev/null", out);
    return rc == 0 && out == "true";
}

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

// ============================================================================
//  Autogenerated gRPC Dashboard service implementation
// ============================================================================

namespace fb = mirage::simulator;

class MirageDashboardService final : public fb::Dashboard::Service {
public:
    MirageDashboardService(mirage::DashboardService& svc,
                           RunTracker& runs,
                           TerminalManager& terms,
                           SessionLogStore& session_logs)
        : svc_(svc), runs_(runs), terms_(terms), session_logs_(session_logs) {}

    // ── GetOverview ────────────────────────────────────────────────────
    ::grpc::Status GetOverview(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::GetOverviewRequest>*,
        flatbuffers::grpc::Message<fb::GetOverviewReply>* response) override {
        auto overview = svc_.get_overview();
        auto docker_sessions = docker_list_sessions();
        overview.session_count = static_cast<uint32_t>(docker_sessions.size());

        flatbuffers::grpc::MessageBuilder mb;
        auto off = fb::CreateGetOverviewReply(
            mb, overview.simulator_count, overview.profile_count,
            overview.session_count);
        mb.Finish(off);
        *response = mb.ReleaseMessage<fb::GetOverviewReply>();
        return ::grpc::Status::OK;
    }

    // ── ListSimulators ─────────────────────────────────────────────────
    ::grpc::Status ListSimulators(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::ListSimulatorsRequest>*,
        flatbuffers::grpc::Message<fb::ListSimulatorsReply>* response) override {
        auto sims = svc_.list_simulators();
        auto docker_sessions = docker_list_sessions();
        for (auto& sim : sims) {
            uint32_t count = 0;
            for (const auto& ds : docker_sessions)
                if (ds.simulator == sim.name) ++count;
            sim.active_session_count = count;
        }

        flatbuffers::grpc::MessageBuilder mb;
        std::vector<flatbuffers::Offset<fb::SimulatorSummary>> sim_offsets;
        for (const auto& s : sims) {
            auto nm = mb.CreateString(s.name);
            auto ver = mb.CreateString(s.version);
            auto desc = mb.CreateString(s.description);
            std::vector<flatbuffers::Offset<mirage::fb::GpuDef>> gpu_offsets;
            for (const auto& g : s.supported_gpus) {
                gpu_offsets.push_back(mirage::fb::CreateGpuDef(
                    mb, mb.CreateString(g.name), mb.CreateString(g.arch),
                    static_cast<mirage::fb::GpuFamily>(g.family),
                    mb.CreateString(g.description)));
            }
            auto gpus = mb.CreateVector(gpu_offsets);
            std::vector<int8_t> mode_vals;
            for (auto m : s.supported_modes)
                mode_vals.push_back(static_cast<int8_t>(m));
            auto modes = mb.CreateVector(mode_vals);
            sim_offsets.push_back(fb::CreateSimulatorSummary(
                mb, nm, ver, desc, gpus, s.supports_custom_gpus, modes,
                s.active_session_count));
        }
        auto sims_vec = mb.CreateVector(sim_offsets);
        mb.Finish(fb::CreateListSimulatorsReply(mb, sims_vec));
        *response = mb.ReleaseMessage<fb::ListSimulatorsReply>();
        return ::grpc::Status::OK;
    }

    // ── GetSimulator ───────────────────────────────────────────────────
    ::grpc::Status GetSimulator(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::GetSimulatorRequest>* request,
        flatbuffers::grpc::Message<fb::GetSimulatorReply>* response) override {
        auto req = request->GetRoot();
        auto name = req->name() ? req->name()->str() : "";
        auto sim = svc_.get_simulator(name);
        flatbuffers::grpc::MessageBuilder mb;
        if (sim) {
            auto sim_off = build_simulator_summary(mb, *sim);
            mb.Finish(fb::CreateGetSimulatorReply(mb, sim_off));
        } else {
            mb.Finish(fb::CreateGetSimulatorReply(mb));
        }
        *response = mb.ReleaseMessage<fb::GetSimulatorReply>();
        return ::grpc::Status::OK;
    }

    // ── ListProfiles ───────────────────────────────────────────────────
    ::grpc::Status ListProfiles(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::ListProfilesRequest>* request,
        flatbuffers::grpc::Message<fb::ListProfilesReply>* response) override {
        auto req = request->GetRoot();
        auto filter = req->simulator_filter() ? req->simulator_filter()->str() : "";
        auto profiles = svc_.list_profiles(filter);
        flatbuffers::grpc::MessageBuilder mb;
        std::vector<flatbuffers::Offset<mirage::fb::ProfileDef>> offsets;
        for (const auto& p : profiles)
            offsets.push_back(build_profile(mb, p));
        mb.Finish(fb::CreateListProfilesReply(mb, mb.CreateVector(offsets)));
        *response = mb.ReleaseMessage<fb::ListProfilesReply>();
        return ::grpc::Status::OK;
    }

    // ── CreateProfile ──────────────────────────────────────────────────
    ::grpc::Status CreateProfile(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::CreateProfileRequest>* request,
        flatbuffers::grpc::Message<fb::CreateProfileReply>* response) override {
        auto req = request->GetRoot();
        mirage::ProfileDef p;
        if (auto fp = req->profile()) {
            p.name = fp->name() ? fp->name()->str() : "";
            p.simulator = fp->simulator() ? fp->simulator()->str() : "";
            p.mode = static_cast<mirage::SimulatorMode>(fp->mode());
            p.gpu = fp->gpu() ? fp->gpu()->str() : "";
            p.num_gpus = fp->num_gpus();
            p.num_nodes = fp->num_nodes();
        }
        auto result = svc_.create_profile(p);
        flatbuffers::grpc::MessageBuilder mb;
        mb.Finish(fb::CreateCreateProfileReply(mb, result.ok, mb.CreateString(result.error)));
        *response = mb.ReleaseMessage<fb::CreateProfileReply>();
        return ::grpc::Status::OK;
    }

    // ── DeleteProfile ──────────────────────────────────────────────────
    ::grpc::Status DeleteProfile(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::DeleteProfileRequest>* request,
        flatbuffers::grpc::Message<fb::DeleteProfileReply>* response) override {
        auto req = request->GetRoot();
        auto name = req->name() ? req->name()->str() : "";
        auto result = svc_.delete_profile(name);
        flatbuffers::grpc::MessageBuilder mb;
        mb.Finish(fb::CreateDeleteProfileReply(mb, result.ok, mb.CreateString(result.error)));
        *response = mb.ReleaseMessage<fb::DeleteProfileReply>();
        return ::grpc::Status::OK;
    }

    // ── ListSessions ───────────────────────────────────────────────────
    ::grpc::Status ListSessions(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::ListSessionsRequest>* request,
        flatbuffers::grpc::Message<fb::ListSessionsReply>* response) override {
        auto req = request->GetRoot();
        auto filter = req->profile_filter() ? req->profile_filter()->str() : "";
        auto sessions = docker_list_sessions();
        flatbuffers::grpc::MessageBuilder mb;
        std::vector<flatbuffers::Offset<fb::SessionSummary>> offsets;
        for (const auto& s : sessions) {
            if (!filter.empty() && s.profile != filter) continue;
            mirage::fb::HealthStatus hs = mirage::fb::HealthStatus_Unknown;
            if (s.status.find("Up") != std::string::npos)
                hs = mirage::fb::HealthStatus_Healthy;
            else if (s.status.find("Exited") != std::string::npos)
                hs = mirage::fb::HealthStatus_Unhealthy;
            offsets.push_back(fb::CreateSessionSummary(
                mb, mb.CreateString(s.name), mb.CreateString(s.profile),
                mb.CreateString(s.simulator), mb.CreateString(s.image),
                static_cast<mirage::fb::HealthStatus>(hs)));
        }
        mb.Finish(fb::CreateListSessionsReply(mb, mb.CreateVector(offsets)));
        *response = mb.ReleaseMessage<fb::ListSessionsReply>();
        return ::grpc::Status::OK;
    }

    // ── CreateSession ──────────────────────────────────────────────────
    ::grpc::Status CreateSession(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::DashboardCreateSessionRequest>* request,
        flatbuffers::grpc::Message<fb::DashboardCreateSessionReply>* response) override {
        auto req = request->GetRoot();
        mirage::SessionDef session;
        if (auto s = req->session()) {
            session.name = s->name() ? s->name()->str() : "";
            session.profile = s->profile() ? s->profile()->str() : "";
            session.image = s->image() ? s->image()->str() : "";
        }
        std::string container_name = "mirage-" + session.name;
        std::string check_out;
        int check_rc = exec_cmd(
            "docker inspect " + shell_escape(container_name) + " >/dev/null 2>&1", check_out);
        flatbuffers::grpc::MessageBuilder mb;
        if (check_rc == 0) {
            mb.Finish(fb::CreateDashboardCreateSessionReply(
                mb, false, mb.CreateString("session already exists")));
            *response = mb.ReleaseMessage<fb::DashboardCreateSessionReply>();
            return ::grpc::Status::OK;
        }
        session_logs_.create(session.name);
        mb.Finish(fb::CreateDashboardCreateSessionReply(mb, true, mb.CreateString("")));
        *response = mb.ReleaseMessage<fb::DashboardCreateSessionReply>();

        std::string sess_name = session.name;
        std::string sess_image = session.image.empty() ? "ubuntu:22.04" : session.image;
        std::thread([this, session, sess_name, sess_image]() {
            svc_.delete_session(sess_name);
            session_logs_.append(sess_name, "$ docker pull " + sess_image + "\n");
            {
                std::string pull_cmd = "docker pull " + shell_escape(sess_image) + " 2>&1";
                std::array<char, 4096> buf;
                FILE* pipe = popen(pull_cmd.c_str(), "r");
                if (pipe) {
                    // Use fread for streaming — docker pull writes \r progress
                    // that fgets would buffer until a full newline.
                    size_t n;
                    while ((n = fread(buf.data(), 1, buf.size(), pipe)) > 0)
                        session_logs_.append(sess_name, std::string(buf.data(), n));
                    int rc = pclose(pipe);
                    if (WEXITSTATUS(rc) != 0) {
                        session_logs_.append(sess_name,
                            "\n✗ Pull failed (exit " + std::to_string(WEXITSTATUS(rc)) + ")\n");
                        session_logs_.set_status(sess_name, "error");
                        return;
                    }
                } else {
                    session_logs_.append(sess_name, "\n✗ Failed to run docker pull\n");
                    session_logs_.set_status(sess_name, "error");
                    return;
                }
            }
            session_logs_.set_status(sess_name, "starting");
            session_logs_.append(sess_name, "\n$ docker run -d " + sess_image + "\n");
            auto result = svc_.create_session(session);
            if (result.ok) {
                // Capture the container ID
                std::string cid;
                exec_cmd("docker inspect --format '{{.Id}}' "
                         + shell_escape("mirage-" + sess_name) + " 2>/dev/null", cid);
                if (!cid.empty())
                    session_logs_.append(sess_name, cid.substr(0, 12) + "\n");
                session_logs_.append(sess_name, "✓ Container started\n");
                session_logs_.set_status(sess_name, "ready");
            } else {
                session_logs_.append(sess_name, "✗ Failed: " + result.error + "\n");
                session_logs_.set_status(sess_name, "error");
            }
        }).detach();
        return ::grpc::Status::OK;
    }

    // ── DeleteSession ──────────────────────────────────────────────────
    ::grpc::Status DeleteSession(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::DashboardDeleteSessionRequest>* request,
        flatbuffers::grpc::Message<fb::DashboardDeleteSessionReply>* response) override {
        auto req = request->GetRoot();
        auto name = req->name() ? req->name()->str() : "";
        svc_.delete_session(name);
        session_logs_.remove(name);
        std::string rm_out;
        int rc = exec_cmd("docker rm -f " + shell_escape("mirage-" + name) + " 2>&1", rm_out);
        flatbuffers::grpc::MessageBuilder mb;
        mb.Finish(fb::CreateDashboardDeleteSessionReply(
            mb, rc == 0, mb.CreateString(rc == 0 ? "" : "container not found")));
        *response = mb.ReleaseMessage<fb::DashboardDeleteSessionReply>();
        return ::grpc::Status::OK;
    }

    // ── GetSessionDetail ───────────────────────────────────────────────
    ::grpc::Status GetSessionDetail(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::GetSessionDetailRequest>* request,
        flatbuffers::grpc::Message<fb::GetSessionDetailReply>* response) override {
        auto req = request->GetRoot();
        auto name = req->name() ? req->name()->str() : "";
        bool found_in_docker = false;
        DockerSession docker_info;
        for (const auto& s : docker_list_sessions()) {
            if (s.name == name) { found_in_docker = true; docker_info = s; break; }
        }
        flatbuffers::grpc::MessageBuilder mb;
        if (!found_in_docker) {
            auto snap = session_logs_.get(name);
            if (!snap.status.empty() && snap.status != "ready" && snap.status != "error") {
                auto prof = mirage::fb::CreateProfileDef(
                    mb, mb.CreateString(""), mb.CreateString("rocjitsu"),
                    mirage::fb::SimulatorMode_Functional, mb.CreateString(""), 1, 1);
                mb.Finish(fb::CreateGetSessionDetailReply(
                    mb, mb.CreateString(name), prof,
                    mb.CreateString("rocjitsu"), mb.CreateString(""),
                    mirage::fb::HealthStatus_Unknown, 0,
                    mb.CreateString("Pulling image..."), 0, 0.0, 0.0, 0));
                *response = mb.ReleaseMessage<fb::GetSessionDetailReply>();
                return ::grpc::Status::OK;
            }
            auto detail = svc_.get_session_detail(name);
            if (!detail)
                return ::grpc::Status(::grpc::StatusCode::NOT_FOUND, "session not found");
            auto prof = build_profile(mb, detail->profile);
            mb.Finish(fb::CreateGetSessionDetailReply(
                mb, mb.CreateString(detail->name), prof,
                mb.CreateString(detail->simulator), mb.CreateString(detail->image),
                static_cast<mirage::fb::HealthStatus>(detail->health), 0,
                mb.CreateString(detail->error_message), detail->ticks,
                detail->ipc, detail->simulation_speed, detail->active_contexts));
            *response = mb.ReleaseMessage<fb::GetSessionDetailReply>();
            return ::grpc::Status::OK;
        }
        bool running = docker_info.status.find("Up") != std::string::npos;
        uint64_t uptime = running ? docker_container_uptime(name) : 0;
        auto prof = mirage::fb::CreateProfileDef(
            mb, mb.CreateString(docker_info.profile),
            mb.CreateString(docker_info.simulator),
            mirage::fb::SimulatorMode_Functional,
            mb.CreateString(docker_info.gpu), 1, 1);
        mb.Finish(fb::CreateGetSessionDetailReply(
            mb, mb.CreateString(name), prof,
            mb.CreateString(docker_info.simulator),
            mb.CreateString(docker_info.image),
            running ? mirage::fb::HealthStatus_Healthy : mirage::fb::HealthStatus_Unhealthy,
            0, mb.CreateString(running ? "" : "Container " + docker_info.status),
            running ? (uptime + 1) * 2400000 : 0,
            running ? 1.85 : 0.0, running ? 0.42 : 0.0,
            running ? 64u : 0u));
        *response = mb.ReleaseMessage<fb::GetSessionDetailReply>();
        return ::grpc::Status::OK;
    }

    // ── ListRuns ───────────────────────────────────────────────────────
    ::grpc::Status ListRuns(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::ListRunsRequest>* request,
        flatbuffers::grpc::Message<fb::ListRunsReply>* response) override {
        auto req = request->GetRoot();
        auto filter = req->session_filter() ? req->session_filter()->str() : "";
        auto all_runs = runs_.list_runs(filter);
        flatbuffers::grpc::MessageBuilder mb;
        std::vector<flatbuffers::Offset<fb::RunRecord>> offsets;
        for (const auto& r : all_runs) offsets.push_back(build_run_record(mb, r));
        mb.Finish(fb::CreateListRunsReply(mb, mb.CreateVector(offsets)));
        *response = mb.ReleaseMessage<fb::ListRunsReply>();
        return ::grpc::Status::OK;
    }

    // ── CreateRun ──────────────────────────────────────────────────────
    ::grpc::Status CreateRun(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::CreateRunRequest>* request,
        flatbuffers::grpc::Message<fb::CreateRunReply>* response) override {
        auto req = request->GetRoot();
        auto session = req->session() ? req->session()->str() : "";
        auto command = req->command() ? req->command()->str() : "";
        flatbuffers::grpc::MessageBuilder mb;
        if (session.empty() || command.empty()) {
            mb.Finish(fb::CreateCreateRunReply(mb, false, mb.CreateString("session and command required")));
            *response = mb.ReleaseMessage<fb::CreateRunReply>();
            return ::grpc::Status::OK;
        }
        if (!docker_container_running(session)) {
            mb.Finish(fb::CreateCreateRunReply(mb, false, mb.CreateString("session container not running")));
            *response = mb.ReleaseMessage<fb::CreateRunReply>();
            return ::grpc::Status::OK;
        }
        auto rec = runs_.start_run(session, command);
        auto run_off = build_run_record(mb, rec);
        mb.Finish(fb::CreateCreateRunReply(mb, true, mb.CreateString(""), run_off));
        *response = mb.ReleaseMessage<fb::CreateRunReply>();
        return ::grpc::Status::OK;
    }

    // ── ListTerminals ──────────────────────────────────────────────────
    ::grpc::Status ListTerminals(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::ListTerminalsRequest>*,
        flatbuffers::grpc::Message<fb::ListTerminalsReply>* response) override {
        auto all = terms_.list();
        flatbuffers::grpc::MessageBuilder mb;
        std::vector<flatbuffers::Offset<fb::TerminalInfo>> offsets;
        for (const auto& t : all)
            offsets.push_back(fb::CreateTerminalInfo(
                mb, mb.CreateString(t.id), mb.CreateString(t.session), t.alive));
        mb.Finish(fb::CreateListTerminalsReply(mb, mb.CreateVector(offsets)));
        *response = mb.ReleaseMessage<fb::ListTerminalsReply>();
        return ::grpc::Status::OK;
    }

    // ── CreateTerminal ─────────────────────────────────────────────────
    ::grpc::Status CreateTerminal(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::CreateTerminalRequest>* request,
        flatbuffers::grpc::Message<fb::CreateTerminalReply>* response) override {
        auto req = request->GetRoot();
        auto session = req->session() ? req->session()->str() : "";
        flatbuffers::grpc::MessageBuilder mb;
        if (session.empty()) {
            mb.Finish(fb::CreateCreateTerminalReply(mb, false, mb.CreateString("session required")));
            *response = mb.ReleaseMessage<fb::CreateTerminalReply>();
            return ::grpc::Status::OK;
        }
        auto result = terms_.create(session);
        mb.Finish(fb::CreateCreateTerminalReply(
            mb, result.ok, mb.CreateString(result.error), mb.CreateString(result.id)));
        *response = mb.ReleaseMessage<fb::CreateTerminalReply>();
        return ::grpc::Status::OK;
    }

    // ── TerminalInput ──────────────────────────────────────────────────
    ::grpc::Status TerminalInput(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::TerminalInputRequest>* request,
        flatbuffers::grpc::Message<fb::TerminalInputReply>* response) override {
        auto req = request->GetRoot();
        auto id = req->id() ? req->id()->str() : "";
        auto data_b64 = req->data() ? req->data()->str() : "";
        bool ok = terms_.write_input(id, base64_decode(data_b64));
        flatbuffers::grpc::MessageBuilder mb;
        mb.Finish(fb::CreateTerminalInputReply(mb, ok));
        *response = mb.ReleaseMessage<fb::TerminalInputReply>();
        return ::grpc::Status::OK;
    }

    // ── TerminalOutput ─────────────────────────────────────────────────
    ::grpc::Status TerminalOutput(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::TerminalOutputRequest>* request,
        flatbuffers::grpc::Message<fb::TerminalOutputReply>* response) override {
        auto req = request->GetRoot();
        auto id = req->id() ? req->id()->str() : "";
        auto raw = terms_.read_output(id);
        bool alive = terms_.is_alive(id);
        flatbuffers::grpc::MessageBuilder mb;
        mb.Finish(fb::CreateTerminalOutputReply(mb, mb.CreateString(base64_encode(raw)), alive));
        *response = mb.ReleaseMessage<fb::TerminalOutputReply>();
        return ::grpc::Status::OK;
    }

    // ── TerminalResize ─────────────────────────────────────────────────
    ::grpc::Status TerminalResize(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::TerminalResizeRequest>* request,
        flatbuffers::grpc::Message<fb::TerminalResizeReply>* response) override {
        auto req = request->GetRoot();
        auto id = req->id() ? req->id()->str() : "";
        bool ok = terms_.resize(id, static_cast<uint16_t>(req->rows()),
                                static_cast<uint16_t>(req->cols()));
        flatbuffers::grpc::MessageBuilder mb;
        mb.Finish(fb::CreateTerminalResizeReply(mb, ok));
        *response = mb.ReleaseMessage<fb::TerminalResizeReply>();
        return ::grpc::Status::OK;
    }

    // ── CloseTerminal ──────────────────────────────────────────────────
    ::grpc::Status CloseTerminal(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::CloseTerminalRequest>* request,
        flatbuffers::grpc::Message<fb::CloseTerminalReply>* response) override {
        auto req = request->GetRoot();
        auto id = req->id() ? req->id()->str() : "";
        bool ok = terms_.close_terminal(id);
        flatbuffers::grpc::MessageBuilder mb;
        mb.Finish(fb::CreateCloseTerminalReply(mb, ok));
        *response = mb.ReleaseMessage<fb::CloseTerminalReply>();
        return ::grpc::Status::OK;
    }

    // ── GetSessionLog ──────────────────────────────────────────────────
    ::grpc::Status GetSessionLog(
        ::grpc::ServerContext*,
        const flatbuffers::grpc::Message<fb::GetSessionLogRequest>* request,
        flatbuffers::grpc::Message<fb::GetSessionLogReply>* response) override {
        auto req = request->GetRoot();
        auto name = req->name() ? req->name()->str() : "";
        auto snap = session_logs_.get(name);

        // If no log entry exists, check Docker for a running container
        // and synthesize a log so it's always available for live sessions.
        if (snap.status.empty()) {
            for (const auto& ds : docker_list_sessions()) {
                if (ds.name == name) {
                    bool running = ds.status.find("Up") != std::string::npos;
                    snap.status = running ? "ready" : "error";
                    snap.log = "Container " + ds.container_id.substr(0, 12)
                               + " (" + ds.image + ")\n"
                               + "Status: " + ds.status + "\n";
                    break;
                }
            }
        }

        flatbuffers::grpc::MessageBuilder mb;
        mb.Finish(fb::CreateGetSessionLogReply(
            mb, mb.CreateString(snap.log), mb.CreateString(snap.status)));
        *response = mb.ReleaseMessage<fb::GetSessionLogReply>();
        return ::grpc::Status::OK;
    }

private:
    flatbuffers::Offset<fb::SimulatorSummary>
    build_simulator_summary(flatbuffers::grpc::MessageBuilder& mb,
                            const mirage::SimulatorSummary& s) {
        std::vector<flatbuffers::Offset<mirage::fb::GpuDef>> gpu_offsets;
        for (const auto& g : s.supported_gpus)
            gpu_offsets.push_back(mirage::fb::CreateGpuDef(
                mb, mb.CreateString(g.name), mb.CreateString(g.arch),
                static_cast<mirage::fb::GpuFamily>(g.family),
                mb.CreateString(g.description)));
        std::vector<int8_t> mode_vals;
        for (auto m : s.supported_modes)
            mode_vals.push_back(static_cast<int8_t>(m));
        return fb::CreateSimulatorSummary(
            mb, mb.CreateString(s.name), mb.CreateString(s.version),
            mb.CreateString(s.description), mb.CreateVector(gpu_offsets),
            s.supports_custom_gpus, mb.CreateVector(mode_vals),
            s.active_session_count);
    }

    flatbuffers::Offset<mirage::fb::ProfileDef>
    build_profile(flatbuffers::grpc::MessageBuilder& mb,
                  const mirage::ProfileDef& p) {
        return mirage::fb::CreateProfileDef(
            mb, mb.CreateString(p.name), mb.CreateString(p.simulator),
            static_cast<mirage::fb::SimulatorMode>(p.mode),
            mb.CreateString(p.gpu), p.num_gpus, p.num_nodes);
    }

    flatbuffers::Offset<fb::RunRecord>
    build_run_record(flatbuffers::grpc::MessageBuilder& mb,
                     const ::RunRecord& r) {
        return fb::CreateRunRecord(
            mb, mb.CreateString(r.id), mb.CreateString(r.session),
            mb.CreateString(r.command), mb.CreateString(r.status),
            r.exit_code, mb.CreateString(r.output));
    }

    mirage::DashboardService& svc_;
    RunTracker& runs_;
    TerminalManager& terms_;
    SessionLogStore& session_logs_;
};

// ============================================================================
//  gRPC-Web binary bridge (httplib → gRPC service)
// ============================================================================

using GrpcWebHandler = std::function<std::string(const std::string& body)>;

std::unordered_map<std::string, GrpcWebHandler>
build_dispatch(MirageDashboardService& svc) {
    std::unordered_map<std::string, GrpcWebHandler> m;

#define GRPC_WEB_RPC(Method, Req, Reply) \
    m[#Method] = [&svc](const std::string& body) -> std::string { \
        auto slice = ::grpc::Slice(body.data(), body.size()); \
        flatbuffers::grpc::Message<fb::Req> request(slice); \
        flatbuffers::grpc::Message<fb::Reply> response; \
        auto status = svc.Method(nullptr, &request, &response); \
        if (!status.ok()) return ""; \
        return std::string(reinterpret_cast<const char*>(response.data()), \
                           response.size()); \
    }

    GRPC_WEB_RPC(GetOverview, GetOverviewRequest, GetOverviewReply);
    GRPC_WEB_RPC(ListSimulators, ListSimulatorsRequest, ListSimulatorsReply);
    GRPC_WEB_RPC(GetSimulator, GetSimulatorRequest, GetSimulatorReply);
    GRPC_WEB_RPC(ListProfiles, ListProfilesRequest, ListProfilesReply);
    GRPC_WEB_RPC(CreateProfile, CreateProfileRequest, CreateProfileReply);
    GRPC_WEB_RPC(DeleteProfile, DeleteProfileRequest, DeleteProfileReply);
    GRPC_WEB_RPC(ListSessions, ListSessionsRequest, ListSessionsReply);
    GRPC_WEB_RPC(CreateSession, DashboardCreateSessionRequest, DashboardCreateSessionReply);
    GRPC_WEB_RPC(DeleteSession, DashboardDeleteSessionRequest, DashboardDeleteSessionReply);
    GRPC_WEB_RPC(GetSessionDetail, GetSessionDetailRequest, GetSessionDetailReply);
    GRPC_WEB_RPC(ListRuns, ListRunsRequest, ListRunsReply);
    GRPC_WEB_RPC(CreateRun, CreateRunRequest, CreateRunReply);
    GRPC_WEB_RPC(ListTerminals, ListTerminalsRequest, ListTerminalsReply);
    GRPC_WEB_RPC(CreateTerminal, CreateTerminalRequest, CreateTerminalReply);
    GRPC_WEB_RPC(TerminalInput, TerminalInputRequest, TerminalInputReply);
    GRPC_WEB_RPC(TerminalOutput, TerminalOutputRequest, TerminalOutputReply);
    GRPC_WEB_RPC(TerminalResize, TerminalResizeRequest, TerminalResizeReply);
    GRPC_WEB_RPC(CloseTerminal, CloseTerminalRequest, CloseTerminalReply);
    GRPC_WEB_RPC(GetSessionLog, GetSessionLogRequest, GetSessionLogReply);

#undef GRPC_WEB_RPC
    return m;
}

} // namespace

int main(int argc, char* argv[]) {
    int http_port = 50051;
    int grpc_port = 50052;
    std::string static_dir;

    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if ((arg == "--port" || arg == "-p") && i + 1 < argc)
            http_port = std::atoi(argv[++i]);
        else if (arg == "--grpc-port" && i + 1 < argc)
            grpc_port = std::atoi(argv[++i]);
        else if (arg == "--static" && i + 1 < argc)
            static_dir = argv[++i];
    }

    mirage::Daemon daemon;
    daemon.register_simulator(std::make_shared<DummySimulator>());
    daemon.add_profile({"mi300x-functional", "rocjitsu",
                         mirage::SimulatorMode::Functional, "MI300X", 1, 1});
    daemon.add_profile({"mi300x-8gpu-cycle", "rocjitsu",
                         mirage::SimulatorMode::CycleAccurate, "MI300X", 8, 1});
    daemon.add_profile({"mi325x-clocked", "rocjitsu",
                         mirage::SimulatorMode::Clocked, "MI325X", 4, 2});

    mirage::DashboardService svc(daemon);
    RunTracker runs;
    TerminalManager terms;
    SessionLogStore session_logs;

    // ── gRPC service (autogenerated Dashboard::Service) ────────────────
    MirageDashboardService grpc_service(svc, runs, terms, session_logs);

    // Start native gRPC server in background
    std::unique_ptr<::grpc::Server> grpc_server;
    std::thread grpc_thread([&grpc_service, &grpc_server, grpc_port]() {
        std::string addr = "0.0.0.0:" + std::to_string(grpc_port);
        ::grpc::ServerBuilder builder;
        builder.AddListeningPort(addr, ::grpc::InsecureServerCredentials());
        builder.RegisterService(&grpc_service);
        grpc_server = builder.BuildAndStart();
        if (!grpc_server) {
            std::cerr << "Failed to start gRPC server on " << addr << std::endl;
            return;
        }
        std::cout << "gRPC server listening on " << addr << std::endl;
        grpc_server->Wait();
    });
    grpc_thread.detach();

    // ── gRPC-Web binary bridge (httplib) ───────────────────────────────
    auto dispatch = build_dispatch(grpc_service);
    httplib::Server srv;

    srv.set_default_headers({
        {"Access-Control-Allow-Origin", "*"},
        {"Access-Control-Allow-Methods", "POST, OPTIONS"},
        {"Access-Control-Allow-Headers", "Content-Type, X-Grpc-Web"},
    });

    const std::string prefix = "/mirage.simulator.Dashboard/";

    srv.Post(prefix + "(.*)",
             [&dispatch, &prefix](const httplib::Request& req,
                                  httplib::Response& res) {
                 auto method = req.path.substr(prefix.size());
                 auto it = dispatch.find(method);
                 if (it == dispatch.end()) {
                     res.status = 404;
                     res.set_content("unknown method: " + method, "text/plain");
                     return;
                 }
                 auto result = it->second(req.body);
                 if (result.empty()) {
                     res.status = 500;
                     res.set_content("internal error", "text/plain");
                     return;
                 }
                 res.set_content(result, "application/x-flatbuffers");
             });

    srv.Options("/(.*)", [](const httplib::Request&, httplib::Response& res) {
        res.status = 204;
    });

    if (!static_dir.empty()) {
        srv.set_mount_point("/", static_dir);
        srv.set_error_handler(
            [&static_dir](const httplib::Request& req, httplib::Response& res) {
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

    std::cout << "gRPC-Web bridge listening on http://0.0.0.0:" << http_port << std::endl;
    if (!srv.listen("0.0.0.0", http_port)) {
        std::cerr << "Failed to start HTTP server on port " << http_port << std::endl;
        return 1;
    }
    return 0;
}
