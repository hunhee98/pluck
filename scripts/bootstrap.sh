#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "[pluck] Bootstrap"

# Rust toolchain
if ! command -v rustup >/dev/null 2>&1; then
  echo "[pluck] Installing rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
fi
rustup show >/dev/null
rustup component add rustfmt clippy >/dev/null

# Faster linker on macOS (optional)
if [[ "$(uname)" == "Darwin" ]] && command -v brew >/dev/null 2>&1; then
  brew list zld >/dev/null 2>&1 || brew install michaeleisel/zld/zld 2>/dev/null || true
fi

# ONNX model (potion-code-16M)
mkdir -p models
if [[ ! -f models/potion-code-16m.onnx ]]; then
  echo "[pluck] Downloading potion-code-16M..."
  curl -L --fail -o models/potion-code-16m.onnx \
    "https://huggingface.co/minishlab/potion-code-16M/resolve/main/model.onnx" \
    || echo "[pluck] (skipped: model download failed — Phase 0 scaffold can still build)"
fi

# Benchmark repos
if [[ -f .gitmodules ]]; then
  git submodule update --init --recursive
fi

# Build
echo "[pluck] cargo build --release"
cargo build --release

echo "[pluck] Ready. Try: ./target/release/pluck version"
