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

A working control plane is deployed on a cloud VM for evaluation.

| Component | Endpoint |
|---|---|
| Control-plane web UI | `http://<controller-host>:8080` |
| Controller gRPC (agent endpoint) | `<controller-host>:50051` |
| Grafana (embedded dashboards) | `http://<controller-host>:3000` |
| Prometheus | `localhost:9091` (on the host) |

**Default login:** `admin` / `admin` — **change this before any real use** (UI →
account settings, or the `/api/change-credentials` endpoint).

Deployment details:

- **Host:** a small cloud VM (2 vCPU / 8 GB, Ubuntu 24.04 LTS), in an EU region close to the
  validator host to minimize gRPC/UI latency.
- **Install:** one command, `install-controller.sh`, which also provisions Prometheus +
  Grafana with dashboards out of the box:

  ```bash
  curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-controller.sh \
    | sudo bash -s -- --external-url http://<controller-host>:50051
  ```

> **Note:** the published binaries are built against GLIBC 2.39, so the controller host
> must be Ubuntu 24.04 (Noble) or newer. A 22.04 host fails with
> `version 'GLIBC_2.39' not found`. This is captured as a TODO below (static/musl build).

### Adding a validator node

Once a node is reachable, onboarding is one command (token + URL are issued by the
controller, available at `GET /api/onboard-command`):

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-node.sh \
  | sudo bash -s -- \
      --controller http://<controller-host>:50051 \
      --token <issued-by-controller> \
      --http-url http://<controller-host>:8080
```

After that, provisioning a validator (client, version, cluster, paths, flags) is driven
entirely from the web UI's "Setup Validator" form.

---

## Target validator node (status)

The intended validator host for this submission is an EU bare-metal Ubuntu box:

- **Host:** `<validator-host>` (Monogon SE / ex-Hetzner range, **Germany**)
- **Planned client:** **Agave** (the fully supported client today)

**Current status: blocked on network access.** The host is firewalled — all ports
(22, alt-SSH, HTTP) are filtered from outside its allowlist, so the agent could not be
installed during this submission. The unblock is mechanical:

1. Add the control-plane VM's egress IP (or the operator's IP) to the host's firewall
   allowlist for port 22.
2. Run the node-onboarding command above (it points the agent at
   `<controller-host>:50051`).
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
| **Jito** | ✅ | ✅ source build + cluster-aware MEV | ✅ reuses RPC probe (same surface) | ✅ provisioned + ran via control plane (testnet) | **Working** |
| **Firedancer** | ✅ | ✅ source build + provider-aware `configure` + validated TOML | ✅ RPC once running | ⚙️ provisions + boots all tiles; one tile-init frontier remains | **Boots; near-complete** |
| **Frankendancer** | ✅ | ✅ shares `fdctl` path (same as Firedancer) | ⚙️ via `fdctl` | ⚙️ same as Firedancer | **Boots; near-complete** |

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
- ✅ `provision-jito.sh.tmpl` builds `jito-solana` from source when no `download_url`
      is given (Jito Labs publishes no standalone validator binary asset) — mirrors the
      Agave source-build path, with `--recurse-submodules` for the bundled jito-programs.
- ✅ **Verified live end-to-end**: built `agave-validator 4.1.0-rc.0 (client:JitoLabs)`
      from source, provisioned via the control plane on **testnet**, and confirmed the unit
      carries the correct cluster-aware MEV flags — `--block-engine-url
      https://testnet.block-engine.jito.wtf`, testnet tip-payment
      (`GJHtFqM9agxPmkeKjHny6qiRKrXZALvvFGiKf11QE7hy`) and tip-distribution
      (`DzvGET57TAgEDxvm3ERUM4GNcsAJdqjDLCne9sdfY4wf`) programs, `--commission-bps 800`.
- [ ] Remaining: full chain sync on a live node (gated by the same inbound-UDP host
      firewall as Agave).

**Firedancer / Frankendancer** — provisions and **boots end-to-end** via the control plane;
extensively tested live, down to a single remaining tile-init frontier:
- ✅ `provision-firedancer.sh.tmpl` builds `fdctl` from source (`0.101.0-beta.40101`).
      **Three build-tooling bugs fixed**: don't shallow-clone (submodules → `opt/git/zstd`
      missing), use `deps.sh fetch install` (fetch clones the vendored libs), and put Rust
      in the build env.
- ✅ **Config schema reverse-engineered and validated against `fdctl 1.0`**. The generated
      TOML now uses an explicit per-cluster `expected_genesis_hash` (FD rejects "auto"),
      `[gossip] port_check` (the `--no-port-check` equivalent), `[snapshots] path`,
      `[rpc] port`, and 2 MB huge pages (`[hugetlbfs] max_page_size = "huge"` — gigantic /
      1 GB pages would need GRUB + reboot). Earlier wrong keys (`rpc.full`, bool
      `ledger.limit_size`) removed.
- ✅ **Net provider is selectable** (new `net_provider` field): `socket` (XDP-less
      fallback, works on any NIC incl. bonded — the validated default) or `xdp`. This
      corrects an earlier assumption: the host's Intel E810 (`ice`) NICs *do* support
      AF_XDP and FD's `native_bond` handles bonds, so AF_XDP is not a hard wall.
- ✅ **Runtime prerequisites validated and encoded**: `configure init hugetlbfs sysctl`
      reserves 2 MB pages + creates the `/mnt/.fd` mounts (must run in the host mount
      namespace — the agent's plain-bash executor does, a transient unit does not);
      `fs.nr_open >= 1024000` (added to install-node sysctl tuning); the systemd unit runs
      **as root** so fdctl can set up its PID namespace and drop to the TOML `user` (fixed
      — unlike agave/jito which run as `sol`); `fdctl` added to the `sol` sudoers; configure
      stages are provider-aware (socket → `hugetlbfs sysctl`, xdp → `all`).
- ✅ With all of the above, **Frankendancer boots**: shmem, all tiles (gossip, shred,
      verify, plugin) and the Agave subsystem (snapshot download + replay threads) start; it
      reaches *"Waiting for shred version via gossip."*
- [ ] **Remaining frontier**: on this host a tile exits during init, cascading through the
      diag tile (`/proc/<tid>/stat` race) and fdctl's pidns supervisor (`wait4 unexpected
      pid`). Needs FD-internals debugging specific to this `fdctl` build/host. Full chain
      sync is additionally gated by the same inbound-UDP firewall as Agave.
- [ ] `fdctl`-aware health/version probe (currently reuses the RPC probe); distinguish
      Frankendancer from full Firedancer in the lifecycle manager (shared `fdctl`).

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
