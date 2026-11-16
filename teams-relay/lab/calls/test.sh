set -e
. ~/.cargo/env
SRC="${SRC:-/mnt/c/Users/Antonio/Downloads/personal projects/OE-Copilot/teams-relay/app/webapp-rs}"
rsync -a --delete --exclude target "$SRC/" ~/trcalls-dbg/
cd ~/trcalls-dbg
export CARGO_TARGET_DIR=~/trcalls-dbg-target-test
export CMAKE_POLICY_VERSION_MINIMUM=3.5
cargo test --locked ${FILTER-calling} 2>&1 | grep -E '^test |test result|error|warning|panicked|FAILED' | grep -v ' ok$' | tail -40
cargo clippy --locked --all-targets 2>&1 | grep -E '^(warning|error)' -A6 | head -40
cargo fmt --check 2>&1 | head -20 && echo fmt-ok
