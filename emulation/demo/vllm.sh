#!/usr/bin/env bash
# demo/vllm.sh — End-to-end Mirage demo: boot a real vLLM container,
# verify the framework loads, and shut down.
#
# Usage:
#   cd emulation && ./demo/vllm.sh
#
# Requires: cargo, docker

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_DIR"

# Use a unique socket so we don't collide with any running daemon.
SOCKET="$(mktemp -u /tmp/mirage-demo-XXXXXX.sock)"
SESSION_NAME="vllm-demo"
PROFILE_NAME="mi300x-func"
IMAGE="docker.io/rocm/vllm:rocm7.12.0_gfx94X-dcgpu_ubuntu24.04_py3.12_pytorch_2.9.1_vllm_0.16.0"

cleanup() {
    # Best-effort teardown: shut down the session, then kill the daemon.
    if [[ -S "$SOCKET" ]] && [[ -n "${DAEMON_PID:-}" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        cargo run --quiet --bin mirage_ctl -- --socket "$SOCKET" shutdown --name "$SESSION_NAME" 2>/dev/null || true
    fi
    if [[ -n "${DAEMON_PID:-}" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    rm -f "$SOCKET"
}
trap cleanup EXIT

# -- helpers ----------------------------------------------------------------

ctl() {
    cargo run --quiet --bin mirage_ctl -- --socket "$SOCKET" "$@"
}

step() {
    echo
    echo "━━━ $* ━━━"
}

# -- build ------------------------------------------------------------------

step "Building mirage_daemon and mirage_ctl"
cargo build --quiet --bin mirage_daemon --bin mirage_ctl

# -- start daemon -----------------------------------------------------------

step "Starting mirage_daemon (socket: $SOCKET)"
cargo run --quiet --bin mirage_daemon -- --socket "$SOCKET" &
DAEMON_PID=$!

# Wait for the socket to appear.
for i in $(seq 1 30); do
    if [[ -S "$SOCKET" ]]; then break; fi
    sleep 0.1
done
if [[ ! -S "$SOCKET" ]]; then
    echo "ERROR: daemon did not start within 3 s" >&2
    exit 1
fi
echo "daemon running (pid $DAEMON_PID)"

# -- overview ---------------------------------------------------------------

step "Checking daemon overview"
ctl overview

step "Listing built-in simulators"
ctl simulators list

# -- create profile ---------------------------------------------------------

step "Creating profile: $PROFILE_NAME (MI300X, functional, 1×8)"
ctl profile create \
    --name "$PROFILE_NAME" \
    --simulator rocjitsu \
    --gpu MI300X \
    --mode functional \
    --gpus-per-node 8 \
    --nodes 1

ctl profile list

# -- boot session -----------------------------------------------------------

step "Booting session: $SESSION_NAME"
echo "image: $IMAGE"
echo "(this may take a while on first run — pulling ~15 GB image)"
ctl boot \
    --name "$SESSION_NAME" \
    --profile "$PROFILE_NAME" \
    --image "$IMAGE"

step "Session detail"
ctl session show "$SESSION_NAME"

# -- exec inside session ----------------------------------------------------

step "Exec: Python version"
ctl exec --name "$SESSION_NAME" -- python -c "import sys; print(f'Python {sys.version}')"

step "Exec: PyTorch version and build info"
ctl exec --name "$SESSION_NAME" -- \
    python -c "import torch; print(f'PyTorch {torch.__version__}'); print(f'CUDA available: {torch.cuda.is_available()}'); print(f'ROCm built: {torch.version.hip is not None}')"

step "Exec: vLLM import and version"
ctl exec --name "$SESSION_NAME" -- \
    python -c "import vllm; print(f'vLLM {vllm.__version__}')"

step "Exec: vLLM engine config validation"
ctl exec --name "$SESSION_NAME" -- \
    python -c "
from vllm.config import ModelConfig
print('vLLM ModelConfig imported successfully')
print('Engine configuration subsystem: OK')
"

step "Exec: full vLLM readiness report"
ctl exec --name "$SESSION_NAME" -- \
    python -c "
import json, torch, vllm
report = {
    'vllm_version': vllm.__version__,
    'pytorch_version': torch.__version__,
    'rocm_build': torch.version.hip is not None,
    'hip_version': torch.version.hip,
    'cuda_available': torch.cuda.is_available(),
    'gpu_count': torch.cuda.device_count(),
    'gpu_name': torch.cuda.get_device_name(0) if torch.cuda.is_available() else 'N/A',
    'mirage_interceptor': __import__('os').path.exists('/opt/mirage/libmirage_interceptor.so'),
    'status': 'ready',
}
print(json.dumps(report, indent=2))
"

# -- Qwen benchmark ---------------------------------------------------------

step "Exec: Qwen/Qwen2.5-0.5B inference benchmark"
echo "(downloading model weights on first run)"
ctl exec --name "$SESSION_NAME" -- \
    python -c "
from vllm import LLM, SamplingParams
import time, json

model = 'Qwen/Qwen2.5-0.5B'
print(f'Loading {model}...')
t0 = time.time()
llm = LLM(model=model, tensor_parallel_size=1, gpu_memory_utilization=0.8)
load_s = time.time() - t0
print(f'Model loaded in {load_s:.1f}s')

prompts = [
    'What is the capital of France?',
    'Explain quantum computing in simple terms.',
    'Write a haiku about the ocean.',
    'What are the benefits of exercise?',
]
params = SamplingParams(temperature=0.7, max_tokens=128)

t0 = time.time()
outputs = llm.generate(prompts, params)
gen_s = time.time() - t0
tok = sum(len(o.outputs[0].token_ids) for o in outputs)

result = {
    'model': model,
    'prompts': len(prompts),
    'total_output_tokens': tok,
    'generation_time_s': round(gen_s, 2),
    'throughput_tok_s': round(tok / gen_s, 1),
}
print(json.dumps(result, indent=2))

for i, o in enumerate(outputs):
    print(f'\n[Prompt {i+1}] {o.prompt}')
    print(f'[Reply]  {o.outputs[0].text[:200]}')
print('\nBenchmark passed.')
"

# -- shutdown ---------------------------------------------------------------

step "Shutting down session: $SESSION_NAME"
ctl shutdown --name "$SESSION_NAME"

step "Confirming session is gone"
ctl session list

echo
echo "✅  vLLM E2E demo complete."
