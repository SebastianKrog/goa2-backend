#!/bin/sh
# Compatibility entry point retained for existing WSL and verification callers.
set -eu
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/goa2-rust/target}"
exec sh "$(dirname "$0")/cargo-dev.sh" "$@"
