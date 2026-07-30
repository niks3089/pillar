# Releasing

Pillar ships two independently-versioned binaries — `pillar-agent` and
`pillar-controller` — managed by
[release-please](https://github.com/googleapis/release-please) in monorepo mode.
Each has its own version, tag (`pillar-agent-vX.Y.Z` / `pillar-controller-vX.Y.Z`),
and CHANGELOG.

## How a release happens

1. Merge conventional commits to `main`.
2. release-please routes each commit to a component **by the files it changed**:
   a commit touching `agent/**` bumps the agent; `controller/**` bumps the
   controller. It opens a **separate release PR per component**
   (`separate-pull-requests: true`) so each component releases on its own
   cadence — a controller release never forces an agent release. When both
   have pending commits, two release PRs are open at once. They share
   `.release-please-manifest.json`, so merging the first briefly marks the
   second as conflicting — do **not** rebase it by hand. The Release workflow
   re-runs on the merge and release-please force-updates the second PR to
   resolve the manifest automatically; wait for that run (~1-2 min), then
   merge it. Merging only one PR releases only that component.
3. Merging a release PR tags that component and the CI `build`/`publish` jobs
   attach the rebuilt binaries, install scripts, and `manifest.json` to the
   GitHub release. The controller reads `manifest.json` to offer upgrades.

## Rules that keep the two trains from thrashing

- **Never use empty marker commits.** An empty commit (`--allow-empty`) changes
  no files, so release-please cannot route it to a component and attributes it
  to *both* — polluting both CHANGELOGs and bumping both versions. If a PR was
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
