#!/usr/bin/env bash
# Run logic-crate host tests on macOS/Linux. Detects the host triple automatically
# so no per-machine target hardcoding is needed.
#   ./test.sh            # run all logic tests
#   ./test.sh auth       # filter to tests matching "auth"
set -euo pipefail
host_triple=$(rustc -vV | sed -n 's/^host: //p')
cargo test -p jzf407-logic --target "$host_triple" "$@"
