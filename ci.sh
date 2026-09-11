#!/usr/bin/env bash
# CI quality gate for fsmeta.
# Usage: ./ci.sh
set -euo pipefail
cd "$(dirname "$0")"

cargo fmt --check
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo test

# Complexity/length/arg-count gates (clippy, configured in clippy.toml).
echo "✓ All checks passed."
