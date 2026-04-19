#!/usr/bin/env python3
"""PyTorch MNIST training script for Mirage + Rocjitsu E2E testing.

Trains a small CNN on MNIST, validates accuracy, and emits structured JSON
results so the calling harness can assert on training outcomes.

Usage:
    python mnist_train.py [--epochs N] [--batch-size N] [--lr LR] [--no-cuda]
"""

import argparse
import json
import os
import sys
import time

import torch
import torch.nn as nn
import torch.nn.functional as F
import torch.optim as optim
from torchvision import datasets, transforms


# ---------------------------------------------------------------------------
#  Model
# ---------------------------------------------------------------------------

class MnistCNN(nn.Module):
    """Small CNN for MNIST classification."""

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
        x = self.fc2(x)
        return x


# ---------------------------------------------------------------------------
#  Training and evaluation
# ---------------------------------------------------------------------------

def train_epoch(model, device, loader, optimizer, epoch):
    """Train for one epoch, return average loss."""
    model.train()
    total_loss = 0.0
    correct = 0
    total = 0
    for batch_idx, (data, target) in enumerate(loader):
        data, target = data.to(device), target.to(device)
        optimizer.zero_grad()
        output = model(data)
        loss = F.cross_entropy(output, target)
        loss.backward()
        optimizer.step()
        total_loss += loss.item() * data.size(0)
        pred = output.argmax(dim=1)
        correct += pred.eq(target).sum().item()
        total += data.size(0)

    avg_loss = total_loss / total
    accuracy = 100.0 * correct / total
    print(f"  Epoch {epoch}: loss={avg_loss:.4f}  train_acc={accuracy:.1f}%")
    return avg_loss, accuracy


def evaluate(model, device, loader):
    """Evaluate on a dataset, return loss and accuracy."""
    model.eval()
    total_loss = 0.0
    correct = 0
    total = 0
    with torch.no_grad():
        for data, target in loader:
            data, target = data.to(device), target.to(device)
            output = model(data)
            total_loss += F.cross_entropy(output, target, reduction="sum").item()
            pred = output.argmax(dim=1)
            correct += pred.eq(target).sum().item()
            total += data.size(0)

    avg_loss = total_loss / total
    accuracy = 100.0 * correct / total
    return avg_loss, accuracy


# ---------------------------------------------------------------------------
#  Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="MNIST training for Mirage E2E test")
    parser.add_argument("--epochs", type=int, default=2, help="training epochs")
    parser.add_argument("--batch-size", type=int, default=256, help="batch size")
    parser.add_argument("--lr", type=float, default=0.01, help="learning rate")
    parser.add_argument("--no-cuda", action="store_true", help="disable CUDA")
    parser.add_argument("--data-dir", default="/tmp/mnist-data", help="dataset directory")
    args = parser.parse_args()

    use_cuda = not args.no_cuda and torch.cuda.is_available()
    device = torch.device("cuda" if use_cuda else "cpu")

    print(f"Device: {device}")
    if use_cuda:
        print(f"GPU: {torch.cuda.get_device_name(0)}")
        print(f"HIP version: {torch.version.hip}")

    transform = transforms.Compose([
        transforms.ToTensor(),
        transforms.Normalize((0.1307,), (0.3081,)),
    ])

    print("Loading MNIST dataset...")
    train_dataset = datasets.MNIST(
        args.data_dir, train=True, download=True, transform=transform,
    )
    test_dataset = datasets.MNIST(
        args.data_dir, train=False, download=True, transform=transform,
    )

    train_loader = torch.utils.data.DataLoader(
        train_dataset, batch_size=args.batch_size, shuffle=True, num_workers=0,
    )
    test_loader = torch.utils.data.DataLoader(
        test_dataset, batch_size=args.batch_size, shuffle=False, num_workers=0,
    )

    model = MnistCNN().to(device)
    optimizer = optim.Adam(model.parameters(), lr=args.lr)

    print(f"Training for {args.epochs} epoch(s)...")
    t0 = time.time()
    epoch_results = []
    for epoch in range(1, args.epochs + 1):
        loss, acc = train_epoch(model, device, train_loader, optimizer, epoch)
        epoch_results.append({"epoch": epoch, "train_loss": round(loss, 4), "train_acc": round(acc, 1)})

    training_time = time.time() - t0

    print("Evaluating on test set...")
    test_loss, test_accuracy = evaluate(model, device, test_loader)
    print(f"  Test: loss={test_loss:.4f}  accuracy={test_accuracy:.1f}%")

    result = {
        "status": "success",
        "device": str(device),
        "cuda_available": torch.cuda.is_available(),
        "gpu_name": torch.cuda.get_device_name(0) if use_cuda else None,
        "hip_version": torch.version.hip if hasattr(torch.version, "hip") else None,
        "epochs": args.epochs,
        "batch_size": args.batch_size,
        "learning_rate": args.lr,
        "training_time_s": round(training_time, 2),
        "epoch_results": epoch_results,
        "test_loss": round(test_loss, 4),
        "test_accuracy": round(test_accuracy, 1),
        "model_parameters": sum(p.numel() for p in model.parameters()),
        "passed": test_accuracy > 85.0,
    }

    print("\n--- RESULT ---")
    print(json.dumps(result, indent=2))

    if not result["passed"]:
        print(f"\nFAILED: test accuracy {test_accuracy:.1f}% < 85.0% threshold", file=sys.stderr)
        sys.exit(1)

    print("\nMNIST training test passed.")


if __name__ == "__main__":
    main()
