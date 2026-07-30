# Releasing

Pillar ships two independently-versioned binaries — `pillar-agent` and
`pillar-controller` — managed by
[release-please](https://github.com/googleapis/release-please) in monorepo mode.
Each has its own version, tag (`pillar-agent-vX.Y.Z` / `pillar-controller-vX.Y.Z`),
and CHANGELOG.

## How a release happens

1. Merge conventional commits to `main`.
2. Each component runs its **own** release-please with its own config and
   manifest (`release-please-config-<c>.json` + `.release-please-manifest-<c>.json`),
   as two steps in the Release workflow. A commit touching `agent/**` bumps
   only the agent; `controller/**` only the controller. Each opens its own
   release PR against its own manifest file — the two never share state, so
   merging one never conflicts with or bumps the other. No coupling, no
   cross-PR rebase.
3. Merging a release PR tags that component and the CI `build`/`publish` jobs
   attach the rebuilt binaries, install scripts, and `manifest.json` to the
   GitHub release. The controller reads `manifest.json` to offer upgrades.

## Rules that keep the two trains from thrashing

- **Never use empty marker commits.** An empty commit (`--allow-empty`) changes
  no files, so neither component's release-please can route it. If a PR was
  merged with a non-conventional message and needs a release, land a real
  follow-up commit that touches the relevant component's files, not an empty
  marker.
- **Scope commits to one component's path** where practical. A single commit
  that edits both `agent/**` and `controller/**` will produce two release PRs;
  that's fine when the change genuinely spans both, but avoid incidental
  cross-touches.
- **Shared paths** (`scripts/`, top-level docs) belong to no component and bump
  neither on their own — they ride into the next release of whichever component
  is cut, because the install scripts and manifest are rebuilt from `main` on
  every release. If a shared change must ship immediately, pair it with the
  component release it's relevant to.

## Manual release binaries

`install-node.sh`, `install-controller.sh`, and both binaries are attached to
every release and always built from the release commit — so a fix to a shared
script reaches `releases/latest/download/…` with the next release of either
component, even without its own version bump.
