# Why Pillar is Client-Agnostic

## The Problem

Running a Solana validator fleet used to mean picking a single validator client and building all your tooling around its idiosyncrasies. Agave uses CLI flags. Firedancer uses a TOML file. Jito adds MEV flags on top of Agave. Frankendancer needs kernel-bypass initialization before it can even start. Each client has a different binary name, a different systemd service name, different configuration mechanics, and different monitoring surfaces.

For a single node, this is manageable. For a fleet -- especially one where you want to diversify across clients for resilience, or migrate from one client to another -- it becomes an operational burden. Every client change means rewriting systemd units, updating monitoring, and re-learning a different configuration model.

## Why Client-Agnostic Makes Sense

Pillar treats the validator client as a swappable component behind a uniform interface. This works because, despite surface differences, every Solana validator client does the same fundamental things:

1. **Runs as a systemd service** on a Linux machine
2. **Consumes the same inputs**: identity keypair, vote account, ledger/accounts/snapshot paths, entrypoints, known validators
3. **Speaks the same protocol**: Solana gossip, turbine, repair, JSON-RPC
4. **Produces the same observable outputs**: a local RPC endpoint that responds to `getSlot`, `getHealth`, `getVoteAccounts`

Pillar exploits this uniformity. The agent does not need to understand the internals of Agave or Firedancer -- it only needs to know how to install, start, stop, health-check, and upgrade a systemd service that exposes a Solana JSON-RPC port.

### What Stays the Same Across All Clients

| Concern | Agave | Jito | Firedancer | Frankendancer |
|---------|-------|------|------------|---------------|
| Process model | systemd service | systemd service | systemd service | systemd service |
| Health check | `getSlot` + `getVoteAccounts` via JSON-RPC | Same | Same | Same |
| State machine | Off -> StartingUp -> Behind -> Healthy | Same | Same | Same |
| Metrics collection | sysinfo (CPU, mem, disk, net, process) | Same | Same | Same |
| Log collection | journald tail | Same | Same | Same |
| Lifecycle ops | `systemctl stop/start/restart` | Same | Same | Same |
| Snapshot recovery | stop -> wipe ledger -> download -> start | Same | Same | Same |

The reconciliation loop, health checking, metrics enrichment, log collection, and crash-loop detection are entirely client-agnostic. They depend only on the systemd service name and the RPC endpoint -- not on how the validator was configured or which binary is running.

### What Differs (and How Pillar Handles It)

| Concern | Agave / Jito | Firedancer / Frankendancer | How Pillar Abstracts It |
|---------|-------------|---------------------------|------------------------|
| Config format | CLI flags in ExecStart | TOML file | `ClientInstaller.exec_start()` branches by `ClientKind` |
| Binary name | `agave-validator` / `jito-validator` | `fdctl` | `ClientInstaller.for_client()` maps kind to path |
| Service name | `solana-validator` / `jito-validator` | `firedancer` / `frankendancer` | `ClientInstaller.service_name` |
| MEV config | `--block-engine-url` + tip flags | `[tiles.bundle]` TOML section | Provisioner adds Jito flags or writes TOML section |
| Geyser plugins | `--geyser-plugin-config` flag | Not yet supported in TOML | Provisioner adds flag for Agave/Jito only |
| System init | sysctl + ulimits | sysctl + hugetlbfs + ethtool + XDP | Not yet automated by Pillar (manual or install script) |

## How It Works in Code

### The Abstraction Layers

Pillar's client abstraction is intentionally thin. It consists of two enums and one struct:

**`ClientKind`** -- the enum that travels through the entire system, from the controller UI dropdown to the agent provisioner:

```
Agave | Jito | Firedancer | Frankendancer
```

**`ClientInstaller`** -- maps a `ClientKind` to the two things Pillar needs to manage the service:

| Client | service_name | binary_path |
|--------|-------------|-------------|
| Agave | `solana-validator` | `/usr/local/bin/agave-validator` |
| Jito | `jito-validator` | `/usr/local/bin/jito-validator` |
| Firedancer | `firedancer` | `/usr/local/bin/fdctl` |
| Frankendancer | `frankendancer` | `/usr/local/bin/fdctl` |

**`ProvisionConfig`** -- a unified config struct parsed from the gRPC `ProvisionCommand`. It holds every field for every client. Fields that don't apply to a particular client are simply ignored during ExecStart generation.

### The Provisioning Flow

Regardless of client, provisioning follows the same sequence:

```
1. Parse ProvisionCommand -> ProvisionConfig
2. Stop existing service (if running)
3. Download binary from URL, verify SHA256
4. Install binary to client-specific path
5. Write client configs:
   - Agave/Jito: nothing (config is in CLI flags)
   - Firedancer/Frankendancer: write /etc/pillar/validator.toml
   - If yellowstone_grpc: write /etc/pillar/yellowstone-grpc.json
6. Generate systemd unit (ExecStart varies by client)
7. systemctl daemon-reload
8. systemctl enable --now <service_name>
9. Update agent config (client, cluster, service_name)
```

The branching happens in exactly two places:

1. **`exec_start()`** -- Agave/Jito get a long CLI command with all flags; Firedancer/Frankendancer get `fdctl run --config /etc/pillar/validator.toml`
2. **`write_client_configs()`** -- Firedancer/Frankendancer get a TOML file written; Agave/Jito don't need one

Everything else is identical.

### Health Checking is Universal

The health checker does not know or care which client is running. It uses raw JSON-RPC calls against `http://localhost:8899`:

- `getSlot` -- is the validator's local slot progressing?
- `getHealth` -- does the client consider itself healthy?
- `getVoteAccounts` -- is the node actively voting? (validators only)

These RPC methods are part of the Solana protocol and are implemented identically by every client. This is what makes client-agnostic health monitoring possible.

### Lifecycle Management is Universal

All lifecycle operations go through systemd:

```
systemctl stop <service_name>
systemctl start <service_name>
systemctl restart <service_name>
systemctl is-active <service_name>
```

The `service_name` is the only client-specific value. The `SystemdManager` in the agent takes it as a constructor argument and uses it for every operation.

## How Upgrades Work

### Same-Client Upgrade (Version Bump)

The most common upgrade: updating from one version to another of the same client (e.g., Agave v2.1.5 to v2.1.6).

```
Controller                              Agent
    |                                     |
    |  POST /api/nodes/:id/upgrade        |
    |  { binary_name: "agave-validator",  |
    |    version: "2.1.6",                |
    |    download_url: "https://...",     |
    |    sha256: "abc123" }               |
    |                                     |
    |  --UpgradeCommand via gRPC-------->  |
    |                                     |
    |                          1. Download binary to /tmp/pillar-staging/
    |                          2. Verify SHA256
    |                          3. systemctl stop solana-validator
    |                          4. sudo install binary -> /usr/local/bin/agave-validator
    |                          5. systemctl restart solana-validator
    |                                     |
    |  <--ReportUpgradeStatus (success)--  |
```

This is a binary-only swap. The systemd unit, config files, and all flags remain unchanged. The process takes seconds of downtime -- stop, replace binary, restart.

The upgrade command maps binary names to service names:

| binary_name | service_name |
|-------------|-------------|
| `agave-validator` | `solana-validator` |
| `jito-validator` | `jito-validator` |
| `fdctl` | `firedancer` |

### Cross-Client Migration (e.g., Agave to Frankendancer)

Switching validator clients is a full re-provision, not an upgrade. This is intentional -- the configuration format, binary, service name, and possibly system initialization all change.

```
Controller                              Agent
    |                                     |
    |  POST /api/nodes/:id/provision      |
    |  { client: "frankendancer",         |
    |    version: "0.811.30108",          |
    |    cluster: "mainnet-beta",         |
    |    download_url: "https://...",     |
    |    sha256: "def456",               |
    |    identity_keypair_path: "...",    |
    |    ... }                            |
    |                                     |
    |  --ProvisionCommand via gRPC------>  |
    |                                     |
    |                          1. Parse ProvisionCommand
    |                          2. systemctl stop solana-validator (old service)
    |                          3. Download fdctl binary, verify SHA256
    |                          4. Install to /usr/local/bin/fdctl
    |                          5. Write /etc/pillar/validator.toml
    |                          6. Write frankendancer.service systemd unit
    |                          7. systemctl daemon-reload
    |                          8. systemctl enable --now frankendancer
    |                          9. Update agent.yaml: client=frankendancer, service=frankendancer
    |                                     |
    |  Agent exits for config reload       |
    |  systemd restarts agent             |
    |  Agent reconnects with new client   |
    |                                     |
    |  <--NodeStatus (state: starting_up) |
```

Key points about cross-client migration:

1. **The old service is stopped first.** Pillar stops whatever service is currently running before installing the new one.
2. **The ledger is preserved.** The Agave and Frankendancer blockstores are compatible. The ledger directory (`/mnt/ledger`) is reused without modification.
3. **Config format changes transparently.** Agave used CLI flags in the systemd unit; Frankendancer uses a TOML file. Pillar generates the correct format for the new client.
4. **The agent updates itself.** After provisioning, the agent writes its own config to reflect the new client and service name, then exits so systemd restarts it with the updated config.
5. **System initialization may be needed.** Switching to Firedancer/Frankendancer requires hugetlbfs, sysctl, and ethtool configuration that Agave doesn't need. This is currently a manual step.

### Migration Compatibility Matrix

| From / To | Agave | Jito | Firedancer | Frankendancer |
|-----------|-------|------|------------|---------------|
| **Agave** | Upgrade | Re-provision | Re-provision | Re-provision |
| **Jito** | Re-provision | Upgrade | Re-provision | Re-provision |
| **Firedancer** | Re-provision | Re-provision | Upgrade | Re-provision |
| **Frankendancer** | Re-provision | Re-provision | Re-provision | Upgrade |

**Upgrade** = binary swap only (same client, new version). Fast, seconds of downtime.

**Re-provision** = full provision sequence (new client). Generates new config, new systemd unit, new service. The validator restarts from its existing ledger and catches up.

### Fleet-Wide Rolling Upgrades

For upgrading an entire fleet, the controller supports bulk operations:

1. Upload the new binary as an artifact (`POST /api/artifacts`)
2. Select target nodes in the UI
3. Trigger upgrade (one-by-one or parallel)
4. Each node: download artifact from controller -> stop -> swap -> restart
5. Controller tracks progress via `ReportUpgradeStatus` gRPC

The upgrade is client-aware at the binary level but client-agnostic at the orchestration level -- the same flow works whether the fleet runs Agave, Jito, Frankendancer, or a mix.

### Agent Self-Upgrade

The Pillar agent can upgrade itself:

1. Controller sends `UpgradeCommand` with `binary_name: "pillar-agent"`
2. Agent downloads the new `pillar-agent` binary, verifies SHA256
3. Agent replaces its own binary at `/usr/local/bin/pillar-agent`
4. Agent exits cleanly
5. systemd `Restart=on-failure` restarts the agent with the new binary

This works because the agent is a stateless process -- all state is in the shared `NodeStatus` proto, the config file, and the controller's database.

## What This Enables

### 1. Client Diversity Without Operational Overhead

A fleet can run Agave on some nodes and Frankendancer on others, managed from the same controller UI. Health checks, metrics, alerts, and log streaming work identically regardless of client.

### 2. Zero-Downtime Client Migration

Switching a node from Agave to Frankendancer is a single `POST /api/nodes/:id/provision` call. The operator fills out the same form in the UI -- they just change the "Client" dropdown from Agave to Frankendancer.

### 3. A/B Testing New Clients

Operators can provision a few nodes with a new client (e.g., Frankendancer) while the rest of the fleet stays on Agave. The controller shows all nodes side-by-side with the same metrics, making it easy to compare performance.

### 4. Rapid Rollback

If a new client causes issues, re-provisioning back to the previous client is the same one-click operation. The ledger is compatible across clients, so no data is lost.

## Current Limitations

1. **Firedancer/Frankendancer TOML generation is minimal.** Pillar generates only layout, consensus, ledger, and gossip sections. Advanced settings (tile counts, XDP mode, RPC config, snapshot paths, bundle config) require manual editing of `/etc/pillar/validator.toml`.

2. **System initialization is not automated.** Firedancer/Frankendancer need `fdctl configure init all` (hugetlbfs, sysctl, ethtool) before they can run. Pillar does not run this during provisioning. The install script or the operator must handle it.

3. **Upgrade maps `fdctl` to `firedancer` only.** The upgrade command's binary-to-service mapping hardcodes `fdctl -> firedancer`. Nodes running the `frankendancer` service may need special handling.

4. **No version fetching from GitHub.** The UI requires manually entering the version and download URL. A future version fetcher could query GitHub Releases for each client to populate a dropdown.

5. **Geyser plugins (Yellowstone gRPC) are not wired for Firedancer/Frankendancer.** The `--geyser-plugin-config` flag is only added for Agave/Jito ExecStart lines. Firedancer has a different plugin model that is not yet supported.

## References

- [Agave Client Documentation](./Agave.md)
- [Jito Client Documentation](./Jito.md)
- [Firedancer Client Documentation](./Firedancer.md)
- [Frankendancer Client Documentation](./Frankendancer.md)
- Agent provisioner: `agent/src/provisioner.rs`
- Agent client abstraction: `agent/src/client/mod.rs`
- Agent health checker: `agent/src/health/mod.rs`
- Agent reconciler: `agent/src/reconcile.rs`
