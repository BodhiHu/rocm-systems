"""Extensive integration tests for vLLM benchmarking with real Mirage daemon + rocjitsu.

Every test starts a real mirage_daemon process, communicates through a real
mirage_ctl binary, parses real JSON responses, and validates the full
Mirage + rocjitsu vLLM benchmark workflow.
"""

import argparse
import json
import os
import signal
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]  # rocm-systems/
DAEMON_BIN = REPO_ROOT / "target" / "debug" / "mirage_daemon"
CTL_BIN = REPO_ROOT / "target" / "debug" / "mirage_ctl"

VLLM_IMAGE = (
    "docker.io/rocm/vllm:"
    "rocm7.12.0_gfx94X-dcgpu_ubuntu24.04_py3.12_pytorch_2.9.1_vllm_0.16.0"
)

import sys
sys.path.insert(0, str(REPO_ROOT))
from emulation.demo import vllm_benchmark


def _ensure_binaries():
    """Build daemon + ctl if not already present."""
    if DAEMON_BIN.exists() and CTL_BIN.exists():
        return
    subprocess.run(
        ["cargo", "build", "--quiet", "--bin", "mirage_daemon", "--bin", "mirage_ctl"],
        cwd=REPO_ROOT,
        check=True,
    )


class MirageTestDaemon:
    """Context manager that starts a real mirage_daemon on a temp socket."""

    def __init__(self):
        self.socket_path = tempfile.mktemp(prefix="mirage-test-", suffix=".sock")
        self.proc = None

    def __enter__(self):
        _ensure_binaries()
        self.proc = subprocess.Popen(
            [str(DAEMON_BIN), "--socket", self.socket_path],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.time() + 5.0
        while time.time() < deadline:
            if os.path.exists(self.socket_path):
                break
            time.sleep(0.05)
        if not os.path.exists(self.socket_path):
            self.proc.kill()
            raise RuntimeError("daemon did not create socket within 5 s")
        return self

    def __exit__(self, *_):
        if self.proc and self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        try:
            os.remove(self.socket_path)
        except OSError:
            pass

    def ctl(self, *args: str) -> dict:
        """Run mirage_ctl --json and return the parsed JSON reply dict."""
        cmd = [str(CTL_BIN), "--socket", self.socket_path, "--json", *args]
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
        if result.returncode != 0:
            raise RuntimeError(
                f"ctl {' '.join(args)} failed (rc={result.returncode}): "
                f"{result.stderr.strip()}"
            )
        return json.loads(result.stdout)

    def ctl_raw(self, *args: str) -> subprocess.CompletedProcess:
        """Run mirage_ctl and return the raw CompletedProcess."""
        cmd = [str(CTL_BIN), "--socket", self.socket_path, "--json", *args]
        return subprocess.run(cmd, capture_output=True, text=True, timeout=15)


# ============================================================================
# 1. Daemon startup and overview
# ============================================================================


class TestDaemonStartup(unittest.TestCase):
    def test_daemon_starts_and_responds(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("get-overview")
            overview = reply["GetOverview"]
            self.assertGreaterEqual(overview["simulator_count"], 1)
            self.assertEqual(overview["profile_count"], 0)
            self.assertEqual(overview["session_count"], 0)

    def test_overview_counts_update_after_profile_creation(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "p1",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            overview = d.ctl("get-overview")["GetOverview"]
            self.assertEqual(overview["profile_count"], 1)


# ============================================================================
# 2. Rocjitsu simulator registration and GPU support
# ============================================================================


class TestRocjitsuSimulator(unittest.TestCase):
    def test_rocjitsu_is_listed(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("list-simulators")
            sims = reply["ListSimulators"]["simulators"]
            names = [s["name"] for s in sims]
            self.assertIn("rocjitsu", names)

    def test_rocjitsu_show_details(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("show-simulator", "--name", "rocjitsu")
            sim = reply["ShowSimulator"]["simulator"]
            self.assertEqual(sim["name"], "rocjitsu")
            self.assertEqual(sim["version"], "0.5.0")
            self.assertFalse(sim["supports_custom_gpus"])
            self.assertIn("functional", sim["supported_modes"])

    def test_rocjitsu_supports_mi300x_mi325x_mi350x(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("show-simulator", "--name", "rocjitsu")
            gpus = reply["ShowSimulator"]["simulator"]["supported_gpus"]
            gpu_names = [g["name"] for g in gpus]
            self.assertIn("MI300X", gpu_names)
            self.assertIn("MI325X", gpu_names)
            self.assertIn("MI350X", gpu_names)

    def test_rocjitsu_gpu_architectures(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("show-simulator", "--name", "rocjitsu")
            gpus = reply["ShowSimulator"]["simulator"]["supported_gpus"]
            by_name = {g["name"]: g for g in gpus}
            self.assertEqual(by_name["MI300X"]["arch"], "gfx942")
            self.assertEqual(by_name["MI325X"]["arch"], "gfx942")
            self.assertEqual(by_name["MI350X"]["arch"], "gfx950")
            for g in gpus:
                self.assertEqual(g["family"], "amd_cdna")

    def test_rocjitsu_active_session_count_starts_at_zero(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("show-simulator", "--name", "rocjitsu")
            self.assertEqual(
                reply["ShowSimulator"]["simulator"]["active_session_count"], 0
            )

    def test_show_nonexistent_simulator_returns_empty(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("show-simulator", "--name", "gem5")
            self.assertNotIn("simulator", reply.get("ShowSimulator", {}))


# ============================================================================
# 3. Profile management for vLLM
# ============================================================================


class TestProfileManagement(unittest.TestCase):
    def test_create_mi300x_profile(self):
        with MirageTestDaemon() as d:
            reply = d.ctl(
                "create-profile",
                "--name", "mi300x-func",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "8",
                "--nodes", "1",
            )
            self.assertTrue(reply["CreateProfile"]["ok"])

    def test_create_multiple_gpu_profiles(self):
        with MirageTestDaemon() as d:
            for name, gpu in [("p300", "MI300X"), ("p325", "MI325X"), ("p350", "MI350X")]:
                reply = d.ctl(
                    "create-profile",
                    "--name", name,
                    "--simulator", "rocjitsu",
                    "--gpu", gpu,
                    "--mode", "functional",
                    "--gpus-per-node", "8",
                    "--nodes", "1",
                )
                self.assertTrue(reply["CreateProfile"]["ok"], f"failed for {name}")

            profiles = d.ctl("list-profiles", "--simulator", "rocjitsu")
            self.assertEqual(len(profiles["ListProfiles"]["profiles"]), 3)

    def test_list_profiles_returns_correct_fields(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "mi300x-func",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "8",
                "--nodes", "1",
            )
            profiles = d.ctl("list-profiles", "--simulator", "rocjitsu")
            p = profiles["ListProfiles"]["profiles"][0]
            self.assertEqual(p["name"], "mi300x-func")
            self.assertEqual(p["simulator"], "rocjitsu")
            self.assertEqual(p["gpu"], "MI300X")
            self.assertEqual(p["mode"], "functional")
            self.assertEqual(p["num_gpus"], 8)
            self.assertEqual(p["num_nodes"], 1)

    def test_create_profile_rejects_unsupported_gpu(self):
        with MirageTestDaemon() as d:
            reply = d.ctl(
                "create-profile",
                "--name", "bad-gpu",
                "--simulator", "rocjitsu",
                "--gpu", "RTX4090",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            self.assertFalse(reply["CreateProfile"]["ok"])
            self.assertIn("RTX4090", reply["CreateProfile"]["error"])

    def test_create_profile_rejects_empty_name(self):
        with MirageTestDaemon() as d:
            reply = d.ctl(
                "create-profile",
                "--name", "",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            self.assertFalse(reply["CreateProfile"]["ok"])
            self.assertIn("empty", reply["CreateProfile"]["error"])

    def test_create_duplicate_profile_fails(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "dup",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            reply = d.ctl(
                "create-profile",
                "--name", "dup",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            self.assertFalse(reply["CreateProfile"]["ok"])
            self.assertIn("already exists", reply["CreateProfile"]["error"])

    def test_delete_profile(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "deletable",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            reply = d.ctl("delete-profile", "--name", "deletable")
            self.assertTrue(reply["DeleteProfile"]["ok"])

            profiles = d.ctl("list-profiles", "--simulator", "rocjitsu")
            self.assertEqual(len(profiles["ListProfiles"]["profiles"]), 0)

    def test_delete_nonexistent_profile_fails(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("delete-profile", "--name", "ghost")
            self.assertFalse(reply["DeleteProfile"]["ok"])

    def test_profile_filter_by_simulator(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "rj-prof",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            reply = d.ctl("list-profiles", "--simulator", "rocjitsu")
            self.assertEqual(len(reply["ListProfiles"]["profiles"]), 1)

            reply = d.ctl("list-profiles", "--simulator", "gem5")
            self.assertEqual(len(reply["ListProfiles"]["profiles"]), 0)


# ============================================================================
# 4. Workload definitions for vLLM benchmarks
# ============================================================================


class TestWorkloadManagement(unittest.TestCase):
    def test_create_vllm_benchmark_workload(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "mi300x-func",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "8",
                "--nodes", "1",
            )
            reply = d.ctl(
                "create-workload",
                "--name", "vllm-bench",
                "--profile", "mi300x-func",
                "--image", VLLM_IMAGE,
                "--startup", "python,-c,import vllm",
                "--exec", "python,-c,from vllm import LLM",
                "--cleanup", "always",
            )
            self.assertTrue(reply["CreateWorkload"]["ok"])

    def test_show_workload_returns_full_definition(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "p",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            d.ctl(
                "create-workload",
                "--name", "w1",
                "--profile", "p",
                "--image", "img:latest",
                "--startup", "echo,ready",
                "--exec", "python,-c,print(1)",
                "--exec", "python,-c,print(2)",
                "--cleanup", "on-success",
            )
            reply = d.ctl("show-workload", "--name", "w1")
            wl = reply["ShowWorkload"]["workload"]
            self.assertEqual(wl["name"], "w1")
            self.assertEqual(wl["profile"], "p")
            self.assertEqual(wl["image"], "img:latest")
            self.assertEqual(wl["startup"]["command"], "echo")
            self.assertEqual(wl["startup"]["args"], ["ready"])
            self.assertEqual(len(wl["execs"]), 2)
            self.assertEqual(wl["execs"][0]["command"], "python")
            self.assertEqual(wl["execs"][1]["args"], ["-c", "print(2)"])
            self.assertEqual(wl["cleanup"], "on_success")

    def test_list_workloads_summary(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "p",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            d.ctl(
                "create-workload",
                "--name", "bench-summary",
                "--profile", "p",
                "--image", VLLM_IMAGE,
                "--startup", "echo,hi",
                "--exec", "step1",
                "--exec", "step2",
                "--exec", "step3",
                "--cleanup", "never",
            )
            reply = d.ctl("list-workloads")
            summaries = reply["ListWorkloads"]["workloads"]
            self.assertEqual(len(summaries), 1)
            s = summaries[0]
            self.assertEqual(s["name"], "bench-summary")
            self.assertEqual(s["profile"], "p")
            self.assertEqual(s["image"], VLLM_IMAGE)
            self.assertTrue(s["has_startup"])
            self.assertEqual(s["exec_count"], 3)
            self.assertEqual(s["cleanup"], "never")

    def test_delete_workload(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "p",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            d.ctl(
                "create-workload",
                "--name", "del-me",
                "--profile", "p",
                "--image", "img",
                "--startup", "echo,hi",
                "--exec", "echo,bye",
                "--cleanup", "always",
            )
            reply = d.ctl("delete-workload", "--name", "del-me")
            self.assertTrue(reply["DeleteWorkload"]["ok"])

            reply = d.ctl("list-workloads")
            self.assertEqual(len(reply["ListWorkloads"]["workloads"]), 0)

    def test_delete_nonexistent_workload_fails(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("delete-workload", "--name", "ghost")
            self.assertFalse(reply["DeleteWorkload"]["ok"])

    def test_show_nonexistent_workload_returns_empty(self):
        with MirageTestDaemon() as d:
            reply = d.ctl("show-workload", "--name", "nope")
            self.assertNotIn("workload", reply.get("ShowWorkload", {}))


# ============================================================================
# 5. Boot session with real daemon (container runtime errors expected)
# ============================================================================


class TestBootSession(unittest.TestCase):
    def _create_profile(self, d):
        d.ctl(
            "create-profile",
            "--name", "mi300x-func",
            "--simulator", "rocjitsu",
            "--gpu", "MI300X",
            "--mode", "functional",
            "--gpus-per-node", "8",
            "--nodes", "1",
        )

    def test_boot_with_invalid_image_returns_error(self):
        with MirageTestDaemon() as d:
            self._create_profile(d)
            reply = d.ctl(
                "boot",
                "--name", "bad-img",
                "--profile", "mi300x-func",
                "--image", "nonexistent-image:99.99",
            )
            boot = reply["Boot"]
            self.assertFalse(boot["ok"])
            self.assertIn("failed to start container", boot["error"])

    def test_boot_with_nonexistent_profile_fails(self):
        with MirageTestDaemon() as d:
            reply = d.ctl(
                "boot",
                "--name", "s1",
                "--profile", "no-such-profile",
                "--image", "img:latest",
            )
            self.assertFalse(reply["Boot"]["ok"])

    def test_boot_returns_container_ids_field(self):
        with MirageTestDaemon() as d:
            self._create_profile(d)
            reply = d.ctl(
                "boot",
                "--name", "test-boot",
                "--profile", "mi300x-func",
                "--image", "nonexistent:1",
            )
            self.assertIn("container_ids", reply["Boot"])

    def test_boot_sets_up_mirage_env_in_docker_command(self):
        with MirageTestDaemon() as d:
            self._create_profile(d)
            reply = d.ctl(
                "boot",
                "--name", "env-check",
                "--profile", "mi300x-func",
                "--image", "nonexistent:1",
            )
            error = reply["Boot"].get("error", "")
            self.assertIn("MIRAGE_SESSION=env-check", error)
            self.assertIn("LD_PRELOAD=/opt/mirage/libmirage_interceptor.so", error)
            self.assertIn("MIRAGE_INTERCEPTOR_SOCKET=/opt/mirage/emulator.sock", error)

    def test_boot_uses_sleep_infinity_entrypoint(self):
        with MirageTestDaemon() as d:
            self._create_profile(d)
            reply = d.ctl(
                "boot",
                "--name", "entry-check",
                "--profile", "mi300x-func",
                "--image", "nonexistent:1",
            )
            error = reply["Boot"].get("error", "")
            self.assertIn("--entrypoint sleep", error)
            self.assertIn("infinity", error)

    def test_boot_mounts_interceptor_library(self):
        with MirageTestDaemon() as d:
            self._create_profile(d)
            reply = d.ctl(
                "boot",
                "--name", "mount-check",
                "--profile", "mi300x-func",
                "--image", "nonexistent:1",
            )
            error = reply["Boot"].get("error", "")
            self.assertIn("libmirage_interceptor.so:/opt/mirage/libmirage_interceptor.so:ro", error)

    def test_boot_mounts_emulator_socket(self):
        with MirageTestDaemon() as d:
            self._create_profile(d)
            reply = d.ctl(
                "boot",
                "--name", "sock-check",
                "--profile", "mi300x-func",
                "--image", "nonexistent:1",
            )
            error = reply["Boot"].get("error", "")
            self.assertIn("emulator.sock", error)

    def test_boot_mounts_kfd_topology(self):
        with MirageTestDaemon() as d:
            self._create_profile(d)
            reply = d.ctl(
                "boot",
                "--name", "topo-check",
                "--profile", "mi300x-func",
                "--image", "nonexistent:1",
            )
            error = reply["Boot"].get("error", "")
            self.assertIn("/sys/class/kfd", error)
            self.assertIn("/dev/kfd", error)
            self.assertIn("/dev/dri", error)


# ============================================================================
# 6. Session management (create / list / delete without boot)
# ============================================================================


class TestSessionManagement(unittest.TestCase):
    def test_create_session_without_boot(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "p",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            reply = d.ctl(
                "create-session",
                "--name", "unbooted",
                "--profile", "p",
                "--image", "img:latest",
            )
            self.assertTrue(reply["CreateSession"]["ok"])

    def test_list_sessions_shows_created_session(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "p",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            d.ctl(
                "create-session",
                "--name", "listed",
                "--profile", "p",
                "--image", "img:latest",
            )
            reply = d.ctl("list-sessions", "--profile", "p")
            sessions = reply["ListSessions"]["sessions"]
            self.assertEqual(len(sessions), 1)
            self.assertEqual(sessions[0]["name"], "listed")

    def test_delete_unbooted_session(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "p",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            d.ctl(
                "create-session",
                "--name", "del-me",
                "--profile", "p",
                "--image", "img:latest",
            )
            reply = d.ctl("delete-session", "--name", "del-me")
            self.assertTrue(reply["DeleteSession"]["ok"])

    def test_duplicate_session_name_fails(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "p",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            d.ctl("create-session", "--name", "dup", "--profile", "p", "--image", "img")
            reply = d.ctl("create-session", "--name", "dup", "--profile", "p", "--image", "img")
            self.assertFalse(reply["CreateSession"]["ok"])
            self.assertIn("already exists", reply["CreateSession"]["error"])


# ============================================================================
# 7. Full vLLM benchmark lifecycle (profile → workload → overview)
# ============================================================================


class TestVllmBenchmarkLifecycle(unittest.TestCase):
    def test_full_vllm_benchmark_setup_lifecycle(self):
        """Create profile → create workload → verify → delete workload → delete profile."""
        with MirageTestDaemon() as d:
            overview = d.ctl("get-overview")["GetOverview"]
            self.assertEqual(overview["profile_count"], 0)

            d.ctl(
                "create-profile",
                "--name", "mi300x-vllm",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "8",
                "--nodes", "1",
            )

            d.ctl(
                "create-workload",
                "--name", "vllm-qwen-bench",
                "--profile", "mi300x-vllm",
                "--image", VLLM_IMAGE,
                "--startup", "python,-c,import vllm; print(vllm.__version__)",
                "--exec", "python,-c,from vllm import LLM; print('loaded')",
                "--exec", "python,-c,import torch; print(torch.cuda.is_available())",
                "--cleanup", "on-success",
            )

            wl = d.ctl("show-workload", "--name", "vllm-qwen-bench")["ShowWorkload"]["workload"]
            self.assertEqual(wl["name"], "vllm-qwen-bench")
            self.assertEqual(wl["profile"], "mi300x-vllm")
            self.assertEqual(wl["image"], VLLM_IMAGE)
            self.assertEqual(len(wl["execs"]), 2)
            self.assertEqual(wl["cleanup"], "on_success")

            overview = d.ctl("get-overview")["GetOverview"]
            self.assertEqual(overview["profile_count"], 1)

            d.ctl("delete-workload", "--name", "vllm-qwen-bench")
            d.ctl("delete-profile", "--name", "mi300x-vllm")

            overview = d.ctl("get-overview")["GetOverview"]
            self.assertEqual(overview["profile_count"], 0)

    def test_multi_gpu_benchmark_profiles(self):
        """Set up benchmark profiles for MI300X, MI325X, MI350X simultaneously."""
        with MirageTestDaemon() as d:
            for name, gpu in [("bench-mi300x", "MI300X"), ("bench-mi325x", "MI325X"), ("bench-mi350x", "MI350X")]:
                reply = d.ctl(
                    "create-profile",
                    "--name", name,
                    "--simulator", "rocjitsu",
                    "--gpu", gpu,
                    "--mode", "functional",
                    "--gpus-per-node", "8",
                    "--nodes", "1",
                )
                self.assertTrue(reply["CreateProfile"]["ok"])

            profiles = d.ctl("list-profiles", "--simulator", "rocjitsu")["ListProfiles"]["profiles"]
            self.assertEqual(len(profiles), 3)
            profile_names = {p["name"] for p in profiles}
            self.assertEqual(profile_names, {"bench-mi300x", "bench-mi325x", "bench-mi350x"})

    def test_benchmark_workload_with_multiple_exec_steps(self):
        """Create a realistic multi-step vLLM benchmark workload and verify all steps."""
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "bench-profile",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "8",
                "--nodes", "1",
            )
            d.ctl(
                "create-workload",
                "--name", "vllm-full-bench",
                "--profile", "bench-profile",
                "--image", VLLM_IMAGE,
                "--startup", "python,-c,import sys; print(sys.version)",
                "--exec", "python,-c,import torch; print(torch.__version__)",
                "--exec", "python,-c,import vllm; print(vllm.__version__)",
                "--exec", "python,-c,from vllm import LLM; llm = LLM('Qwen/Qwen2.5-0.5B')",
                "--exec", "python,-c,print('benchmark complete')",
                "--cleanup", "always",
            )

            wl = d.ctl("show-workload", "--name", "vllm-full-bench")["ShowWorkload"]["workload"]
            self.assertEqual(len(wl["execs"]), 4)

            summary = d.ctl("list-workloads")["ListWorkloads"]["workloads"][0]
            self.assertEqual(summary["exec_count"], 4)
            self.assertTrue(summary["has_startup"])


# ============================================================================
# 8. Boot with real vLLM image (Docker required, auto-skipped if missing)
# ============================================================================


class TestVllmBootWithDocker(unittest.TestCase):
    """Tests that actually boot a vLLM container via Docker.

    Automatically pulls the vLLM image if it is not available locally.
    Skipped only if Docker itself is not available.
    """

    @classmethod
    def _docker_available(cls):
        try:
            result = subprocess.run(
                ["docker", "info"],
                capture_output=True,
                timeout=10,
            )
            return result.returncode == 0
        except (FileNotFoundError, subprocess.TimeoutExpired):
            return False

    @classmethod
    def _ensure_image(cls):
        """Pull the vLLM image if not already present."""
        inspect = subprocess.run(
            ["docker", "image", "inspect", VLLM_IMAGE],
            capture_output=True,
        )
        if inspect.returncode == 0:
            return
        print(f"\nPulling {VLLM_IMAGE} (this may take a while)...")
        pull = subprocess.run(
            ["docker", "pull", VLLM_IMAGE],
            capture_output=True,
            timeout=1800,
        )
        if pull.returncode != 0:
            raise unittest.SkipTest(
                f"Failed to pull {VLLM_IMAGE}: {pull.stderr.decode(errors='replace').strip()}"
            )

    @classmethod
    def setUpClass(cls):
        if not cls._docker_available():
            raise unittest.SkipTest("Docker is not available")
        cls._ensure_image()

    def test_boot_vllm_session_succeeds(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "mi300x-func",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "8",
                "--nodes", "1",
            )
            reply = d.ctl(
                "boot",
                "--name", "vllm-docker-test",
                "--profile", "mi300x-func",
                "--image", VLLM_IMAGE,
            )
            boot = reply["Boot"]
            try:
                self.assertTrue(boot["ok"], f"boot failed: {boot.get('error')}")
                self.assertGreater(len(boot["container_ids"]), 0)

                sessions = d.ctl("list-sessions", "--profile", "mi300x-func")
                self.assertEqual(len(sessions["ListSessions"]["sessions"]), 1)

                status = d.ctl("status", "--name", "vllm-docker-test")
                self.assertEqual(status["Status"]["name"], "vllm-docker-test")
                self.assertEqual(status["Status"]["simulator"], "rocjitsu")

                exec_reply = d.ctl(
                    "exec",
                    "--session-name", "vllm-docker-test",
                    "--", "python", "-c", "import sys; print(f'Python {sys.version}')",
                )
                self.assertEqual(exec_reply["Exec"]["exit_code"], 0)

                exec_reply = d.ctl(
                    "exec",
                    "--session-name", "vllm-docker-test",
                    "--", "python", "-c", "import torch; print(f'PyTorch {torch.__version__}')",
                )
                self.assertEqual(exec_reply["Exec"]["exit_code"], 0)

                exec_reply = d.ctl(
                    "exec",
                    "--session-name", "vllm-docker-test",
                    "--", "python", "-c", "import vllm; print(f'vLLM {vllm.__version__}')",
                )
                self.assertEqual(exec_reply["Exec"]["exit_code"], 0)

            finally:
                d.ctl("shutdown", "--name", "vllm-docker-test")


# ============================================================================
# 9. Benchmark helper logic (pure Python, no mocks — uses real MirageCtl class)
# ============================================================================


class TestBenchmarkHelpers(unittest.TestCase):
    def test_extract_first_json_object_with_noise(self):
        text = "Loading model...\n{\"a\":1,\"b\":2}\nDone.\n"
        obj = vllm_benchmark.extract_first_json_object(text)
        self.assertEqual(obj, {"a": 1, "b": 2})

    def test_extract_first_json_object_no_json_raises(self):
        with self.assertRaises(ValueError):
            vllm_benchmark.extract_first_json_object("no json here at all")

    def test_extract_first_json_object_nested(self):
        text = '{"outer": {"inner": 42}}'
        obj = vllm_benchmark.extract_first_json_object(text)
        self.assertEqual(obj["outer"]["inner"], 42)

    def test_percentile_single(self):
        self.assertEqual(vllm_benchmark.percentile([5.0], 0.5), 5.0)
        self.assertEqual(vllm_benchmark.percentile([5.0], 0.95), 5.0)

    def test_percentile_two_values(self):
        self.assertAlmostEqual(vllm_benchmark.percentile([10.0, 20.0], 0.5), 15.0)

    def test_percentile_three_values(self):
        vals = [10.0, 30.0, 50.0]
        self.assertEqual(vllm_benchmark.percentile(vals, 0.5), 30.0)
        self.assertAlmostEqual(vllm_benchmark.percentile(vals, 0.95), 48.0)

    def test_percentile_empty_raises(self):
        with self.assertRaises(ValueError):
            vllm_benchmark.percentile([], 0.5)

    def test_summarize_metrics(self):
        rows = [
            vllm_benchmark.IterationMetrics(1, "m", 4, 400, 10.0, 40.0, 2.0),
            vllm_benchmark.IterationMetrics(2, "m", 4, 450, 9.0, 50.0, 2.1),
            vllm_benchmark.IterationMetrics(3, "m", 4, 500, 8.0, 62.5, 1.9),
        ]
        s = vllm_benchmark.summarize_metrics(rows)
        self.assertEqual(s.iterations, 3)
        self.assertEqual(s.prompts_per_iteration, 4)
        self.assertEqual(s.total_output_tokens, 1350)
        self.assertAlmostEqual(s.throughput_mean_tok_s, (40.0 + 50.0 + 62.5) / 3)
        self.assertEqual(s.throughput_p50_tok_s, 50.0)
        self.assertAlmostEqual(s.total_generation_time_s, 27.0)

    def test_build_benchmark_script_embeds_params(self):
        script = vllm_benchmark.build_benchmark_python_script(
            model="Qwen/Qwen2.5-0.5B",
            prompts=["hello", "world"],
            max_tokens=64,
            temperature=0.5,
            gpu_memory_utilization=0.9,
        )
        self.assertIn("Qwen/Qwen2.5-0.5B", script)
        self.assertIn("max_tokens=64", script)
        self.assertIn("temperature=0.5", script)
        self.assertIn("gpu_memory_utilization=0.9", script)
        self.assertIn('"hello"', script)
        self.assertIn('"world"', script)
        self.assertIn("from vllm import LLM", script)
        self.assertIn("json.dumps(result)", script)

    def test_read_prompts_defaults(self):
        args = argparse.Namespace(prompt=[], prompt_file=None)
        prompts = vllm_benchmark.read_prompts(args)
        self.assertEqual(prompts, list(vllm_benchmark.DEFAULT_PROMPTS))

    def test_read_prompts_from_file(self):
        with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as f:
            f.write("prompt one\n\nprompt two\n")
            f.flush()
            args = argparse.Namespace(prompt=["cli prompt"], prompt_file=f.name)
            prompts = vllm_benchmark.read_prompts(args)
            self.assertEqual(prompts, ["cli prompt", "prompt one", "prompt two"])
            os.unlink(f.name)

    def test_run_iterations_with_real_miragectl_class(self):
        """Use real MirageCtl class with a custom run_fn to test run_iterations."""
        iteration_json = json.dumps({
            "model": "Qwen/Qwen2.5-0.5B",
            "prompts": 4,
            "total_output_tokens": 512,
            "generation_time_s": 12.34,
            "throughput_tok_s": 41.5,
            "model_load_time_s": 3.2,
        })
        call_count = [0]

        def real_run_fn(cmd, **kwargs):
            call_count[0] += 1
            return subprocess.CompletedProcess(cmd, 0, stdout=iteration_json, stderr="")

        ctl = vllm_benchmark.MirageCtl(
            socket_path="/tmp/unused.sock",
            ctl_prefix=["echo"],
            run_fn=real_run_fn,
        )
        rows = vllm_benchmark.run_iterations(ctl, "test-session", "print(1)", 3)
        self.assertEqual(len(rows), 3)
        self.assertEqual(call_count[0], 3)
        self.assertEqual(rows[0].model, "Qwen/Qwen2.5-0.5B")
        self.assertEqual(rows[2].total_output_tokens, 512)
        self.assertAlmostEqual(rows[1].throughput_tok_s, 41.5)

    def test_main_rejects_zero_iterations(self):
        rc = vllm_benchmark.main(["--iterations", "0"])
        self.assertEqual(rc, 2)

    def test_parse_args_defaults(self):
        args = vllm_benchmark.parse_args([])
        self.assertEqual(args.iterations, 3)
        self.assertEqual(args.model, "Qwen/Qwen2.5-0.5B")
        self.assertEqual(args.gpu, "MI300X")
        self.assertEqual(args.gpus_per_node, 8)
        self.assertEqual(args.nodes, 1)
        self.assertEqual(args.max_tokens, 128)
        self.assertAlmostEqual(args.temperature, 0.7)
        self.assertIsNone(args.min_throughput_tok_s)


# ============================================================================
# 10. Concurrent profile + workload operations
# ============================================================================


class TestConcurrentOperations(unittest.TestCase):
    def test_create_many_profiles_and_workloads(self):
        with MirageTestDaemon() as d:
            for i in range(5):
                d.ctl(
                    "create-profile",
                    "--name", f"prof-{i}",
                    "--simulator", "rocjitsu",
                    "--gpu", "MI300X",
                    "--mode", "functional",
                    "--gpus-per-node", str(i + 1),
                    "--nodes", "1",
                )
                d.ctl(
                    "create-workload",
                    "--name", f"wl-{i}",
                    "--profile", f"prof-{i}",
                    "--image", VLLM_IMAGE,
                    "--startup", f"echo,start-{i}",
                    "--exec", f"echo,step-{i}",
                    "--cleanup", "always",
                )

            profiles = d.ctl("list-profiles", "--simulator", "rocjitsu")["ListProfiles"]["profiles"]
            self.assertEqual(len(profiles), 5)

            workloads = d.ctl("list-workloads")["ListWorkloads"]["workloads"]
            self.assertEqual(len(workloads), 5)

            overview = d.ctl("get-overview")["GetOverview"]
            self.assertEqual(overview["profile_count"], 5)

    def test_create_delete_create_same_name(self):
        with MirageTestDaemon() as d:
            d.ctl(
                "create-profile",
                "--name", "reuse",
                "--simulator", "rocjitsu",
                "--gpu", "MI300X",
                "--mode", "functional",
                "--gpus-per-node", "1",
                "--nodes", "1",
            )
            d.ctl("delete-profile", "--name", "reuse")
            reply = d.ctl(
                "create-profile",
                "--name", "reuse",
                "--simulator", "rocjitsu",
                "--gpu", "MI325X",
                "--mode", "functional",
                "--gpus-per-node", "4",
                "--nodes", "1",
            )
            self.assertTrue(reply["CreateProfile"]["ok"])

            profiles = d.ctl("list-profiles", "--simulator", "rocjitsu")["ListProfiles"]["profiles"]
            self.assertEqual(len(profiles), 1)
            self.assertEqual(profiles[0]["gpu"], "MI325X")
            self.assertEqual(profiles[0]["num_gpus"], 4)


if __name__ == "__main__":
    unittest.main()