import argparse
import json
import os
import tempfile
import unittest
from unittest.mock import patch

from emulation.demo import vllm_benchmark


class _FakeCtl:
    def __init__(self, outputs):
        self.outputs = list(outputs)
        self.calls = []

    def run(self, args):
        self.calls.append(list(args))
        if not self.outputs:
            raise RuntimeError("no more fake outputs")
        return self.outputs.pop(0)


class _FakeProc:
    def __init__(self):
        self.signals = []
        self.killed = False

    def poll(self):
        return None

    def send_signal(self, sig):
        self.signals.append(sig)

    def wait(self, timeout=None):
        return 0

    def kill(self):
        self.killed = True


class VllmBenchmarkUnitTests(unittest.TestCase):
    def test_extract_first_json_object_with_prefix_and_suffix(self):
        text = "noise before\n{\"a\":1,\"b\":2}\nnoise after"
        obj = vllm_benchmark.extract_first_json_object(text)
        self.assertEqual(obj["a"], 1)
        self.assertEqual(obj["b"], 2)

    def test_extract_first_json_object_raises_on_missing_json(self):
        with self.assertRaises(ValueError):
            vllm_benchmark.extract_first_json_object("no json here")

    def test_percentile_single_value(self):
        self.assertEqual(vllm_benchmark.percentile([7.5], 0.95), 7.5)

    def test_percentile_interpolates(self):
        values = [10.0, 30.0, 50.0]
        self.assertEqual(vllm_benchmark.percentile(values, 0.50), 30.0)
        self.assertAlmostEqual(vllm_benchmark.percentile(values, 0.95), 48.0)

    def test_summarize_metrics(self):
        rows = [
            vllm_benchmark.IterationMetrics(
                iteration=1,
                model="m",
                prompts=4,
                total_output_tokens=400,
                generation_time_s=10.0,
                throughput_tok_s=40.0,
                model_load_time_s=2.0,
            ),
            vllm_benchmark.IterationMetrics(
                iteration=2,
                model="m",
                prompts=4,
                total_output_tokens=450,
                generation_time_s=9.0,
                throughput_tok_s=50.0,
                model_load_time_s=2.1,
            ),
        ]
        summary = vllm_benchmark.summarize_metrics(rows)
        self.assertEqual(summary.iterations, 2)
        self.assertEqual(summary.total_output_tokens, 850)
        self.assertAlmostEqual(summary.throughput_mean_tok_s, 45.0)
        self.assertAlmostEqual(summary.generation_time_mean_s, 9.5)

    def test_build_benchmark_python_script_contains_parameters(self):
        script = vllm_benchmark.build_benchmark_python_script(
            model="Qwen/Qwen2.5-0.5B",
            prompts=["p1", "p2"],
            max_tokens=64,
            temperature=0.5,
            gpu_memory_utilization=0.9,
        )
        self.assertIn("Qwen/Qwen2.5-0.5B", script)
        self.assertIn("max_tokens=64", script)
        self.assertIn("temperature=0.5", script)
        self.assertIn("gpu_memory_utilization=0.9", script)

    def test_read_prompts_from_file_and_flags(self):
        with tempfile.TemporaryDirectory() as td:
            path = os.path.join(td, "prompts.txt")
            with open(path, "w", encoding="utf-8") as f:
                f.write("file prompt 1\n\nfile prompt 2\n")

            args = argparse.Namespace(prompt=["cli prompt"], prompt_file=path)
            prompts = vllm_benchmark.read_prompts(args)
            self.assertEqual(prompts, ["cli prompt", "file prompt 1", "file prompt 2"])

    def test_run_iterations_parses_json_rows(self):
        outputs = [
            '{"model":"m","prompts":2,"total_output_tokens":100,"generation_time_s":2.0,"throughput_tok_s":50.0,"model_load_time_s":1.2}',
            '{"model":"m","prompts":2,"total_output_tokens":120,"generation_time_s":2.4,"throughput_tok_s":50.0,"model_load_time_s":1.0}',
        ]
        fake = _FakeCtl(outputs)
        rows = vllm_benchmark.run_iterations(fake, "bench-session", "print(1)", 2)
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0].iteration, 1)
        self.assertEqual(rows[1].total_output_tokens, 120)
        self.assertEqual(len(fake.calls), 2)

    def test_run_iterations_fails_on_invalid_json(self):
        fake = _FakeCtl(["not-json"])
        with self.assertRaises(ValueError):
            vllm_benchmark.run_iterations(fake, "bench-session", "print(1)", 1)

    def test_main_rejects_zero_iterations(self):
        rc = vllm_benchmark.main(["--iterations", "0"])
        self.assertEqual(rc, 2)

    @patch("emulation.demo.vllm_benchmark.run_benchmark")
    def test_main_writes_json_file(self, mock_run_benchmark):
        mock_run_benchmark.return_value = {
            "summary": {"throughput_mean_tok_s": 10.0},
            "iterations": [],
        }
        with tempfile.TemporaryDirectory() as td:
            out_path = os.path.join(td, "result.json")
            rc = vllm_benchmark.main(["--output-json", out_path])
            self.assertEqual(rc, 0)
            with open(out_path, "r", encoding="utf-8") as f:
                saved = json.load(f)
            self.assertEqual(saved["summary"]["throughput_mean_tok_s"], 10.0)

    @patch("emulation.demo.vllm_benchmark.os.remove")
    @patch("emulation.demo.vllm_benchmark.os.path.exists", return_value=True)
    @patch("emulation.demo.vllm_benchmark.wait_for_unix_socket")
    @patch("emulation.demo.vllm_benchmark.subprocess.Popen")
    @patch("emulation.demo.vllm_benchmark.MirageCtl")
    def test_run_benchmark_success_path(
        self,
        mock_ctl_cls,
        mock_popen,
        _mock_wait,
        _mock_exists,
        _mock_remove,
    ):
        fake_ctl = _FakeCtl(
            [
                "profile ok",  # profile create
                "boot ok",  # boot
                '{"status":"ready"}',  # readiness
                '{"model":"m","prompts":2,"total_output_tokens":100,"generation_time_s":2.0,"throughput_tok_s":50.0,"model_load_time_s":1.0}',
                '{"model":"m","prompts":2,"total_output_tokens":120,"generation_time_s":2.0,"throughput_tok_s":60.0,"model_load_time_s":1.0}',
                "shutdown ok",  # cleanup
            ]
        )
        mock_ctl_cls.return_value = fake_ctl
        mock_popen.return_value = _FakeProc()

        args = vllm_benchmark.parse_args(["--iterations", "2", "--skip-build"])
        result = vllm_benchmark.run_benchmark(args)

        self.assertEqual(result["simulator"], "rocjitsu")
        self.assertEqual(len(result["iterations"]), 2)
        self.assertAlmostEqual(result["summary"]["throughput_mean_tok_s"], 55.0)

    @patch("emulation.demo.vllm_benchmark.os.remove")
    @patch("emulation.demo.vllm_benchmark.os.path.exists", return_value=True)
    @patch("emulation.demo.vllm_benchmark.wait_for_unix_socket")
    @patch("emulation.demo.vllm_benchmark.subprocess.Popen")
    @patch("emulation.demo.vllm_benchmark.MirageCtl")
    def test_run_benchmark_enforces_throughput_gate(
        self,
        mock_ctl_cls,
        mock_popen,
        _mock_wait,
        _mock_exists,
        _mock_remove,
    ):
        fake_ctl = _FakeCtl(
            [
                "profile ok",
                "boot ok",
                '{"status":"ready"}',
                '{"model":"m","prompts":2,"total_output_tokens":10,"generation_time_s":2.0,"throughput_tok_s":5.0,"model_load_time_s":1.0}',
                "shutdown ok",
            ]
        )
        mock_ctl_cls.return_value = fake_ctl
        mock_popen.return_value = _FakeProc()

        args = vllm_benchmark.parse_args(
            [
                "--iterations",
                "1",
                "--skip-build",
                "--min-throughput-tok-s",
                "10",
            ]
        )
        with self.assertRaises(RuntimeError):
            vllm_benchmark.run_benchmark(args)


if __name__ == "__main__":
    unittest.main()