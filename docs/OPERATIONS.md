# Pillar Operations Guide

How to run a Solana validator fleet with Pillar: stand up the control plane, add
validators, provision/update them, monitor, and operate day-to-day. Plus the gotchas
learned the hard way.

- **Controller** — one central control plane (web UI + gRPC + SQLite + Prometheus/Grafana).
- **Agent** (`pillar-agent`) — one per validator host; supervises the validator, reports
  status/logs, and runs commands the controller pushes.

---

## 1. Stand up the controller

Run on an **Ubuntu 24.04+** host (the binary needs GLIBC 2.39):

```bash
curl -sSL https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/install-controller.sh \
  | sudo bash -s -- --external-url https://<controller-host>:50051
```

This installs the controller, Prometheus, and Grafana (dashboards auto-provisioned), and
enables TLS on the gRPC port. Then:

- **Open the UI** at `http://<controller-host>:8080` (or put it behind a domain — see §7).
- **Change the default `admin` / `admin` credentials immediately** (avatar menu → change
  credentials). This is the single most important first step.
- The gRPC server runs with TLS, so agents must connect over `https://…:50051`.

---

## 2. Add a validator (onboard a host)

**Overview → "Add a Validator"** shows the exact one-line command (it embeds the controller
URL + auth token). Run it on the validator host:

```bash
curl -sSL https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/install-node.sh \
  | sudo bash -s -- --controller https://<controller-host>:50051 \
      --token <token> --http-url http://<controller-host>:8080
```

`install-node.sh` creates the `sol` user + sudoers, applies sysctl/limits tuning, installs
the Solana CLI + Rust toolchain, generates validator/vote keypairs, installs the agent, and
starts it. Within ~10s the host appears in **Overview** as a node.

### Onboarding an *existing* validator
If the host already runs a validator:
1. Run `install-node.sh` as above (it's idempotent and won't disturb a running validator).
2. In the node's detail page, open **Update Validator → Configure** and set the **client**,
   **cluster**, **service name**, and **paths** to match the existing setup, then save —
   this points the agent at the running service for health/lifecycle without re-provisioning.
   (If the systemd unit/paths already match Pillar's conventions, the agent picks it up from
   the config update alone.)

---

## 3. Create / provision a validator

In a node's detail page: **Setup Validator → Configure**. Pick a **client**, **cluster**,
paths/keypairs, ports, and submit. The controller renders a provisioning script and pushes it
to the agent, which runs it (download/build → systemd unit → start → report).

| Client | Notes |
|---|---|
| **Agave** | Production path. v2.x ships the binary in the release tarball; **v3.x/v4.x build from source** (no validator binary in tarballs) — allow 10–30 min on first provision. |
| **Jito** | Builds `jito-solana` from source. MEV flags are **cluster-aware** (block-engine + tip programs auto-filled per cluster); set relayer/shred-receiver if you run them. |
| **Firedancer / Frankendancer** | Builds `fdctl` from source. Needs an **AF_XDP-capable NIC** (or `net_provider=socket`) + hugepages; runs as root (drops to `sol`). |
| **Surfpool** | **Local test validator / mainnet-fork** (drop-in for `solana-test-validator`). No gossip/snapshot sync → instantly healthy. Ideal for testing + demos. |

After provisioning, the same panel becomes **"Update Validator"** — use it to change version,
flags, or cluster.

---

## 4. Update / upgrade

- **Re-provision** (Update Validator) to change cluster, flags, or rebuild a version.
- **Upgrade** a binary in place via the upgrade flow (download → SHA256-verify → swap →
  restart) when you have a prebuilt artifact URL.
- **Agent self-upgrade** appears as a button on the node when a newer agent is available.

---

## 5. Day-to-day operations

- **Health at a glance:** Overview shows each validator's state (healthy / behind / offline /
  unhealthy) + slots-behind. The node detail page shows live metrics (CPU/mem/disk, slots
  behind, restarts, uptime).
- **Logs:** node detail → Logs (Controller / Validator / Agent tabs), with **level + text
  filtering** and live streaming.
- **Grafana:** each node detail page has a **Grafana** link that opens the node-detail
  dashboard scoped to that validator (`var-node_id`). Fleet-wide dashboard is the Grafana
  home / fleet-overview.
- **Lifecycle actions** (bottom of node detail): **Restart**, **Recover** (snapshot
  recovery), **Stop**, **Delete**.

---

## 6. Best practices

**Security**
- Change `admin/admin` before exposing the UI. Keep gRPC on TLS (`https`).
- Don't expose the gRPC port through a proxy that can't pass TLS/HTTP2 (use DNS-only if
  fronting with Cloudflare).
- Back up `authorized-withdrawer-keypair.json` offline — losing it is unrecoverable.

**Storage & host**
- Put `ledger`, `accounts`, and `snapshots` on fast NVMe with ample space. On hosts without a
  separate `/mnt`, point the paths at the data disk explicitly (don't leave defaults like
  `/mnt/ledger` if that isn't mounted).
- Reserve hugepages for Firedancer (2 MB at runtime via `fdctl configure init`; 1 GB needs
  GRUB + reboot). Ensure `fs.nr_open >= 1024000`.

**Networking / staying synced**
- A validator must have **inbound UDP reachable** (gossip + dynamic port range) so turbine
  delivers blocks; otherwise it falls back to repair and **drifts behind** over time.
- An **unstaked** validator sits at the edge of turbine and may lag on a busy cluster — give
  it stake (delegate to its vote account) to stay synced, or use it as an RPC node.
- Behind NAT/upstream-firewalled hosts, use `--no-port-check` (a provision option) so the
  validator proceeds to bootstrap.

**Switching clusters**
- Switching a node's cluster (e.g. testnet → devnet) requires a **clean ledger** — a stale
  genesis from the old cluster causes a *genesis hash mismatch* and the node will reject every
  peer. Clear `ledger/` + `accounts/` (keep a fresh snapshot for the new cluster) when
  changing clusters.

---

## 7. Putting the UI behind a domain (optional)

- Reserve a **static IP** for the controller host so DNS doesn't break on reboot.
- Point an **A record** at it (DNS-only if using Cloudflare and agents hit gRPC directly).
- The controller UI serves on `:8080`; to use the bare domain on `:80`, either run a reverse
  proxy (Caddy gives automatic HTTPS) or redirect `80 → 8080`.

---

## 8. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Node shows **offline**, RPC not serving | Validator still bootstrapping (snapshot download/replay) — check Validator logs. |
| **slots_behind grows** over time | Inbound UDP/turbine not reaching the host, or unstaked node — see §6 Networking. |
| **Genesis hash mismatch** in logs | Stale ledger from a different cluster — clear `ledger/` + `accounts/`. |
| Agent fails to register, `h2 FRAME_SIZE_ERROR` | TLS scheme mismatch — agent endpoint must be `https://` when the controller has TLS. |
| Grafana **"Dashboard not found"** | Dashboards not provisioned — ensure the JSONs are in `/var/lib/grafana/dashboards/pillar/`. |
| Firedancer won't start | Check `fdctl configure init` (hugepages), `fs.nr_open`, and NIC AF_XDP support (or `net_provider=socket`). |
| Want a guaranteed-healthy node for a demo | Provision **Surfpool** — local fork, instantly healthy, no sync. |
