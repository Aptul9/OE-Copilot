set -e
. ~/.cargo/env
SRC="${SRC:-/mnt/c/Users/Antonio/Downloads/personal projects/OE-Copilot/teams-relay/app/webapp-rs}"
mkdir -p ~/trcalls-dbg
rsync -a --delete --exclude target "$SRC/" ~/trcalls-dbg/
cd ~/trcalls-dbg
export CARGO_TARGET_DIR=~/trcalls-dbg-target
export CMAKE_POLICY_VERSION_MINIMUM=3.5
export CC_x86_64_unknown_linux_musl=musl-gcc
start=$(date +%s)
cargo build --release --locked --target x86_64-unknown-linux-musl --bin teamsrelay-calling 2>&1 | tail -15
echo "build $(( $(date +%s) - start ))s"
ls -la $CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/teamsrelay-calling
file $CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/teamsrelay-calling
