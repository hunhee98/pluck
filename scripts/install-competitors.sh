#!/usr/bin/env bash
set -euo pipefail

# Installs competitor MCPs used in benchmarks. Idempotent.

echo "[pluck-bench] Installing a prior-art code search tool..."
cargo install a prior-art code search tool 2>/dev/null || echo "(a prior-art code search tool install skipped)"

echo "[pluck-bench] Installing ..."
# Zig-based; ship via prebuilt release
if ! command -v  >/dev/null 2>&1; then
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)  ASSET="-x86_64-linux" ;;
    Darwin-arm64)  ASSET="-aarch64-darwin" ;;
    Darwin-x86_64) ASSET="-x86_64-darwin" ;;
    *) echo "(: unsupported platform, skipped)"; exit 0 ;;
  esac
  echo "( prebuilt download placeholder for $ASSET — wire up after first release)"
fi
