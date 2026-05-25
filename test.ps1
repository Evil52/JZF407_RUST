# Run logic-crate host tests on Windows. Detects the host triple automatically
# so no per-machine target hardcoding is needed.
#   .\test.ps1            # run all logic tests
#   .\test.ps1 auth       # filter to tests matching "auth"
$ErrorActionPreference = "Stop"
$host_triple = (rustc -vV | Select-String '^host:').ToString().Split()[1]
cargo test -p jzf407-logic --target $host_triple @args
