# Security model

Pillar manages validators by design: the controller sends shell scripts to
agents, and agents execute them. Installing pillar on a node is a decision to
trust the controller (and whoever can reach it) with significant control over
that machine. This document states exactly what that means, what access is
granted, and what the known residual risks are — so the decision is an
informed one.

## The access model in one paragraph

The **controller** can execute arbitrary scripts on every connected agent.
The **agent** runs as the unprivileged `sol` user, with root access limited
to an argument-pinned sudoers policy (below). Therefore: compromise of the
controller — or of anything holding its API/session credentials — means
control of every managed node at the level the sudoers policy allows.
Protect the controller like the fleet-wide credential it is: private
network or firewalled API, TLS with client certificates
(`require_client_certs`), strong admin password, no public exposure.

## What `install-node.sh` does to your machine

Run `install-node.sh --dry-run` to see the full plan — including the exact
sudoers policy — before anything is written. Summary:

- Installs `pillar-agent` to `/usr/local/bin`, running as the `sol` user
  (created if missing; no password, no SSH keys).
- Writes `/etc/sudoers.d/sol-pillar` (argument-pinned, `visudo`-validated).
- Installs `/usr/local/bin/pillar-datadir`, a root helper whose only job is
  `mkdir`/`chown`/`rm -rf` of data directories under an allowlist of
  prefixes (`/mnt`, `/data`, `/srv`, `/home/sol`).
- Writes kernel/network tuning (`sysctl`, limits) recommended by Anza.
- Writes `/etc/pillar/agent.yaml` owned by `sol`, mode `600` — it contains
  the controller auth token.
- Installs and enables the `pillar-agent` systemd unit.

## The sudoers policy

The agent may run, as root, **only** exact pinned commands:

| Group | Commands | Why |
|---|---|---|
| Services | `systemctl daemon-reload`; `start/stop/restart/enable --now` of the five known validator units; `restart pillar-agent` | lifecycle management |
| Unit/config writes | `tee` of the five known unit paths, `/etc/pillar/validator.toml`, `/etc/pillar/yellowstone-grpc.json`; `mkdir -p /etc/pillar` | provisioning |
| Binaries | `install -m 755 * <known binary path>` for the five client binaries + `pillar-agent` | installs/upgrades |
| Build deps | the two exact `apt-get` invocations used by source builds | source builds |
| Data dirs | `pillar-datadir ensure|recreate <dirs>` (path-allowlisted helper) | ledger/snapshot/accounts dirs |
| Firedancer | `fdctl`, `timeout 180 fdctl *` | fd requires root operations |

Removed relative to older installs: unrestricted `tee`, `sed`, `cp`, `find`,
`mkdir`, `chown`, `install`, `systemctl`, `apt-get` — any of which was
root-equivalent. Re-run `install-node.sh` on existing nodes to migrate to
the pinned policy.

## Known residual risks (deliberate, documented)

1. **Unit-file writes are root-equivalent.** `tee` to a systemd unit plus
   permission to restart that unit means the agent could set `User=root` and
   run anything. This is the largest remaining hole; the planned fix is the
   "launcher model" (static root-owned units whose `ExecStart` points at a
   `sol`-owned launcher script, so provisioning never touches unit files) —
   tracked in [#37](https://github.com/niks3089/pillar/issues/37).
2. **Firedancer.** `fdctl` legitimately performs privileged operations, and
   the agent may install the `fdctl` binary it then runs as root. If you do
   not run firedancer, you can delete the `PILLAR_FD` alias and the
   `fdctl`-related lines from `/etc/sudoers.d/sol-pillar`.
3. **Script execution is the product.** Even with perfect sudoers, the
   controller can run anything as `sol` — including reading validator
   keypairs owned by `sol`. Identity/vote keys are `sol`-owned by the
   standard validator layout; treat controller access accordingly.

## Binary provenance

The installer downloads `pillar-agent` from GitHub Releases over TLS.
Validator binaries fetched during provisioning support an optional
`sha256` field that is verified when set — set it. Checksums for
`pillar-agent` release artifacts (and installer verification of them) are
planned; until then, pin `--version` rather than `latest` if you need
reproducibility.

## Reporting

Found a vulnerability? Open a GitHub security advisory on this repository
(preferred) or a private report to the maintainer — not a public issue.
