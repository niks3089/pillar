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

## Before opening a pull request

- Run `cargo test` and `cargo clippy -- -D warnings` — both must pass cleanly.
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
