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
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-controller.sh \
  | sudo bash -s -- --external-url https://<controller-host>:50051
```

This installs the controller, Prometheus, and Grafana (dashboards auto-provisioned), and
enables TLS on the gRPC port. Then:

- **Open the UI** at `http://<controller-host>:8080` (or put it behind a domain — see §10).
- **Change the default `admin` / `admin` credentials immediately** (avatar menu → change
  credentials). This is the single most important first step.
- The gRPC server runs with TLS, so agents must connect over `https://…:50051`.

---

## 2. Add a validator (onboard a host)

**Overview → "Add a Validator"** shows the exact one-line command (it embeds the controller
URL + auth token). Pick the target cluster with the **mainnet-beta / testnet / devnet** toggle
above the command — it fills in `--cluster` for you — then copy and run it on the host:

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-node.sh | sudo bash -s -- --controller https://<controller-host>:50051 --token <token> --http-url http://<controller-host>:8080 --cluster testnet
```

The command is a single line — paste it as-is (line continuations break when pasted).
`install-node.sh` creates the `sol` user + sudoers, applies sysctl/limits tuning, installs
the Solana CLI + Rust + Go toolchains, generates validator/vote keypairs, installs the agent,
and starts it. Within ~10s the host appears in **Overview** as a node. It's **idempotent** —
safe to re-run to pick up new toolchains or sudoers.

### Onboarding an *existing* validator
If the host already runs a validator you don't want Pillar to rebuild:
1. Run `install-node.sh` as above — it won't disturb a running validator.
2. In the node's detail page, **Edit Config** and set the **client**, **cluster**,
   **service name**, and **paths** to match the existing setup, then save. This points the
   agent at the running service for health/lifecycle **without re-provisioning**. If the
   systemd unit and paths already match Pillar's conventions, the config update alone is enough.

---

## 3. Create / provision a validator

On a freshly onboarded node the Validator Configuration card shows **Setup Validator**; open it
and:

1. **Client** — pick from the dropdown (see the table below).
2. **Cluster** — mainnet-beta / testnet / devnet. This seeds entrypoints, known validators, and
   the reference RPC.
3. **Version** — type it, or click the **▾** to pick from the client's recent GitHub releases
   (fetched live). A yellow hint shows the version the node is currently running when it differs.
4. **Node type** — Validator / RPC / Archival (adjusts the flag preset).
5. **Paths / keypairs / ports** — defaults suit a standard `/mnt` layout; override as needed.
6. Submit. The controller renders a provisioning script and pushes it to the agent, which runs
   it as `sol`: download or build → write the systemd unit → start → report. A spinner banner
   tracks progress and disables other actions until it finishes; the outcome (success/failure
   with the real error) is shown when it completes. Follow live output in the **Validator** and
   **Agent** log tabs.

| Client | Notes |
|---|---|
| **Agave** | Production path. v2.x ships the binary in the release tarball; **v3.x/v4.x build from source** (no validator binary in tarballs) — allow 10–30 min on first provision. v4.2+ needs raw-socket capabilities (granted in the unit). |
| **Jito** | Builds `jito-solana` from source. MEV flags are **cluster-aware** (block-engine + tip programs auto-filled per cluster); set relayer/shred-receiver if you run them. |
| **Firedancer / Frankendancer** | Builds `fdctl` from source (Rust). Needs an **AF_XDP-capable NIC** (or `net_provider=socket`) + hugepages; **starts as root** and drops to `sol`. |
| **Surfpool** | **Local test validator / mainnet-fork** (drop-in for `solana-test-validator`). No gossip/snapshot sync → instantly healthy. Ideal for testing + demos. Installed from the versioned release tarball. |
| **Mithril** | Go full/verifying node (`Overclock-Validator/mithril`). **Builds from source with Go** when no download URL is given. Bootstraps by downloading a snapshot and building AccountsDB, then serves Solana RPC on `:8899`. Needs ample fast NVMe (AccountsDB ~500 GB on mainnet). |

After provisioning, the same panel becomes **"Update Validator"** — use it to change version,
flags, or cluster.

---

## 4. Upgrading

There are three distinct upgrade paths:

**a) Upgrade the validator version (re-provision)**
Node detail → **Update Validator → Configure**, change **Version** (and any flags), submit.
The agent re-runs provisioning: for Agave v3/v4 and Jito it rebuilds from source; for v2.x it
fetches the release tarball. The old service is stopped, the new binary installed, and the
service restarted. Watch progress in the **Validator** logs tab.

**b) Upgrade a binary in place (fast, prebuilt)**
If you have a prebuilt artifact + SHA256, use the upgrade flow (`POST /api/nodes/:id/upgrade`
with `binary_name`, `version`, `download_url`, `sha256`). The agent downloads →
`sha256sum -c` (fails fast on mismatch) → stops the service → installs → restarts. This
avoids a source rebuild.

**c) Upgrade the agent**
When the controller detects a newer agent release, an **"Upgrade Agent to vX"** button
appears on the node. It swaps the agent binary and restarts via systemd. The controller
itself upgrades with `POST /api/upgrade-controller` (or re-run `install-controller.sh`).

> Tip: for zero-surprise upgrades, test the new version on a **Surfpool** node first (instant,
> disposable), then roll it to real validators.

---

## 5. Day-to-day operations

- **Health at a glance:** Overview shows each validator's state (healthy / behind / offline /
  unhealthy, or a **deploying** spinner while a provision runs) + slots-behind. A bootstrapping
  node (downloading a snapshot / replaying) reads as **starting up**, not offline. The node
  detail page shows live metrics: CPU/mem/disk, slots behind, restarts, and **validator process
  uptime** (real process start time, so an out-of-band restart is visible). IP addresses are
  click-to-copy.
- **Logs:** node detail → Logs (Controller / Validator / Agent tabs), with **level + text
  filtering** and live streaming. Each tab fetches its own service history.
- **Metrics:** the nav's **Metrics** link opens the global fleet dashboard; each node-detail
  page and Overview row has a **Metrics** link that opens the per-node dashboard scoped to
  that validator (`var-node_id`). Per-node panels include CPU/mem/disk, slots behind, and
  **RPC connections** (established TCP connections to the validator's RPC port). (Grafana
  dashboards under the hood.)
- **Versions & upgrades:** the running controller version shows in the nav; click it (**↻**)
  to force an update check. When a newer controller or agent release exists, an upgrade banner
  appears.
- **Lifecycle actions** (top of node detail): **Restart**, **Recover** (snapshot
  recovery), **Stop**, **Delete** — disabled while a deployment is in progress.

---

## 6. Alerting (Slack / PagerDuty / Telegram)

Pillar exposes per-node metrics at the controller's `/metrics` (scraped by Prometheus), so
alerting is done in **Grafana's unified alerting** against the `pillar-prometheus` data source.

### Common alert rules
Create these in **Grafana → Alerting → Alert rules** (or provision them via
`/etc/grafana/provisioning/alerting/*.yaml`). Useful conditions on the Pillar metrics:

| Alert | Expression | Meaning |
|---|---|---|
| Validator unhealthy/offline | `pillar_node_healthy == 0` | agent reports the node not healthy |
| Lagging behind | `pillar_node_slots_behind > 500` (for 10m) | falling behind the cluster tip; also covers slow catch-up after a restart |
| Metrics pipeline dark | `absent(pillar_node_healthy)` | no metrics reaching Prometheus — agent, controller, or remote_write down; all other alerts are blind |
| Frequent restarts | `increase(pillar_node_restarts_total[15m]) > 3` | crash-looping |
| Disk filling | `pillar_system_disk_used_bytes / pillar_system_disk_total_bytes > 0.9` | low disk |

Label each rule (e.g. `severity: page` vs `severity: warn`) so notification policies can route
them differently. A starter set lives in `controller/dashboards/grafana/alert-rules.json`.

### Connect a notification channel (contact points)
**Grafana → Alerting → Contact points → Add contact point**:

- **Slack** — type *Slack*, paste an [incoming webhook URL]
  (`https://hooks.slack.com/services/…`), set the channel.
- **PagerDuty** — type *PagerDuty*, paste the **Integration Key** (Events API v2 routing key)
  from a PD service.
- **Telegram** — type *Telegram*, paste the **bot token** (from @BotFather) and the **chat ID**.

Then **Alerting → Notification policies**: route by label (e.g. `severity=page` →
PagerDuty/Telegram, `severity=warn` → Slack). Use **Test** on the contact point to confirm
delivery, and a **mute timing** for maintenance windows.

> Provisioning these as code (checked-in YAML under `provisioning/alerting/`) makes them
> reproducible across controllers; the webhook URL / PD key / bot token are the only secrets
> to supply per environment.

## 7. Best practices

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

## 8. Security & data

**What Pillar stores, and where**
- **Controller SQLite** (`/var/lib/pillar/controller.db`): node status + history, logs,
  per-node provision configs, the admin username + **argon2 password hash**, and the gRPC
  **auth token**. It does **not** hold validator private keys.
- **Validator hosts**: the **identity / vote / authorized-withdrawer keypairs** live on each
  host under `/home/sol/*.json` and are never sent to the controller. The
  authorized-withdrawer key is the crown jewel — back it up offline.
- **Controller TLS material**: `/etc/pillar/certs/` (CA + server cert/key).

**Encryption**
- **In transit (agent ↔ controller): TLS.** The gRPC channel uses the controller's CA; agents
  pull `ca.pem` at install. Always use `https://…:50051`.
- **The web UI (:8080) is plain HTTP by default** — login + API traffic is unencrypted on the
  wire. Front it with HTTPS (Caddy / Cloudflare — see §10) before exposing it publicly.
- **At rest: the SQLite DB is not encrypted.** It's owned by the `pillar` user (restrict file
  perms); use disk/volume encryption if you need at-rest guarantees. The admin password is
  argon2-hashed, but the gRPC auth token is stored in clear — treat the DB as a secret.

**Who can read data**
- **Web UI / JSON API** — gated by admin login (session cookie). Anyone with the admin
  credentials can read all status, logs, and configs and run lifecycle actions.
- **`/metrics` is unauthenticated** — it exposes per-node metrics to anyone who can reach it.
- **Grafana ships with anonymous Admin access** (no login) for convenience — anyone who can
  reach `:3000` / `/grafana` can view dashboards. Disable anonymous auth + set an admin
  password for anything beyond a demo.

**Hardening checklist**
- Change `admin/admin` immediately; use a strong password.
- Front the UI with HTTPS; don't expose `:8080` / `:3000` / `/metrics` to the public internet
  unprotected.
- Keep gRPC on TLS. Today a single shared auth token is used for all agents — consider
  per-node tokens / mTLS so a leaked token can't impersonate the whole fleet.
- The `sol` sudoers is a fixed minimal allow-list — keep it that way.
- Back up the authorized-withdrawer keypair offline.

---

## 9. Backup, recovery & loss of access

**Lost admin password** — on the controller host, clear the stored credential so it re-seeds
`admin/admin` on restart, then change it again:

```bash
sqlite3 /var/lib/pillar/controller.db \
  "DELETE FROM settings WHERE key IN ('admin_username','admin_password_hash');"
sudo systemctl restart pillar-controller
```

The gRPC auth token is untouched, so agents stay connected.

**Lost the controller host** — **validators keep running.** The agent is autonomous: its
reconcile loop keeps supervising the validator (health, restarts, recovery) while the
controller is down, and it reconnects automatically when the controller returns. You lose the
dashboard/history until it's back, not the validators. To restore:
- Stand up a new controller and **restore the backup** (below). If you restore the same
  SQLite DB **and** `/etc/pillar/certs`, the auth token + CA match and existing agents
  reconnect with no changes.
- If you generate a **new** CA instead, redistribute the new `ca.pem` to agents (or re-run
  `install-node.sh`) — their pinned CA won't match the old one.

**What to back up**
- `/var/lib/pillar/controller.db` — all fleet state, configs, logs, credentials + auth token.
- `/etc/pillar/` — controller config + `certs/`. Backing these up makes controller recovery a
  restore-and-restart.
- Per validator host: the keypairs under `/home/sol/` (especially authorized-withdrawer) —
  offline.

**Validator recovery**
- **Recover** (node detail) triggers snapshot recovery: the agent wipes a stale/corrupt ledger
  and re-bootstraps from a snapshot — use it when a validator is wedged or far behind.
- **Restart** for a clean service restart; **Stop** to take it offline.

**Sessions** are in-memory, so a controller restart logs everyone out — just log back in (it
doesn't affect agents or validators).

---

## 10. Putting the UI behind a domain (optional)

- Reserve a **static IP** for the controller host so DNS doesn't break on reboot.
- Point an **A record** at it (DNS-only if using Cloudflare and agents hit gRPC directly).
- The controller UI serves on `:8080`; to use the bare domain on `:80`, either run a reverse
  proxy (Caddy gives automatic HTTPS) or redirect `80 → 8080`.

---

## 11. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| Node shows **offline**, RPC not serving | Validator still bootstrapping (snapshot download/replay) — check Validator logs. |
| **slots_behind grows** over time | Inbound UDP/turbine not reaching the host, or unstaked node — see §7 (Networking). |
| **Genesis hash mismatch** in logs | Stale ledger from a different cluster — clear `ledger/` + `accounts/`. |
| Agent fails to register, `h2 FRAME_SIZE_ERROR` | TLS scheme mismatch — agent endpoint must be `https://` when the controller has TLS. |
| Grafana **"Dashboard not found"** | Dashboards not provisioned — ensure the JSONs are in `/var/lib/grafana/dashboards/pillar/`. |
| Firedancer won't start | Check `fdctl configure init` (hugepages), `fs.nr_open`, and NIC AF_XDP support (or `net_provider=socket`). |
| Want a guaranteed-healthy node for a demo | Provision **Surfpool** — local fork, instantly healthy, no sync. |
