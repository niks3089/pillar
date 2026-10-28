# Contributing to Pillar

Thanks for your interest in improving Pillar! Issues and pull requests are welcome.

## Getting started

```bash
cargo build --release          # build the agent + controller
cargo test                     # run the test suite
cargo clippy -- -D warnings    # lint (must be clean)
```

The web UI lives in `controller/web` (React + Vite):

```bash
cd controller/web
npm install
npm run build                  # produces dist/, embedded into the controller binary
```

## End-to-end tests (Playwright)

Both suites boot a throwaway controller (fresh DB, known admin password) and
drive the real UI in Chromium. Specs live in `controller/web/playwright/tests`.

```bash
cd controller/web
npx playwright install chromium   # once
npm run test:e2e                  # ui: login, docs, lifecycle controls (~20s)
npm run test:e2e:system           # system: real node, ~10 min (see below)
```

The **system** suite starts a privileged systemd container as the validator host,
runs `scripts/install-node.sh` with the agent built from your checkout, provisions
Surfpool through the UI, and checks Stop/Start/hold behaviour against `systemctl`
inside the node, using the agent's shipped timings. It needs Docker. Useful env:

| Variable | Purpose |
|---|---|
| `PILLAR_E2E_DOCKER="sudo docker"` | when your Docker socket is root-only |
| `PILLAR_E2E_SKIP_BUILD=1` | reuse existing `target/debug` builds and `dist/` |
| `PILLAR_E2E_SURFPOOL_VERSION=1.7.0` | pin instead of looking up the latest release |
| `PILLAR_E2E_AGENT_BINARY=/path/agent` | run against another agent build (e.g. to confirm a test catches a regression) |
| `PILLAR_E2E_KEEP_NODE=1` | leave the node container running for debugging |

CI runs `cargo test` and both Playwright suites on every pull request.

## Before opening a pull request

- Run `cargo test` and `cargo clippy -- -D warnings` — both must pass cleanly.
- Run `npm run test:e2e` in `controller/web`; run `npm run test:e2e:system` when
  touching the agent, provisioning scripts or lifecycle actions.
- Keep changes focused; match the style and conventions of the surrounding code.
- Update relevant docs (`README.md`, `docs/`) when behavior or interfaces change.
- Write a clear description of **what** changed and **why**.

## Reporting issues

When filing a bug, please include:

- What you expected to happen and what actually happened.
- Steps to reproduce, and the validator client / cluster involved.
- Relevant logs (controller and/or agent), with any secrets redacted.

## License

By contributing, you agree that your contributions will be licensed under the
[Apache License 2.0](LICENSE).
