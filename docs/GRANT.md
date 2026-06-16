# Pillar — Grant Submission

**Pillar** is an open-source operations platform for running Solana validators at fleet
scale. It replaces the usual pile of ad-hoc shell scripts, `cron` jobs, and manual SSH
sessions with two small Rust binaries: an **agent** on every node and a central
**controller** with a web UI.

This document is the grant-submission overview: what the project does, what is working
today (with a live demo), how it is deployed, and the roadmap for supporting validator
clients beyond Agave.

---

## Problem

Operating Solana validators is operationally heavy:

- **Lifecycle management** — validators crash, fall behind, or need restarts; recovery
  from a snapshot is multi-step and error-prone when done by hand.
- **Observability** — operators stitch together journald, Prometheus exporters, and
  custom RPC polling per node.
- **Fleet scale** — doing all of the above across N nodes, by hand, does not scale.
- **Upgrades & provisioning** — installing a new validator or rolling a binary upgrade is
  a manual, inconsistent process.

Pillar turns these into a single managed control plane.

---

## What Pillar does

| Capability | Agent | Controller |
|---|---|---|
| Validator health checks (RPC + validator-process) | ✅ | — |
| State machine + crash-loop detection + auto-recovery | ✅ | — |
| Snapshot download/recovery with progress observability | ✅ | — |
| System/process metrics (CPU, mem, disk, net) | ✅ | — |
| Prometheus `/metrics` | ✅ (per-node) | ✅ (fleet, labeled) |
| journald log streaming | ✅ | ✅ (stored + SSE to UI) |
| gRPC status reporting + command stream | ✅ | ✅ (5 RPCs) |
| Web UI (fleet overview, node detail, logs, provisioning) | — | ✅ |
| Provision / upgrade / restart / stop / recover commands | ✅ (executor) | ✅ (script render + push) |
| Auth (admin login + bearer token) | — | ✅ |
| SQLite persistence + retention pruning | — | ✅ |

See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for the full design and
[`SKILL.md`](./SKILL.md) for the operational runbook.

---

## Architecture (one data type, end to end)

A single controller manages **many** validators — every node runs its own agent, and
all agents connect to the one shared controller.

```
Agent (one per validator node)            Controller (one, manages every validator)
   │  reconcile loop (health, state)         │   ▲
   │  enrich w/ system metrics               │   │  ... agent N
   │── RegisterNode ────────────────────────►│   │  ... agent 2
   │── ReportStatus (every 10s) ────────────►│ ◄─┘  many agents → one controller
   │◄─ CommandStream (server-stream) ─────────│  push restart/provision/upgrade
   │── PushLogs (client-stream) ─────────────►│  logs table + SSE to web UI
   │                                          │  serve web UI + /metrics + Grafana
```

A single `NodeStatus` proto type flows agent → controller → SQLite → web UI + `/metrics`.
Both binaries share generated stubs via `pillar-shared` (`extern_path` keeps proto types
identical on both sides). No Solana SDK dependency — health checks are raw JSON-RPC.

---

## Live demo

A working control plane is deployed on GCP for evaluation.

| Component | Endpoint |
|---|---|
| Control-plane web UI | `http://34.107.8.212:8080` |
| Controller gRPC (agent endpoint) | `34.107.8.212:50051` |
| Grafana (embedded dashboards) | `http://34.107.8.212:3000` |
| Prometheus | `localhost:9091` (on the VM) |

**Default login:** `admin` / `admin` — **change this before any real use** (UI →
account settings, or the `/api/change-credentials` endpoint).

Deployment details:

- **GCP project:** `trading-prod-494016`
- **Region/zone:** `europe-west3-a` (Frankfurt) — closest GCP region to the EU validator
  host, minimizing gRPC/UI latency.
- **VM:** `pillar-controller`, `e2-standard-2` (2 vCPU / 8 GB), Ubuntu 24.04 LTS, 30 GB.
- **Install:** one command, `install-controller.sh`, which also provisions Prometheus +
  Grafana with dashboards out of the box:

  ```bash
  curl -sSL https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/install-controller.sh \
    | sudo bash -s -- --external-url http://34.107.8.212:50051
  ```

> **Note:** the published binaries are built against GLIBC 2.39, so the controller host
> must be Ubuntu 24.04 (Noble) or newer. A 22.04 host fails with
> `version 'GLIBC_2.39' not found`. This is captured as a TODO below (static/musl build).

### Adding a validator node

Once a node is reachable, onboarding is one command (token + URL are issued by the
controller, available at `GET /api/onboard-command`):

```bash
curl -sSL https://janus-meter.s3.eu-north-1.amazonaws.com/pillar/latest/install-node.sh \
  | sudo bash -s -- \
      --controller http://34.107.8.212:50051 \
      --token <issued-by-controller> \
      --http-url http://34.107.8.212:8080
```

After that, provisioning a validator (client, version, cluster, paths, flags) is driven
entirely from the web UI's "Setup Validator" form.

---

## Target validator node (status)

The intended validator host for this submission is an EU bare-metal Ubuntu box:

- **Host:** `213.239.141.25` (Monogon SE / ex-Hetzner range, **Germany**)
- **Planned client:** **Agave** (the fully supported client today)

**Current status: blocked on network access.** The host is firewalled — all ports
(22, alt-SSH, HTTP) are filtered from outside its allowlist, so the agent could not be
installed during this submission. The unblock is mechanical:

1. Add the control-plane VM's egress IP (or the operator's IP) to the host's firewall
   allowlist for port 22.
2. Run the node-onboarding command above (it points the agent at
   `34.107.8.212:50051`).
3. Use the UI to provision Agave (the validator binary handling differs by version — see
   the Agave notes in [`../CLAUDE.md`](../CLAUDE.md): v2.x ships `agave-validator` in the
   tarball; v3.x must be pre-installed or built from source).

The control plane is already live and waiting for this node to register.

---

## Multi-client roadmap (TODOs for other clients)

Pillar models the validator client as a `ClientKind` enum
(`agent/src/client/mod.rs`) mapping each client to a systemd service name + binary path,
plus a per-client provision script template (`controller/scripts/provision-*.sh.tmpl`).
The architecture is client-agnostic; the work per client is (a) a tested provision
template, (b) client-specific health/version probing, and (c) end-to-end validation.

| Client | `ClientKind` | Provision template | Health probe | E2E tested | Status |
|---|---|---|---|---|---|
| **Agave** | ✅ | ✅ `provision-agave.sh.tmpl` | ✅ RPC + process | ✅ testnet v3.1.8 | **Production path** |
| **Jito** | ✅ | ✅ source build + cluster-aware MEV | ✅ reuses RPC probe (same surface) | ⏳ build verified | **Provisioning complete** |
| **Firedancer** | ✅ | ✅ source build + `configure init` + TOML | ✅ RPC once running | ⚠️ runtime needs AF_XDP-capable NIC | **Provisioning complete; runtime host-gated** |
| **Frankendancer** | ✅ | ⚠️ shares `fdctl` path, untested | ❌ needs `fdctl`-aware probe | ❌ | TODO |

### Per-client status

**Jito** — feature-complete at the controller/UI layer:
- ✅ Cluster-aware MEV defaults (`templates::jito_defaults_for_cluster`): block-engine URL,
      tip-payment and tip-distribution programs are now selected per cluster. **Fixed a bug
      where the mainnet tip-distribution program was hardcoded (and incorrect:
      `…CfnBtxhxJBjKHHaBnQ4SYnHNDn`) and applied on every cluster** — verified against
      `jito-foundation/jito-programs` `declare_id!` (mainnet
      `4R3gSG8BpU4t19KYj8CfnbtRpnT8gtk4dvTHxVRwc2r7`, testnet
      `DzvGET57TAgEDxvm3ERUM4GNcsAJdqjDLCne9sdfY4wf`).
- ✅ Relayer URL + shred-receiver address plumbed through request → `build_exec_start` →
      UI form (both optional — relayer-less is supported).
- ✅ Block-engine URL defaults to the cluster value when left blank; operator overrides
      win. Tip programs / commission overridable via `validator_flags`.
- ✅ `cluster_defaults` API now returns Jito values so the UI can pre-fill them.
- ✅ Unit tests cover mainnet/testnet program selection, relayer/shred inclusion, and
      override precedence (`controller/src/templates.rs`).
- ✅ `provision-jito.sh.tmpl` now builds `jito-solana` from source when no `download_url`
      is given (Jito Labs publishes no standalone validator binary asset) — mirrors the
      Agave source-build path, with `--recurse-submodules` for the bundled jito-programs.
- [ ] Remaining: full chain sync on a live node (gated by the same inbound-UDP host
      firewall as Agave).

**Firedancer** — provisioning implemented; runtime is host-gated:
- ✅ `provision-firedancer.sh.tmpl` builds `fdctl` from source (clone + submodules +
      `deps.sh` + `make fdctl`), runs `fdctl configure init all`, writes the TOML, and
      installs the systemd unit (`fdctl run --config …`).
- ✅ TOML generation in `build_provision_vars` includes `user`, `[consensus]`, `[ledger]`,
      `[gossip]` entrypoints, and `[rpc]`. Health uses the standard JSON-RPC probe (Firedancer
      serves the same RPC surface once running).
- ⚠️ **Runtime is host-gated**: Firedancer requires an AF_XDP-capable NIC/driver and
      hugepages. On hosts with a bonded/unsupported NIC, `fdctl configure init` / `fdctl run`
      cannot bind — this is a hardware/driver constraint, surfaced clearly by the script.
- [ ] `fdctl`-aware health/version probe (currently reuses the RPC probe).

**Frankendancer** (shares the `fdctl` path):
- [ ] Distinguish from full Firedancer in the lifecycle manager — same `fdctl` tooling,
      different component mix.

### Cross-cutting TODOs (apply to all clients)

- [ ] **Static/musl controller build** so the control plane runs on older distros
      (currently requires GLIBC 2.39 / Ubuntu 24.04+).
- [ ] **Controller artifact storage** — upload/serve/SHA256-verify validator binaries
      from the controller (avoids each node hitting upstream release hosts).
- [ ] **Version fetcher** — GitHub Releases API to populate version dropdowns per client.
- [ ] **Agent self-upgrade** — swap binary, exit, let systemd restart on the new version.
- [ ] **`install-controller.sh` hardening** — NAT detection + Cloudflare Tunnel for
      controllers behind NAT; non-default admin credential on first boot.

---

## Why fund this

- **Open and standard** — Prometheus metrics, Grafana dashboards, gRPC, SQLite. No
  proprietary lock-in, no Solana SDK coupling.
- **Operationally complete for Agave today** — provisioning, health, recovery, upgrades,
  logs, and metrics all work end-to-end on testnet.
- **Small, auditable surface** — two Rust binaries, `cargo clippy -- -D warnings` clean,
  shared proto types, script-based provisioning the operator can read before it runs.
- **Clear path to multi-client** — the client abstraction already exists; the remaining
  work is well-scoped per client (above).
</content>
