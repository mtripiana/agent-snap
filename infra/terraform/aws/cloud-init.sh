#!/bin/bash
set -euxo pipefail

export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y curl git build-essential pkg-config libssl-dev python3-venv python3-pip nodejs npm

if ! command -v cargo >/dev/null 2>&1; then
  curl https://sh.rustup.rs -sSf | sh -s -- -y
fi
source /root/.cargo/env

cd /root
git clone "${repo_url}" agent-snap
cd /root/agent-snap

python3 -m venv .venv
.venv/bin/python -m pip install --upgrade pip pytest

cargo test --workspace --all-targets
cargo bench -p agentsnap-core --bench snapshot_latency
(cd sdks/node && npm test)
(cd sdks/python && ../../.venv/bin/python -m pytest -q)

rm -f /tmp/agentsnap.sock
AGENTSNAP_SOCKET=/tmp/agentsnap.sock cargo run -p agentsnap-daemon >/tmp/agentsnap-daemon.log 2>&1 &
sleep 1
AGENTSNAP_SOCKET=/tmp/agentsnap.sock cargo run -p agentsnap-cli -- exec --workspace . --command "echo hello from aws vm"

echo "DONE" > /root/agentsnap-demo-done.txt
