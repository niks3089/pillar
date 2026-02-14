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

Pillar exploits this uniformity. The agent does not need to understand the internals of Agave or Firedancer -- it only needs to know how to execute scripts the controller sends and health-check the resulting systemd service via JSON-RPC.

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
| Config format | CLI flags in ExecStart | TOML file | Controller renders client-specific script template |
| Binary name | `agave-validator` / `jito-validator` | `fdctl` | Controller maps client to binary path in template vars |
| Service name | `solana-validator` / `jito-validator` | `firedancer` / `frankendancer` | Controller maps client to service name in template vars |
| MEV config | `--block-engine-url` + tip flags | `[tiles.bundle]` TOML section | Controller builds ExecStart with Jito flags or writes TOML section |
| Geyser plugins | `--geyser-plugin-config` flag | Not yet supported in TOML | Controller adds flag for Agave/Jito only |
| System init | sysctl + ulimits | sysctl + hugetlbfs + ethtool + XDP | Not yet automated by Pillar (manual or install script) |

## Architecture: Script-Based Execution

### The Key Insight

All client-specific knowledge lives on the **controller**, not the agent. The controller renders bash script templates with node-specific parameters and sends them to the agent as `ExecuteScript` commands. The agent is a thin executor -- it runs whatever bash the controller sends and reports the result.

This means:
- **Adding support for new validator client flags or config changes requires only updating the controller's script templates** -- no agent rebuild, no redeployment to every node.
- **The agent binary is generic and stable** -- it handles health checks, metrics, log collection, and script execution. It never needs to know about Agave CLI flags, Firedancer TOML, or Jito tip addresses.
- **Scripts are auditable** -- operators can inspect exactly what the controller will run before triggering a provision/upgrade.

### How It Works

```
Controller                                Agent
    |                                       |
    | User clicks "Setup Validator"         |
    | (client=agave, version=2.1.6, ...)    |
    |                                       |
    | 1. Select template: provision-agave   |
    | 2. Build vars: exec_start, sha256,    |
    |    service_name, paths, etc.          |
    | 3. Render template -> bash script     |
    |                                       |
    | --ExecuteScript via gRPC----------->  |
    |   { script_id, script, description,   |
    |     timeout_secs }                    |
    |                                       |
    |                        4. Write script to /tmp/
    |                        5. sudo bash /tmp/script.sh
    |                        6. Stream stdout/stderr to journald
    |                        7. Wait for exit (with timeout)
    |                                       |
    | <--ScriptResult via gRPC-----------   |
    |   { exit_code, stdout, stderr,        |
    |     timed_out }                       |
    |                                       |
    | 8. Update lifecycle state in DB       |
    | 9. Log result to node's log stream    |
```

### The Agent's Role

The agent's `ScriptExecutor` is deliberately simple (~130 lines):

1. Writes the script to `/tmp/pillar-scripts/<script_id>.sh`
2. Spawns `sudo bash /tmp/pillar-scripts/<script_id>.sh` with piped stdout/stderr
3. Logs each line of output via `tracing::info!` (appears in journald, streams to controller via log collector)
4. Waits with a configurable timeout (default 3600s)
5. On timeout: kills the process group, sets `timed_out=true`
6. Returns `ScriptResult` with exit_code, stdout, stderr
7. Cleans up the temp script file

The reconciler receives `ExecuteScript` commands via the same mpsc channel used for all controller commands, and sends `ScriptResult` back via a dedicated channel to the gRPC task.

### The Controller's Role

The controller owns all client-specific knowledge in two places:

1. **`templates.rs`** -- helper functions like `build_exec_start()`, `service_name_for_client()`, `binary_path_for_client()` that map client names to concrete values
2. **`scripts/`** -- bash script templates with `{{placeholder}}` variables

Each API handler (restart, recover, provision, upgrade, stop) renders the appropriate template with node-specific variables and wraps it in an `ExecuteScript` proto message.

### Script Templates

Templates use a simple `{{placeholder}}` syntax. All conditional logic (Jito MEV flags, Firedancer TOML, Yellowstone config) is pre-computed by the API handler and injected as fully-rendered variables -- templates contain no if/else.

| Template | Purpose | Key Variables |
|----------|---------|---------------|
| `provision-agave.sh.tmpl` | Download binary, verify SHA256, write systemd unit, start service, update agent config | `version`, `download_url`, `sha256`, `exec_start`, `service_name` |
| `provision-jito.sh.tmpl` | Same as agave, exec_start includes Jito MEV flags | Same + Jito-specific flags in `exec_start` |
| `provision-firedancer.sh.tmpl` | Download binary, write validator.toml, write systemd unit | Same + `firedancer_toml` |
| `provision-frankendancer.sh.tmpl` | Same structure as firedancer | Same as firedancer |
| `upgrade-validator.sh.tmpl` | Download, verify, stop, install, restart | `binary_name`, `version`, `download_url`, `sha256`, `service_name` |
| `upgrade-agent.sh.tmpl` | Download, verify, install, restart agent | `version`, `download_url`, `sha256` |
| `recover.sh.tmpl` | Stop, wipe ledger, start | `service_name`, `ledger_path` |
| `restart.sh.tmpl` | `systemctl restart` | `service_name` |
| `stop.sh.tmpl` | `systemctl stop` | `service_name` |

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
    |  Render upgrade-validator.sh.tmpl   |
    |  --ExecuteScript via gRPC-------->  |
    |                                     |
    |                          Script runs:
    |                          1. Download binary to /tmp/pillar-staging/
    |                          2. Verify SHA256
    |                          3. systemctl stop solana-validator
    |                          4. sudo install binary -> /usr/local/bin/agave-validator
    |                          5. systemctl restart solana-validator
    |                                     |
    |  <--ScriptResult (exit_code=0)----  |
```

This is a binary-only swap. The systemd unit, config files, and all flags remain unchanged. The process takes seconds of downtime -- stop, replace binary, restart.

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
    |  Render provision-frankendancer.sh  |
    |  --ExecuteScript via gRPC-------->  |
    |                                     |
    |                          Script runs:
    |                          1. Download fdctl binary, verify SHA256
    |                          2. systemctl stop solana-validator (old service)
    |                          3. Install to /usr/local/bin/fdctl
    |                          4. Write /etc/pillar/validator.toml
    |                          5. Write frankendancer.service systemd unit
    |                          6. systemctl daemon-reload
    |                          7. systemctl enable --now frankendancer
    |                          8. Update agent.yaml via sed
    |                          9. systemctl restart pillar-agent
    |                                     |
    |  Agent restarts with new config     |
    |  Agent reconnects with new client   |
    |                                     |
    |  <--NodeStatus (state: starting_up) |
```

Key points about cross-client migration:

1. **The old service is stopped first.** The script stops whatever service is currently running before installing the new one.
2. **The ledger is preserved.** The Agave and Frankendancer blockstores are compatible. The ledger directory (`/mnt/ledger`) is reused without modification.
3. **Config format changes transparently.** Agave used CLI flags in the systemd unit; Frankendancer uses a TOML file. The controller selects the appropriate template and renders the correct format.
4. **The agent updates itself.** The script updates the agent config via sed and restarts the agent, which reconnects with the updated client/cluster settings.
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
4. Each node: controller renders upgrade script -> agent executes -> reports result
5. Controller tracks progress via `ScriptResult` + `script_executions` table

The upgrade is client-aware at the template level but client-agnostic at the orchestration level -- the same flow works whether the fleet runs Agave, Jito, Frankendancer, or a mix.

### Agent Self-Upgrade

The Pillar agent can upgrade itself:

1. Controller renders `upgrade-agent.sh.tmpl` with download URL and SHA256
2. Controller sends `ExecuteScript` to agent
3. Script downloads the new binary, verifies SHA256, installs to `/usr/local/bin/pillar-agent`
4. Script restarts the agent via `systemctl restart pillar-agent`
5. systemd restarts the agent with the new binary

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

### 5. Update Without Agent Redeployment

When a validator client adds new flags or changes config format, only the controller's script templates need to be updated. No agent rebuild, no fleet-wide binary deployment.

## Current Limitations

1. **Firedancer/Frankendancer TOML generation is minimal.** The controller generates only layout, consensus, ledger, and gossip sections. Advanced settings (tile counts, XDP mode, RPC config, snapshot paths, bundle config) require manual editing of `/etc/pillar/validator.toml` or adding them to the script template.

2. **System initialization is not automated.** Firedancer/Frankendancer need `fdctl configure init all` (hugetlbfs, sysctl, ethtool) before they can run. Pillar does not run this during provisioning. The install script or the operator must handle it.

3. **No version fetching from GitHub.** The UI requires manually entering the version and download URL. A future version fetcher could query GitHub Releases for each client to populate a dropdown.

4. **Geyser plugins (Yellowstone gRPC) are not wired for Firedancer/Frankendancer.** The `--geyser-plugin-config` flag is only added for Agave/Jito ExecStart lines. Firedancer has a different plugin model that is not yet supported.

## References

- [Agave Client Documentation](./Agave.md)
- [Jito Client Documentation](./Jito.md)
- [Firedancer Client Documentation](./Firedancer.md)
- [Frankendancer Client Documentation](./Frankendancer.md)
- Controller templates: `controller/src/templates.rs`
- Controller script templates: `controller/scripts/`
- Agent script executor: `agent/src/script_executor.rs`
- Agent client abstraction: `agent/src/client/mod.rs`
- Agent health checker: `agent/src/health/mod.rs`
- Agent reconciler: `agent/src/reconcile.rs`
