#!/bin/sh
# Optional platform-neutral wrapper. Ordinary cargo commands also work.
set -eu
cd "$(dirname "$0")"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
exec cargo "$@"
