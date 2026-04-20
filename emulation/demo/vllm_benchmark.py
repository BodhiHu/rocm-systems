#!/usr/bin/env python3
"""Run a configurable vLLM benchmark through Mirage + rocjitsu.

This script mirrors the end-to-end flow from demo/vllm.sh while adding:
- configurable prompts/model/iterations,
- structured JSON output,
- aggregate metric computation,
- throughput gating for CI.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass
from json import JSONDecoder
from pathlib import Path
from typing import Callable


DEFAULT_IMAGE = (
    "docker.io/rocm/vllm:"
    "rocm7.12.0_gfx94X-dcgpu_ubuntu24.04_py3.12_pytorch_2.9.1_vllm_0.16.0"
)

DEFAULT_PROMPTS = [
    "What is the capital of France?",
    "Explain quantum computing in simple terms.",
    "Write a haiku about the ocean.",
    "What are the benefits of exercise?",
]


@dataclass(frozen=True)
class IterationMetrics:
    iteration: int
    model: str
    prompts: int
    total_output_tokens: int
    generation_time_s: float
    throughput_tok_s: float
    model_load_time_s: float


@dataclass(frozen=True)
class SummaryMetrics:
    iterations: int
    prompts_per_iteration: int
    total_output_tokens: int
    total_generation_time_s: float
    throughput_mean_tok_s: float
    throughput_p50_tok_s: float
    throughput_p95_tok_s: float
    generation_time_mean_s: float
    generation_time_p50_s: float
    generation_time_p95_s: float


class CommandError(RuntimeError):
    """Raised when a subprocess command exits non-zero."""


class MirageCtl:
    def __init__(
        self,
        socket_path: str,
        ctl_prefix: list[str] | None = None,
        run_fn: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
    ) -> None:
        self.socket_path = socket_path
        self.ctl_prefix = ctl_prefix or ["cargo", "run", "--quiet", "--bin", "mirage_ctl", "--"]
        self.run_fn = run_fn

    def run(self, args: list[str]) -> str:
        cmd = [*self.ctl_prefix, "--socket", self.socket_path, *args]
        completed = self.run_fn(cmd, capture_output=True, text=True, check=False)
        if completed.returncode != 0:
            raise CommandError(
                f"command failed ({' '.join(cmd)}): "
                f"stdout={completed.stdout.strip()} stderr={completed.stderr.strip()}"
            )
        return completed.stdout


def extract_first_json_object(text: str) -> dict:
    decoder = JSONDecoder()
    for idx, ch in enumerate(text):
        if ch != "{":
            continue
        try:
            obj, _ = decoder.raw_decode(text[idx:])
        except json.JSONDecodeError:
            continue
        if isinstance(obj, dict):
            return obj
    raise ValueError("could not find JSON object in command output")


def percentile(values: list[float], p: float) -> float:
    if not values:
        raise ValueError("percentile requires at least one value")
    if len(values) == 1:
        return values[0]
    sorted_values = sorted(values)
    pos = (len(sorted_values) - 1) * p
    lower = int(pos)
    upper = min(lower + 1, len(sorted_values) - 1)
    weight = pos - lower
    return sorted_values[lower] * (1.0 - weight) + sorted_values[upper] * weight


def summarize_metrics(rows: list[IterationMetrics]) -> SummaryMetrics:
    throughputs = [row.throughput_tok_s for row in rows]
    gen_times = [row.generation_time_s for row in rows]
    return SummaryMetrics(
        iterations=len(rows),
        prompts_per_iteration=rows[0].prompts,
        total_output_tokens=sum(row.total_output_tokens for row in rows),
        total_generation_time_s=sum(gen_times),
        throughput_mean_tok_s=statistics.fmean(throughputs),
        throughput_p50_tok_s=percentile(throughputs, 0.50),
        throughput_p95_tok_s=percentile(throughputs, 0.95),
        generation_time_mean_s=statistics.fmean(gen_times),
        generation_time_p50_s=percentile(gen_times, 0.50),
        generation_time_p95_s=percentile(gen_times, 0.95),
    )


def build_benchmark_python_script(
    model: str,
    prompts: list[str],
    max_tokens: int,
    temperature: float,
    gpu_memory_utilization: float,
) -> str:
    prompt_literal = json.dumps(prompts)
    model_literal = json.dumps(model)
    return "\n".join(
        [
            "from vllm import LLM, SamplingParams",
            "import json, time",
            f"model = {model_literal}",
            f"prompts = {prompt_literal}",
            "t0 = time.time()",
            "llm = LLM(",
            "    model=model,",
            "    tensor_parallel_size=1,",
            f"    gpu_memory_utilization={gpu_memory_utilization},",
            ")",
            "load_s = time.time() - t0",
            "params = SamplingParams(",
            f"    temperature={temperature},",
            f"    max_tokens={max_tokens},",
            ")",
            "t0 = time.time()",
            "outputs = llm.generate(prompts, params)",
            "gen_s = time.time() - t0",
            "tok = sum(len(o.outputs[0].token_ids) for o in outputs)",
            "result = {",
            "    'model': model,",
            "    'prompts': len(prompts),",
            "    'total_output_tokens': tok,",
            "    'generation_time_s': gen_s,",
            "    'throughput_tok_s': (tok / gen_s) if gen_s > 0 else 0.0,",
            "    'model_load_time_s': load_s,",
            "}",
            "print(json.dumps(result))",
        ]
    )


def wait_for_unix_socket(path: str, timeout_s: float) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        if os.path.exists(path):
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
                try:
                    sock.connect(path)
                except OSError:
                    time.sleep(0.1)
                    continue
            return
        time.sleep(0.1)
    raise TimeoutError(f"daemon socket did not become ready: {path}")


def read_prompts(args: argparse.Namespace) -> list[str]:
    prompts = list(args.prompt)
    if args.prompt_file:
        for raw in Path(args.prompt_file).read_text(encoding="utf-8").splitlines():
            line = raw.strip()
            if line:
                prompts.append(line)
    if not prompts:
        prompts = list(DEFAULT_PROMPTS)
    return prompts


def run_iterations(
    ctl: MirageCtl,
    session_name: str,
    script: str,
    iterations: int,
) -> list[IterationMetrics]:
    rows: list[IterationMetrics] = []
    for it in range(1, iterations + 1):
        output = ctl.run([
            "exec",
            "--session-name",
            session_name,
            "--",
            "python",
            "-c",
            script,
        ])
        obj = extract_first_json_object(output)
        rows.append(
            IterationMetrics(
                iteration=it,
                model=str(obj["model"]),
                prompts=int(obj["prompts"]),
                total_output_tokens=int(obj["total_output_tokens"]),
                generation_time_s=float(obj["generation_time_s"]),
                throughput_tok_s=float(obj["throughput_tok_s"]),
                model_load_time_s=float(obj.get("model_load_time_s", 0.0)),
            )
        )
    return rows


def run_benchmark(args: argparse.Namespace) -> dict:
    prompts = read_prompts(args)
    socket_path = args.socket or os.path.join(
        tempfile.gettempdir(), f"mirage-vllm-bench-{os.getpid()}.sock"
    )
    daemon_cmd = ["cargo", "run", "--quiet", "--bin", "mirage_daemon", "--", "--socket", socket_path]

    daemon_proc = None
    ctl = MirageCtl(socket_path=socket_path)
    try:
        if not args.skip_build:
            subprocess.run(
                ["cargo", "build", "--quiet", "--bin", "mirage_daemon", "--bin", "mirage_ctl"],
                check=True,
            )

        daemon_proc = subprocess.Popen(
            daemon_cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        wait_for_unix_socket(socket_path, args.daemon_start_timeout_s)

        ctl.run(["profile", "create", "--name", args.profile_name, "--simulator", "rocjitsu", "--gpu", args.gpu, "--mode", "functional", "--gpus-per-node", str(args.gpus_per_node), "--nodes", str(args.nodes)])
        ctl.run(["boot", "--name", args.session_name, "--profile", args.profile_name, "--image", args.image])

        ready_script = (
            "import json, torch, vllm; "
            "print(json.dumps({'status':'ready','vllm_version':vllm.__version__,'pytorch_version':torch.__version__}))"
        )
        ctl.run(["exec", "--session-name", args.session_name, "--", "python", "-c", ready_script])

        bench_script = build_benchmark_python_script(
            model=args.model,
            prompts=prompts,
            max_tokens=args.max_tokens,
            temperature=args.temperature,
            gpu_memory_utilization=args.gpu_memory_utilization,
        )
        rows = run_iterations(ctl, args.session_name, bench_script, args.iterations)
        summary = summarize_metrics(rows)

        if args.min_throughput_tok_s is not None and summary.throughput_mean_tok_s < args.min_throughput_tok_s:
            raise RuntimeError(
                "throughput gate failed: "
                f"mean={summary.throughput_mean_tok_s:.3f} tok/s "
                f"< required={args.min_throughput_tok_s:.3f} tok/s"
            )

        result = {
            "image": args.image,
            "simulator": "rocjitsu",
            "mode": "functional",
            "gpu": args.gpu,
            "gpus_per_node": args.gpus_per_node,
            "nodes": args.nodes,
            "session": args.session_name,
            "profile": args.profile_name,
            "model": args.model,
            "prompts": prompts,
            "iterations": [asdict(row) for row in rows],
            "summary": asdict(summary),
        }
        return result
    finally:
        if ctl is not None:
            try:
                ctl.run(["shutdown", "--name", args.session_name])
            except Exception:
                pass
        if daemon_proc is not None and daemon_proc.poll() is None:
            daemon_proc.send_signal(signal.SIGTERM)
            try:
                daemon_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                daemon_proc.kill()
        if socket_path and os.path.exists(socket_path):
            try:
                os.remove(socket_path)
            except OSError:
                pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Benchmark vLLM through Mirage + rocjitsu")
    parser.add_argument("--socket", default=None, help="Unix socket path for mirage_daemon")
    parser.add_argument("--session-name", default="vllm-benchmark")
    parser.add_argument("--profile-name", default="mi300x-func")
    parser.add_argument("--image", default=DEFAULT_IMAGE)
    parser.add_argument("--gpu", default="MI300X")
    parser.add_argument("--gpus-per-node", type=int, default=8)
    parser.add_argument("--nodes", type=int, default=1)
    parser.add_argument("--model", default="Qwen/Qwen2.5-0.5B")
    parser.add_argument("--prompt", action="append", default=[])
    parser.add_argument("--prompt-file", default=None)
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--max-tokens", type=int, default=128)
    parser.add_argument("--temperature", type=float, default=0.7)
    parser.add_argument("--gpu-memory-utilization", type=float, default=0.8)
    parser.add_argument("--min-throughput-tok-s", type=float, default=None)
    parser.add_argument("--daemon-start-timeout-s", type=float, default=8.0)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--output-json", default=None)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    if args.iterations < 1:
        print("--iterations must be >= 1", file=sys.stderr)
        return 2

    try:
        result = run_benchmark(args)
    except Exception as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1

    out = json.dumps(result, indent=2)
    print(out)
    if args.output_json:
        Path(args.output_json).write_text(out + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())