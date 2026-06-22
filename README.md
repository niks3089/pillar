# Pillar

**Open-source operations platform for running Solana validators — at one node or fleet scale.**

Pillar gives validator operators a single control plane to install, monitor, heal, and
upgrade their nodes. It replaces the usual pile of ad-hoc shell scripts, `cron` jobs, and
late-night SSH sessions with two small Rust programs and a clean web UI.

---

## Why Pillar

Running a Solana validator is operationally heavy, and running several is much harder:

- **Validators fail in messy ways** — they crash, fall behind the cluster, or get stuck.
  Recovering from a snapshot by hand is multi-step and easy to get wrong.
- **Observability is fragmented** — operators stitch together journald, Prometheus
  exporters, and custom RPC polling, separately, on every box.
- **Provisioning and upgrades are manual** — standing up a new validator or rolling out a
  new binary is inconsistent and error-prone.
- **None of this scales** — doing it across many nodes, by hand, simply doesn't work.

Pillar turns all of that into one managed system: every validator looks after itself, and
you watch and steer the whole fleet from one place.

---

## What it does

- **Keeps validators healthy** — continuously checks each validator, detects crashes and
  crash-loops, and restarts or recovers automatically.
- **Snapshot recovery** — downloads and restores from a snapshot when a node needs it,
  with live progress you can watch.
- **One pane of glass** — a web dashboard shows every node's status, version, sync state,
  and resource usage in real time.
- **Live logs** — stream validator logs straight to the UI, filterable by level and text.
- **Provision & upgrade from the UI** — install a validator or push a binary upgrade to
  any node without SSHing in.
- **Metrics & alerting** — Prometheus metrics and Grafana dashboards out of the box, with
  ready-made alert rules (offline, lagging, restart-looping, disk-full).
- **Multi-client** — Agave today, with Jito, Firedancer/Frankendancer, and a Surfpool test
  validator on the roadmap.

---

## How it works

Pillar has two pieces:

- **Agent** — a single binary that runs on each node. It supervises the validator process,
  reads its health, collects metrics, tails its logs, and reports everything back.
- **Controller** — a central management plane with a web UI. One controller manages many
  nodes: it receives status from every agent, stores history, serves the dashboard and
  metrics, and pushes commands (provision, upgrade, restart, recover, stop) back out.

Agents connect outward to the controller over gRPC, so nodes never need inbound management
ports. For the full design — state machine, data flow, and protocol — see
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

---

## Quick start

> The controller binary needs GLIBC 2.39 — install it on **Ubuntu 24.04 (Noble) or newer**.

**1. Install the controller** (sets up the controller, Prometheus, and Grafana with
dashboards provisioned automatically):

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-controller.sh \
  | sudo bash -s -- --external-url https://<controller-host>:50051
```

The default login is `admin` / `admin` — **change it before any real use.**

**2. Add a node.** Open the controller UI, copy the onboarding command it generates (it
includes the controller URL and a token), and run it on the validator host:

```bash
curl -sSL https://github.com/niks3089/pillar/releases/latest/download/install-node.sh \
  | sudo bash -s -- --controller https://<controller-host>:50051 --token <token> --http-url http://<controller-host>:8080
```

Then provision a validator from the node's detail page in the UI. The day-to-day operator
guide lives in [`docs/OPERATIONS.md`](docs/OPERATIONS.md).

---

## Documentation

- [`docs/OPERATIONS.md`](docs/OPERATIONS.md) — operator runbook: onboarding, provisioning,
  upgrades, alerting, security, backup & recovery, troubleshooting.
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — system design and data flow.
- [`docs/GRANT.md`](docs/GRANT.md) — project overview and roadmap.

---

## Build from source

```bash
cargo build --release          # build agent + controller
cargo test                     # run tests
cargo clippy -- -D warnings    # lint
```

Configuration is via YAML (`agent.yaml`, `controller-config.yaml`) or the
`PILLAR_AGENT_CONFIG` / `PILLAR_CONTROLLER_CONFIG` environment variables.

---

## Roadmap

- Additional validator clients: Jito, Firedancer / Frankendancer, Surfpool
- Controller artifact storage with SHA-256 verification
- GitHub Releases version discovery for upgrades
- Agent self-upgrade

---

## Contributing

Issues and pull requests are welcome — see [`CONTRIBUTING.md`](CONTRIBUTING.md). Please run
`cargo clippy -- -D warnings` and `cargo test` before opening a PR.

## License

Open source under the [Apache License 2.0](LICENSE).

---

Built by Nikhil Acharya.
