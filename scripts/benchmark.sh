#!/usr/bin/env bash
set -euo pipefail

cargo bench -p agentsnap-core --bench snapshot_latency
