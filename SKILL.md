# Pillar Operational Runbooks

## Bootstrap / Snapshot Download Loop

### Symptoms

When a validator starts fresh (or after a snapshot wipe), it must download a snapshot from peers before it can participate in the cluster. During this phase:

- Agent reports `state=Off` because the validator process keeps restarting
- Crash loop detection triggers (3+ restarts/hour) — UI shows "crash loop detected"
- In reality, the validator is progressing through bootstrap: discovering peers, downloading snapshots, and attempting to start from the downloaded state

The typical bootstrap cycle looks like:

1. Validator starts, searches for RPC peers with snapshots
2. Begins downloading a snapshot (can be 50-100+ GB on mainnet)
3. If download completes, validator loads the snapshot and starts catching up
4. If download fails (peer disconnects, stale snapshot 404, blacklisted), validator exits and systemd restarts it
5. Repeat until a valid snapshot is fully downloaded and loaded

### Diagnosis

Check journald for snapshot/download/bootstrap activity:

```bash
# See download progress
sudo journalctl -u solana-validator -f --no-pager | grep -i "download\|snapshot\|bootstrap"

# Check for 404 / blacklist errors
sudo journalctl -u solana-validator --since "1 hour ago" --no-pager | grep -i "404\|blacklist\|stale"

# Check how many times the validator has restarted
systemctl show solana-validator --property=NRestarts
```

Example healthy download progress lines:

```
Downloading 52428800000 bytes from 10.0.0.5:8899...
downloaded 548684968 bytes 10.4% 13474726.0 bytes/s
downloaded 1097369936 bytes 20.9% 14523891.0 bytes/s
...
Downloaded 52428800000 bytes in 3845s
```

### Pillar Observability

When the agent detects snapshot download progress in journald logs, it exposes:

- **Prometheus metrics**: `pillar_snapshot_download_bytes`, `pillar_snapshot_download_total_bytes`, `pillar_snapshot_download_speed_bps`
- **UI logs**: Bootstrap/download INFO lines pass through the log filter even when `validator_min_level: warn`
- **Grafana**: Node Detail dashboard has a "Snapshot Download" row with progress gauge, speed chart, and bytes downloaded

### Fix: Stale Snapshot / Repeated 404s

If the validator is stuck in a loop where it keeps downloading stale snapshots that fail validation:

```bash
# 1. Stop the validator
sudo systemctl stop solana-validator

# 2. Wipe snapshots and ledger (accounts are rebuilt from snapshot)
sudo rm -rf /mnt/snapshots/*
sudo rm -rf /mnt/ledger/*

# 3. Start the validator — it will re-download from scratch
sudo systemctl start solana-validator
```

### Verification

After restarting:

1. Watch logs for download progress: `sudo journalctl -u solana-validator -f`
2. Check Pillar UI logs tab — download lines should appear
3. Check Prometheus: `curl localhost:9090/metrics | grep snapshot_download`
4. Once download completes, validator loads snapshot and starts catching up — `state` transitions from `Off` → `StartingUp` → `Behind` → `Healthy`
