#!/usr/bin/env bash
# Pillar End-to-End Lifecycle Test
# Runs against a live controller to verify the full node lifecycle.
#
# Usage:
#   bash scripts/test-lifecycle.sh
#   CONTROLLER_URL=http://10.0.0.1:8080 NODE_ID=my-node bash scripts/test-lifecycle.sh

set -euo pipefail

CONTROLLER_URL="${CONTROLLER_URL:-http://139.84.215.43:8080}"
NODE_ID="${NODE_ID:-}"
POLL_INTERVAL=15
POLL_COUNT=5

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass=0
fail=0

log_pass() {
    echo -e "  ${GREEN}PASS${NC}: $1"
    pass=$((pass + 1))
}

log_fail() {
    echo -e "  ${RED}FAIL${NC}: $1"
    fail=$((fail + 1))
}

log_skip() {
    echo -e "  ${YELLOW}SKIP${NC}: $1"
}

log_section() {
    echo ""
    echo "============================================"
    echo " $1"
    echo "============================================"
}

api() {
    curl -sf --max-time 10 "${CONTROLLER_URL}$1" 2>/dev/null
}

api_post() {
    local url="$1"
    shift
    curl -sf --max-time 30 -X POST -H "Content-Type: application/json" "${CONTROLLER_URL}${url}" "$@" 2>/dev/null
}

# -------------------------------------------------------------------------
# Phase 1: Controller Health
# -------------------------------------------------------------------------
log_section "Phase 1: Controller Health"

overview=$(api "/api/overview" || true)
if [ -n "$overview" ]; then
    total=$(echo "$overview" | jq -r '.total // 0')
    log_pass "GET /api/overview — total nodes: $total"
else
    log_fail "GET /api/overview — no response"
fi

metrics=$(api "/metrics" || true)
if echo "$metrics" | grep -q "pillar_"; then
    log_pass "GET /metrics — prometheus metrics present"
else
    log_fail "GET /metrics — no pillar_ metrics found"
fi

onboard=$(api "/api/onboard-command" || true)
if echo "$onboard" | jq -e '.command' > /dev/null 2>&1; then
    log_pass "GET /api/onboard-command — command returned"
else
    log_fail "GET /api/onboard-command — missing command field"
fi

# -------------------------------------------------------------------------
# Phase 2: Node Registration
# -------------------------------------------------------------------------
log_section "Phase 2: Node Registration"

nodes=$(api "/api/nodes" || true)
if [ -z "$nodes" ] || [ "$nodes" = "[]" ]; then
    log_fail "GET /api/nodes — no nodes registered"
    echo "  Register a node first, then re-run this script."
    echo ""
    echo "Results: $pass passed, $fail failed"
    exit 1
fi

node_count=$(echo "$nodes" | jq 'length')
log_pass "GET /api/nodes — $node_count node(s) found"

# Auto-detect NODE_ID if not set
if [ -z "$NODE_ID" ]; then
    NODE_ID=$(echo "$nodes" | jq -r '.[0].node_id')
    echo "  Auto-detected NODE_ID: $NODE_ID"
fi

node_detail=$(api "/api/nodes/$NODE_ID" || true)
if echo "$node_detail" | jq -e '.node_id' > /dev/null 2>&1; then
    state=$(echo "$node_detail" | jq -r '.lifecycle_state')
    log_pass "GET /api/nodes/$NODE_ID — state: $state"
else
    log_fail "GET /api/nodes/$NODE_ID — node not found"
    echo ""
    echo "Results: $pass passed, $fail failed"
    exit 1
fi

# Check live_status
if echo "$node_detail" | jq -e '.live_status' > /dev/null 2>&1; then
    live_state=$(echo "$node_detail" | jq -r '.live_status.state // "null"')
    log_pass "live_status present — operator state: $live_state"
else
    log_skip "live_status is null (node may not be reporting yet)"
fi

# -------------------------------------------------------------------------
# Phase 3: Provision (only if state is registered)
# -------------------------------------------------------------------------
log_section "Phase 3: Provision"

current_state=$(echo "$node_detail" | jq -r '.lifecycle_state')
if [ "$current_state" = "registered" ]; then
    echo "  Node is in 'registered' state — sending provision command."
    provision_body='{
        "client": "agave",
        "version": "2.2.5",
        "cluster": "testnet",
        "identity_keypair_path": "/home/sol/validator-keypair.json",
        "vote_account_keypair_path": "",
        "ledger_path": "/mnt/ledger",
        "snapshot_path": "/mnt/snapshots",
        "accounts_path": "/mnt/accounts",
        "entrypoints": ["entrypoint.testnet.solana.com:8001", "entrypoint2.testnet.solana.com:8001", "entrypoint3.testnet.solana.com:8001"],
        "known_validators": ["5D1fNXzvv5NjV1ysLjirC4WY92RNsVH18vjmcszZd8on"],
        "download_url": "https://github.com/anza-xyz/agave/releases/download/v2.2.5/solana-release-x86_64-unknown-linux-gnu.tar.bz2",
        "sha256": "placeholder",
        "jito_mev": false,
        "jito_block_engine_url": "",
        "yellowstone_grpc": false
    }'
    resp=$(api_post "/api/nodes/$NODE_ID/provision" -d "$provision_body" || true)
    if echo "$resp" | jq -e '.ok == true' > /dev/null 2>&1; then
        log_pass "POST provision — command accepted"
    else
        msg=$(echo "$resp" | jq -r '.message // "unknown error"')
        log_fail "POST provision — $msg"
    fi
else
    log_skip "Node already provisioned (state: $current_state)"
fi

# -------------------------------------------------------------------------
# Phase 4: Monitoring (poll status)
# -------------------------------------------------------------------------
log_section "Phase 4: Monitoring (polling ${POLL_COUNT}x at ${POLL_INTERVAL}s)"

for i in $(seq 1 $POLL_COUNT); do
    sleep "$POLL_INTERVAL"
    detail=$(api "/api/nodes/$NODE_ID" || true)
    if [ -z "$detail" ]; then
        log_fail "Poll $i — no response"
        continue
    fi

    state=$(echo "$detail" | jq -r '.lifecycle_state')
    ls=$(echo "$detail" | jq -r '.live_status.local_slot // "n/a"')
    cpu=$(echo "$detail" | jq -r '.live_status.cpu_usage_percent // "n/a"')
    mem=$(echo "$detail" | jq -r '.live_status.memory_used_bytes // "n/a"')
    echo "  Poll $i/$POLL_COUNT — state=$state, local_slot=$ls, cpu=$cpu, mem=$mem"
done
log_pass "Monitoring polls completed"

# -------------------------------------------------------------------------
# Phase 5: Log Streaming
# -------------------------------------------------------------------------
log_section "Phase 5: Log Streaming"

logs=$(api "/api/nodes/$NODE_ID/logs?limit=10" || true)
if [ -n "$logs" ] && [ "$logs" != "[]" ]; then
    log_count=$(echo "$logs" | jq 'length')
    latest=$(echo "$logs" | jq -r '.[0].message // "empty"' | head -c 80)
    log_pass "GET /api/nodes/$NODE_ID/logs — $log_count entries (latest: $latest)"
else
    log_skip "No logs yet (log collector may not have flushed)"
fi

# Quick SSE check — connect for 3 seconds and see if we get any events.
sse_output=$(timeout 3 curl -sf -N "${CONTROLLER_URL}/api/nodes/${NODE_ID}/logs/stream" 2>/dev/null || true)
if [ -n "$sse_output" ]; then
    log_pass "SSE /api/nodes/$NODE_ID/logs/stream — received events"
else
    log_skip "SSE — no events in 3s (normal if node is quiet)"
fi

# -------------------------------------------------------------------------
# Phase 6: Restart
# -------------------------------------------------------------------------
log_section "Phase 6: Restart"

resp=$(api_post "/api/nodes/$NODE_ID/restart" || true)
if echo "$resp" | jq -e '.ok == true' > /dev/null 2>&1; then
    log_pass "POST restart — command accepted"
    echo "  Waiting 30s for restart to take effect..."
    sleep 30
    detail=$(api "/api/nodes/$NODE_ID" || true)
    state=$(echo "$detail" | jq -r '.lifecycle_state // "unknown"')
    log_pass "Post-restart state: $state"
else
    msg=$(echo "$resp" | jq -r '.message // "no response"')
    log_fail "POST restart — $msg"
fi

# -------------------------------------------------------------------------
# Phase 7: Recover
# -------------------------------------------------------------------------
log_section "Phase 7: Recover"

resp=$(api_post "/api/nodes/$NODE_ID/recover" || true)
if echo "$resp" | jq -e '.ok == true' > /dev/null 2>&1; then
    log_pass "POST recover — command accepted"
    echo "  Waiting 30s for recovery to start..."
    sleep 30
    detail=$(api "/api/nodes/$NODE_ID" || true)
    state=$(echo "$detail" | jq -r '.lifecycle_state // "unknown"')
    log_pass "Post-recover state: $state"
else
    msg=$(echo "$resp" | jq -r '.message // "no response"')
    log_fail "POST recover — $msg"
fi

# -------------------------------------------------------------------------
# Phase 8: Upgrade
# -------------------------------------------------------------------------
log_section "Phase 8: Upgrade"

upgrade_body='{
    "binary_name": "agave-validator",
    "version": "2.2.5",
    "download_url": "https://github.com/anza-xyz/agave/releases/download/v2.2.5/solana-release-x86_64-unknown-linux-gnu.tar.bz2",
    "sha256": "placeholder",
    "reason": "lifecycle test"
}'
resp=$(api_post "/api/nodes/$NODE_ID/upgrade" -d "$upgrade_body" || true)
if echo "$resp" | jq -e '.ok == true' > /dev/null 2>&1; then
    log_pass "POST upgrade — command accepted"
else
    msg=$(echo "$resp" | jq -r '.message // "no response"')
    log_fail "POST upgrade — $msg"
fi

# -------------------------------------------------------------------------
# Summary
# -------------------------------------------------------------------------
echo ""
echo "============================================"
echo " Summary"
echo "============================================"
echo -e "  ${GREEN}Passed${NC}: $pass"
echo -e "  ${RED}Failed${NC}: $fail"
echo ""

if [ "$fail" -gt 0 ]; then
    exit 1
fi
