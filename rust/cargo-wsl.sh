#!/bin/sh
# Run from WSL, or: wsl --exec sh /mnt/c/.../rust/cargo-wsl.sh test --workspace
set -eu
cd "$(dirname "$0")"
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
# Keep incremental builds on the Linux filesystem rather than the mounted checkout.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/goa2-rust/target}"
# Bound test-harness concurrency; explicit environment/CLI choices still win.
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
exec cargo "$@"
