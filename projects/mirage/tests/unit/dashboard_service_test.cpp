#include "mirage/dashboard_service.h"

#include "../mock_simulator.h"

#include <gtest/gtest.h>

namespace mirage {
namespace {

using ::mirage::testing::make_mock_simulator;
using ::mirage::testing::make_test_profile;
using ::mirage::testing::make_test_session;

class DashboardServiceTest : public ::testing::Test {
protected:
    void SetUp() override {
        sim_ = make_mock_simulator("sim-alpha");
        daemon_.register_simulator(sim_);
    }

    Daemon daemon_;
    std::shared_ptr<mirage::testing::MockSimulator> sim_;
};

// ── Overview ────────────────────────────────────────────────────────────────

TEST_F(DashboardServiceTest, OverviewEmpty) {
    DashboardService svc(daemon_);
    auto ov = svc.get_overview();
    EXPECT_EQ(ov.simulator_count, 1);
    EXPECT_EQ(ov.profile_count, 0);
    EXPECT_EQ(ov.session_count, 0);
}

TEST_F(DashboardServiceTest, OverviewWithData) {
    daemon_.add_profile(make_test_profile("p1", "sim-alpha"));
    daemon_.add_profile(make_test_profile("p2", "sim-alpha"));
    daemon_.create_session(make_test_session("s1", "p1"));

    DashboardService svc(daemon_);
    auto ov = svc.get_overview();
    EXPECT_EQ(ov.simulator_count, 1);
    EXPECT_EQ(ov.profile_count, 2);
    EXPECT_EQ(ov.session_count, 1);
}

// ── Simulators ──────────────────────────────────────────────────────────────

TEST_F(DashboardServiceTest, ListSimulatorsReturnsAll) {
    auto sim2 = make_mock_simulator("sim-beta");
    daemon_.register_simulator(sim2);

    DashboardService svc(daemon_);
    auto sims = svc.list_simulators();
    EXPECT_EQ(sims.size(), 2);

    bool found_alpha = false, found_beta = false;
    for (const auto& s : sims) {
        if (s.name == "sim-alpha") found_alpha = true;
        if (s.name == "sim-beta") found_beta = true;
    }
    EXPECT_TRUE(found_alpha);
    EXPECT_TRUE(found_beta);
}

TEST_F(DashboardServiceTest, ListSimulatorsIncludesSessionCount) {
    daemon_.add_profile(make_test_profile("p1", "sim-alpha"));
    daemon_.create_session(make_test_session("s1", "p1"));
    daemon_.create_session(make_test_session("s2", "p1"));

    DashboardService svc(daemon_);
    auto sims = svc.list_simulators();
    ASSERT_EQ(sims.size(), 1);
    EXPECT_EQ(sims[0].active_session_count, 2);
}

TEST_F(DashboardServiceTest, GetSimulatorFound) {
    DashboardService svc(daemon_);
    auto result = svc.get_simulator("sim-alpha");
    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(result->name, "sim-alpha");
    EXPECT_EQ(result->version, "1.0.0");
    EXPECT_EQ(result->supported_gpus.size(), 2);
}

TEST_F(DashboardServiceTest, GetSimulatorNotFound) {
    DashboardService svc(daemon_);
    auto result = svc.get_simulator("nonexistent");
    EXPECT_FALSE(result.has_value());
}

TEST_F(DashboardServiceTest, SimulatorSummaryFields) {
    DashboardService svc(daemon_);
    auto result = svc.get_simulator("sim-alpha");
    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(result->description, "Test simulator");
    EXPECT_TRUE(result->supports_custom_gpus);
    EXPECT_EQ(result->supported_modes.size(), 2);
}

// ── Profiles ────────────────────────────────────────────────────────────────

TEST_F(DashboardServiceTest, ListProfilesEmpty) {
    DashboardService svc(daemon_);
    EXPECT_TRUE(svc.list_profiles().empty());
}

TEST_F(DashboardServiceTest, CreateAndListProfiles) {
    DashboardService svc(daemon_);
    auto r = svc.create_profile(make_test_profile("p1", "sim-alpha"));
    EXPECT_TRUE(r.ok);

    auto profiles = svc.list_profiles();
    ASSERT_EQ(profiles.size(), 1);
    EXPECT_EQ(profiles[0].name, "p1");
    EXPECT_EQ(profiles[0].simulator, "sim-alpha");
}

TEST_F(DashboardServiceTest, CreateDuplicateProfile) {
    DashboardService svc(daemon_);
    svc.create_profile(make_test_profile("p1", "sim-alpha"));
    auto r = svc.create_profile(make_test_profile("p1", "sim-alpha"));
    EXPECT_FALSE(r.ok);
    EXPECT_FALSE(r.error.empty());
}

TEST_F(DashboardServiceTest, DeleteProfile) {
    DashboardService svc(daemon_);
    svc.create_profile(make_test_profile("p1", "sim-alpha"));
    auto r = svc.delete_profile("p1");
    EXPECT_TRUE(r.ok);
    EXPECT_TRUE(svc.list_profiles().empty());
}

TEST_F(DashboardServiceTest, DeleteNonexistentProfile) {
    DashboardService svc(daemon_);
    auto r = svc.delete_profile("nope");
    EXPECT_FALSE(r.ok);
}

TEST_F(DashboardServiceTest, ListProfilesFilterBySimulator) {
    auto sim2 = make_mock_simulator("sim-beta");
    daemon_.register_simulator(sim2);
    DashboardService svc(daemon_);
    svc.create_profile(make_test_profile("p1", "sim-alpha"));
    svc.create_profile(make_test_profile("p2", "sim-beta"));

    auto filtered = svc.list_profiles("sim-alpha");
    ASSERT_EQ(filtered.size(), 1);
    EXPECT_EQ(filtered[0].name, "p1");

    auto all = svc.list_profiles();
    EXPECT_EQ(all.size(), 2);
}

// ── Sessions ────────────────────────────────────────────────────────────────

TEST_F(DashboardServiceTest, ListSessionsEmpty) {
    DashboardService svc(daemon_);
    EXPECT_TRUE(svc.list_sessions().empty());
}

TEST_F(DashboardServiceTest, CreateAndListSessions) {
    daemon_.add_profile(make_test_profile("p1", "sim-alpha"));
    DashboardService svc(daemon_);
    auto r = svc.create_session(make_test_session("s1", "p1"));
    EXPECT_TRUE(r.ok);

    auto sessions = svc.list_sessions();
    ASSERT_EQ(sessions.size(), 1);
    EXPECT_EQ(sessions[0].name, "s1");
    EXPECT_EQ(sessions[0].profile, "p1");
    EXPECT_EQ(sessions[0].simulator, "sim-alpha");
}

TEST_F(DashboardServiceTest, SessionSummaryHealth) {
    daemon_.add_profile(make_test_profile("p1", "sim-alpha"));
    DashboardService svc(daemon_);
    svc.create_session(make_test_session("s1", "p1"));

    auto sessions = svc.list_sessions();
    ASSERT_EQ(sessions.size(), 1);
    EXPECT_EQ(sessions[0].health_status, HealthStatus::Healthy);
}

TEST_F(DashboardServiceTest, CreateSessionInvalidProfile) {
    DashboardService svc(daemon_);
    auto r = svc.create_session(make_test_session("s1", "missing-profile"));
    EXPECT_FALSE(r.ok);
    EXPECT_FALSE(r.error.empty());
}

TEST_F(DashboardServiceTest, DeleteSession) {
    daemon_.add_profile(make_test_profile("p1", "sim-alpha"));
    DashboardService svc(daemon_);
    svc.create_session(make_test_session("s1", "p1"));

    auto r = svc.delete_session("s1");
    EXPECT_TRUE(r.ok);
    EXPECT_TRUE(svc.list_sessions().empty());
}

TEST_F(DashboardServiceTest, DeleteNonexistentSession) {
    DashboardService svc(daemon_);
    auto r = svc.delete_session("nope");
    EXPECT_FALSE(r.ok);
}

TEST_F(DashboardServiceTest, ListSessionsFilterByProfile) {
    daemon_.add_profile(make_test_profile("p1", "sim-alpha"));
    daemon_.add_profile(make_test_profile("p2", "sim-alpha"));
    DashboardService svc(daemon_);
    svc.create_session(make_test_session("s1", "p1"));
    svc.create_session(make_test_session("s2", "p2"));

    auto filtered = svc.list_sessions("p1");
    ASSERT_EQ(filtered.size(), 1);
    EXPECT_EQ(filtered[0].name, "s1");
}

// ── Session Detail ──────────────────────────────────────────────────────────

TEST_F(DashboardServiceTest, GetSessionDetailFound) {
    daemon_.add_profile(make_test_profile("p1", "sim-alpha"));
    DashboardService svc(daemon_);
    svc.create_session(make_test_session("s1", "p1"));

    auto detail = svc.get_session_detail("s1");
    ASSERT_TRUE(detail.has_value());
    EXPECT_EQ(detail->name, "s1");
    EXPECT_EQ(detail->simulator, "sim-alpha");
    EXPECT_EQ(detail->profile.name, "p1");
    EXPECT_EQ(detail->health, HealthStatus::Healthy);
    EXPECT_EQ(detail->uptime.seconds, 42);
    EXPECT_EQ(detail->ticks, 1000);
    EXPECT_DOUBLE_EQ(detail->ipc, 2.5);
    EXPECT_DOUBLE_EQ(detail->simulation_speed, 0.8);
    EXPECT_EQ(detail->active_contexts, 64);
}

TEST_F(DashboardServiceTest, GetSessionDetailNotFound) {
    DashboardService svc(daemon_);
    auto detail = svc.get_session_detail("nonexistent");
    EXPECT_FALSE(detail.has_value());
}

TEST_F(DashboardServiceTest, GetSessionDetailImage) {
    daemon_.add_profile(make_test_profile("p1", "sim-alpha"));
    DashboardService svc(daemon_);
    svc.create_session(make_test_session("s1", "p1"));

    auto detail = svc.get_session_detail("s1");
    ASSERT_TRUE(detail.has_value());
    EXPECT_EQ(detail->image, "ghcr.io/rocm/pytorch:latest");
}

// ── Multi-simulator scenario ────────────────────────────────────────────────

TEST_F(DashboardServiceTest, MultiSimulatorWorkflow) {
    auto sim2 = make_mock_simulator("sim-beta");
    daemon_.register_simulator(sim2);
    DashboardService svc(daemon_);

    svc.create_profile(make_test_profile("p-alpha", "sim-alpha"));
    svc.create_profile(make_test_profile("p-beta", "sim-beta", "MI325X"));

    svc.create_session(make_test_session("s-alpha", "p-alpha"));
    svc.create_session(make_test_session("s-beta", "p-beta"));

    auto ov = svc.get_overview();
    EXPECT_EQ(ov.simulator_count, 2);
    EXPECT_EQ(ov.profile_count, 2);
    EXPECT_EQ(ov.session_count, 2);

    auto sims = svc.list_simulators();
    EXPECT_EQ(sims.size(), 2);

    auto sessions = svc.list_sessions();
    EXPECT_EQ(sessions.size(), 2);
}

} // namespace
} // namespace mirage
