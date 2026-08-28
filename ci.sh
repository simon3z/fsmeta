#!/usr/bin/env bash
# CI quality gate for fsmeta.
# Usage: ./ci.sh
set -euo pipefail
cd "$(dirname "$0")"

cargo fmt --check
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo test

# Cognitive complexity threshold (arborist).
arborist src/ --threshold 20 --exceeds-only

echo "✓ All checks passed."
