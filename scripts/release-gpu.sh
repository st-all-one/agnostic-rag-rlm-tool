#!/usr/bin/env bash
# release-gpu.sh — build the OPT-IN arags-server GPU release artifact.
#
# Builds a static musl binary of `arags-server` with the `llamacpp-vulkan`
# feature (llama.cpp + Vulkan backend) and tags the resulting Docker image with
# the `-gpu` suffix. This is STRICTLY OPT-IN and never part of the default
# portable (candle) build.
#
# PREREQUISITES (must be set up before running — cannot be validated in a
# candle-only / no-Vulkan-SDK environment):
#   * rustup target x86_64-unknown-linux-musl installed
#   * Vulkan SDK available (headers/loader) for the llama-cpp-4 `vulkan` link
#   * docker installed (for the image tag step)
#
# Usage:
#   scripts/release-gpu.sh [VERSION]
#   VERSION defaults to the version in the workspace Cargo.toml.
#
# This script does NOT touch the default `docker/Dockerfile` and does NOT
# modify default features. The GPU path stays opt-in only.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)"/\1/')}"

MUSL_TARGET=x86_64-unknown-linux-musl
IMAGE_BASE=arags-server-gpu

echo "==> Ensuring musl target is available"
rustup target add "$MUSL_TARGET" 2>/dev/null || true

echo "==> Building $MUSL_TARGET binary with --features llamacpp-vulkan"
# Baseline x86-64-v2 for portability; static libstdc++ for the C++ runtime.
RUSTFLAGS="-C target-cpu=x86-64-v2 -C link-arg=-static-libstdc++" \
cargo build --release --target "$MUSL_TARGET" -p arags-server --features llamacpp-vulkan

BIN="target/$MUSL_TARGET/release/arags-server"
strip "$BIN"
echo "==> Built: $BIN"

echo "==> Tagging Docker image $IMAGE_BASE:latest and $IMAGE_BASE:$VERSION"
docker build -f docker/Dockerfile.gpu \
  -t "$IMAGE_BASE:latest" \
  -t "$IMAGE_BASE:$VERSION" .

echo "==> Done. Images: $IMAGE_BASE:latest, $IMAGE_BASE:$VERSION"
echo "    NOTE: GPU/musl build is authored for Vulkan-SDK CI; not validated here."
