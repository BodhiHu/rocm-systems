#!/usr/bin/env bash
# demo/mnist.sh — End-to-end Mirage demo: boot a PyTorch container on a
# simulated MI300X GPU and run MNIST training.
#
# Usage:
#   ./emulation/demo/mnist.sh
#
# Requires: cargo, docker

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_DIR"

# Use a unique socket so we don't collide with any running daemon.
SOCKET="$(mktemp -u /tmp/mirage-mnist-XXXXXX.sock)"
SESSION_NAME="mnist-demo"
PROFILE_NAME="mi300x-mnist"
IMAGE="docker.io/rocm/pytorch:rocm6.4_ubuntu24.04_py3.12_pytorch_release_2.6.0"

cleanup() {
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

step "Creating profile: $PROFILE_NAME (MI300X, functional, 1×1)"
ctl profile create \
    --name "$PROFILE_NAME" \
    --simulator rocjitsu \
    --gpu MI300X \
    --mode functional \
    --gpus-per-node 1 \
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

# -- pre-flight checks -----------------------------------------------------

step "Exec: Python version"
ctl exec --name "$SESSION_NAME" -- python -c "import sys; print(f'Python {sys.version}')"

step "Exec: PyTorch version and GPU info"
ctl exec --name "$SESSION_NAME" -- \
    python -c "
import torch, json
report = {
    'pytorch_version': torch.__version__,
    'cuda_available': torch.cuda.is_available(),
    'hip_version': torch.version.hip,
    'gpu_count': torch.cuda.device_count(),
    'gpu_name': torch.cuda.get_device_name(0) if torch.cuda.is_available() else 'N/A',
}
print(json.dumps(report, indent=2))
"

step "Exec: Verify torchvision and MNIST deps"
ctl exec --name "$SESSION_NAME" -- \
    python -c "
import torchvision
print(f'torchvision {torchvision.__version__}')
from torchvision import datasets, transforms
print('MNIST dataset and transforms: OK')
"

# -- MNIST training ---------------------------------------------------------

step "Exec: MNIST training (2 epochs)"
ctl exec --name "$SESSION_NAME" -- \
    python -c "
import argparse, json, os, sys, time
import torch
import torch.nn as nn
import torch.nn.functional as F
import torch.optim as optim
from torchvision import datasets, transforms

class MnistCNN(nn.Module):
    def __init__(self):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 16, 3, padding=1)
        self.conv2 = nn.Conv2d(16, 32, 3, padding=1)
        self.pool = nn.MaxPool2d(2, 2)
        self.fc1 = nn.Linear(32 * 7 * 7, 128)
        self.fc2 = nn.Linear(128, 10)

    def forward(self, x):
        x = self.pool(F.relu(self.conv1(x)))
        x = self.pool(F.relu(self.conv2(x)))
        x = x.view(x.size(0), -1)
        x = F.relu(self.fc1(x))
        return self.fc2(x)

use_cuda = torch.cuda.is_available()
device = torch.device('cuda' if use_cuda else 'cpu')
print(f'Device: {device}')
if use_cuda:
    print(f'GPU: {torch.cuda.get_device_name(0)}')

transform = transforms.Compose([
    transforms.ToTensor(),
    transforms.Normalize((0.1307,), (0.3081,)),
])

data_dir = '/tmp/mnist-data'
train_ds = datasets.MNIST(data_dir, train=True, download=True, transform=transform)
test_ds = datasets.MNIST(data_dir, train=False, download=True, transform=transform)
train_loader = torch.utils.data.DataLoader(train_ds, batch_size=256, shuffle=True, num_workers=0)
test_loader = torch.utils.data.DataLoader(test_ds, batch_size=256, shuffle=False, num_workers=0)

model = MnistCNN().to(device)
optimizer = optim.Adam(model.parameters(), lr=0.01)
epochs = 2

t0 = time.time()
for epoch in range(1, epochs + 1):
    model.train()
    total_loss, correct, total = 0.0, 0, 0
    for data, target in train_loader:
        data, target = data.to(device), target.to(device)
        optimizer.zero_grad()
        output = model(data)
        loss = F.cross_entropy(output, target)
        loss.backward()
        optimizer.step()
        total_loss += loss.item() * data.size(0)
        correct += output.argmax(dim=1).eq(target).sum().item()
        total += data.size(0)
    print(f'  Epoch {epoch}: loss={total_loss/total:.4f} acc={100.0*correct/total:.1f}%')
training_time = time.time() - t0

model.eval()
test_loss, correct, total = 0.0, 0, 0
with torch.no_grad():
    for data, target in test_loader:
        data, target = data.to(device), target.to(device)
        output = model(data)
        test_loss += F.cross_entropy(output, target, reduction='sum').item()
        correct += output.argmax(dim=1).eq(target).sum().item()
        total += data.size(0)

test_accuracy = 100.0 * correct / total
result = {
    'status': 'success',
    'device': str(device),
    'cuda_available': use_cuda,
    'epochs': epochs,
    'training_time_s': round(training_time, 2),
    'test_loss': round(test_loss / total, 4),
    'test_accuracy': round(test_accuracy, 1),
    'model_parameters': sum(p.numel() for p in model.parameters()),
    'passed': test_accuracy > 85.0,
}
print(json.dumps(result, indent=2))

if not result['passed']:
    print(f'FAILED: test accuracy {test_accuracy:.1f}% < 85.0%', file=sys.stderr)
    sys.exit(1)
print('MNIST training test passed.')
"

# -- shutdown ---------------------------------------------------------------

step "Shutting down session: $SESSION_NAME"
ctl shutdown --name "$SESSION_NAME"

step "Confirming session is gone"
ctl session list

echo
echo "✅  MNIST E2E demo complete."
