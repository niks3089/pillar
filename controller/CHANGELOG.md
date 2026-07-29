# Changelog

## [0.13.3](https://github.com/niks3089/pillar/compare/pillar-controller-v0.13.2...pillar-controller-v0.13.3) (2026-07-29)


### Bug Fixes

* **controller:** clear provisioning state when the script result arrives ([de6d556](https://github.com/niks3089/pillar/commit/de6d556d820ff30ecd5afe2599a35a254074f8ca))

## [0.13.2](https://github.com/niks3089/pillar/compare/pillar-controller-v0.13.1...pillar-controller-v0.13.2) (2026-07-29)


### Bug Fixes

* **controller:** agave 4.2 capability grant + copyable IP ([9b737df](https://github.com/niks3089/pillar/commit/9b737df1060949bcfeaa5ecb200adc49211921d2))
* **controller:** grant CAP_NET_RAW/CAP_NET_ADMIN in agave/jito units, copyable IP ([5bd9310](https://github.com/niks3089/pillar/commit/5bd93104ae62fed0e129836cd0f3ee7610e72bb1))

## [0.13.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.13.0...pillar-controller-v0.13.1) (2026-07-29)


### Bug Fixes

* **controller:** install surfpool from versioned release tarball ([3e911ac](https://github.com/niks3089/pillar/commit/3e911acdb37dfb3fa15c24d21d4e2f9b25f0cdce))
* **controller:** install surfpool from versioned release tarball ([aa26a66](https://github.com/niks3089/pillar/commit/aa26a66730013f112cbfff2d6954aa68f8033ef0))

## [0.13.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.12.1...pillar-controller-v0.13.0) (2026-07-29)


### Features

* add Mithril as a fourth validator client ([0d43e67](https://github.com/niks3089/pillar/commit/0d43e67ef42851423f55ec84017304a6ac917433))


### Bug Fixes

* bring mithril up to current provision architecture ([53c7b61](https://github.com/niks3089/pillar/commit/53c7b61f8344dd8b55ea45ee944a69ffd843a988))
* **controller:** agent-restarting scripts must not hold stdout open past exit ([748fa3c](https://github.com/niks3089/pillar/commit/748fa3c97e8c31d742401a5fa00aaa8bb4c262a9))
* **controller:** clarify check-for-updates toast — 'up to date' meant controller only ([2bf43de](https://github.com/niks3089/pillar/commit/2bf43decdc2a8f00a99a8d091aeb358c44990745))
* **controller:** failure banner status icon was identical to dismiss button ([0ba923a](https://github.com/niks3089/pillar/commit/0ba923a900371a6c429691155ebbb00eab99b85f))
* **controller:** re-land stranded [#65](https://github.com/niks3089/pillar/issues/65) fixes — script-result race, surfpool reinstall, upgrade tracking ([a646d9d](https://github.com/niks3089/pillar/commit/a646d9d7b5ea3065f06ac9a76f7755b82bd84f96))
* **controller:** surfpool provision skipped install when any version present ([5adb275](https://github.com/niks3089/pillar/commit/5adb27547ceceee0eb27dc040be00bfa30abec1a))
* **controller:** track agent-upgrade scripts, fail release manifest step on bad download ([341c8b3](https://github.com/niks3089/pillar/commit/341c8b3b4e8390e59d19cbb7363f47c06e49475e))

## [0.12.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.12.0...pillar-controller-v0.12.1) (2026-07-29)


### Bug Fixes

* **controller:** include prereleases in version suggestions, show live version hint ([2ad64bb](https://github.com/niks3089/pillar/commit/2ad64bbd51fe84194d1e482687d4a79c3a8f8867))
* **controller:** real version dropdown, surfpool repo moved, drop button ellipsis ([018a4fb](https://github.com/niks3089/pillar/commit/018a4fbcca9b97df90945c8a804fe2c8635be57f))
* **controller:** version dropdown lists all releases ([5d64b04](https://github.com/niks3089/pillar/commit/5d64b04cb09d893f0df8a8b9e6cf1d0c760e39ae))

## [0.12.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.11.0...pillar-controller-v0.12.0) (2026-07-29)


### Features

* config UX — version suggestions, dismissible outcome, surfpool version fix ([ba73593](https://github.com/niks3089/pillar/commit/ba7359341060f44e99378c54e0a5c5bd6d1577ef))
* **controller:** check-for-updates button in nav ([ef65cf2](https://github.com/niks3089/pillar/commit/ef65cf26dc6ec32fc6af716af8d4d3138cff1622))
* **controller:** version suggestions, dismissible outcome banner, config UX ([5bbee60](https://github.com/niks3089/pillar/commit/5bbee60f80022ade18815b02fe0babc4f138e658))

## [0.11.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.10.1...pillar-controller-v0.11.0) (2026-07-29)


### Features

* **controller:** show real validator uptime in node detail ([e57d6f6](https://github.com/niks3089/pillar/commit/e57d6f6f4d60824e4fd6a0001e7d631717b6d877))
* real validator process uptime ([5a9441e](https://github.com/niks3089/pillar/commit/5a9441e3ec2e442a50c2e71cf7eb50ec24e6190d))

## [0.10.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.10.0...pillar-controller-v0.10.1) (2026-07-29)


### Bug Fixes

* **controller:** provision data-dirs fallback used sudo chown, not in blanket sudoers ([b7538d3](https://github.com/niks3089/pillar/commit/b7538d3bec715105eeae81aeca80d4d7265541e5))
* **controller:** provision sudo fallback + surface last deployment outcome ([a287fae](https://github.com/niks3089/pillar/commit/a287fae8297d2b98377d37ce21997d9a0dbbb4df))

## [0.10.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.9.0...pillar-controller-v0.10.0) (2026-07-28)


### Features

* **controller:** show running version in nav, 5m update-check TTL ([cdfc94a](https://github.com/niks3089/pillar/commit/cdfc94a36bde4d81f2c40e61edfa97f1a241c376))
* **controller:** show running version in nav, 5m update-check TTL ([fd64f34](https://github.com/niks3089/pillar/commit/fd64f34f7a9023cf0d5db914c455f63e3d1788ae))

## [0.9.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.8.0...pillar-controller-v0.9.0) (2026-07-28)


### Features

* **controller:** deployment progress UI + reliable in-progress guard ([743e901](https://github.com/niks3089/pillar/commit/743e90133609ffa199c217f3931996153a27dd85))
* **controller:** deployment progress UI + reliable in-progress guard ([2fb8bfe](https://github.com/niks3089/pillar/commit/2fb8bfec97074e0dc7ee7f2e6eed1c2d4676a7bf))

## [0.8.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.7.0...pillar-controller-v0.8.0) (2026-07-24)


### Features

* sudo-free controller self-upgrade, always target latest release ([12ce883](https://github.com/niks3089/pillar/commit/12ce883bb20b754d7b4fe748facf8c40f230a41e))
* sudo-free controller self-upgrade, always target latest release ([6cf62a2](https://github.com/niks3089/pillar/commit/6cf62a21496536c30ac3ff669c7aeae9c16f77c7))

## [0.7.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.6.1...pillar-controller-v0.7.0) (2026-07-24)


### Features

* auto-inject SOLANA_METRICS_CONFIG for agave/jito provisions ([aa47769](https://github.com/niks3089/pillar/commit/aa47769264f64e4cbea102ed0a9ed397cfa07469))
* auto-inject SOLANA_METRICS_CONFIG for agave/jito provisions ([68db7b9](https://github.com/niks3089/pillar/commit/68db7b9b48302bbf0237ad9aa6f434b18894d4b1))

## [0.6.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.6.0...pillar-controller-v0.6.1) (2026-07-21)


### Bug Fixes

* **controller:** include --cluster placeholder in onboard command ([#42](https://github.com/niks3089/pillar/issues/42)) ([9c649c4](https://github.com/niks3089/pillar/commit/9c649c47a6b633f7aae3f292c31285ede5078f30))
* **controller:** onboard command includes --cluster (fixes broken copy-paste) ([8663f36](https://github.com/niks3089/pillar/commit/8663f36f10c636108b7452b383a9b12f7c910250))

## [0.6.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.5.2...pillar-controller-v0.6.0) (2026-07-21)


### Features

* **controller:** export pillar_node_state one-hot gauge ([#33](https://github.com/niks3089/pillar/issues/33)) ([26312b4](https://github.com/niks3089/pillar/commit/26312b4c845487ba103ed801cd24f2a351e64952))


### Bug Fixes

* **install:** least-privilege sudoers policy, datadir helper, SECURITY.md ([#40](https://github.com/niks3089/pillar/issues/40)) ([61ece94](https://github.com/niks3089/pillar/commit/61ece949d15a9a34fb1b6aff49c20463f911595b))
* **install:** require or auto-detect --cluster; never silently default to mainnet-beta ([#38](https://github.com/niks3089/pillar/issues/38)) ([e661c84](https://github.com/niks3089/pillar/commit/e661c84c3b7404d541aacb0d6ea2c378b500812d))
* **scripts:** correct disk metric names in setup-grafana-alerts ([#34](https://github.com/niks3089/pillar/issues/34)) ([7c46303](https://github.com/niks3089/pillar/commit/7c4630324684f104e6fed2f0721775f6607118bc))

## [0.5.2](https://github.com/niks3089/pillar/compare/pillar-controller-v0.5.1...pillar-controller-v0.5.2) (2026-07-17)


### Bug Fixes

* close onboarding and from-source provisioning gaps ([0f08cea](https://github.com/niks3089/pillar/commit/0f08cea21fc7d5dee15dad4c6d3a0f8acc37c279))
* close onboarding and from-source provisioning gaps ([2828c89](https://github.com/niks3089/pillar/commit/2828c8966822b96f2db2a6311cb9b9b64b387470))
* fetch genesis with pinned hash instead of --no-genesis-fetch ([09543bf](https://github.com/niks3089/pillar/commit/09543bf8bf91b89e0f7cd76cc4034000d3eebbcb))
* persist client/cluster on provision so the UI shows the config ([a0df379](https://github.com/niks3089/pillar/commit/a0df3799595451e39c16256267ada43486612d1b))
* persist client/cluster on provision so the UI shows the config ([ad9b270](https://github.com/niks3089/pillar/commit/ad9b27056599a8454309f247c6c58f66f642ac99))

## [0.5.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.5.0...pillar-controller-v0.5.1) (2026-07-01)


### Features

* Cleaned up and wired with UI fully ([b91cdde](https://github.com/niks3089/pillar/commit/b91cddef36327c03218338cf2b1fe3ac622e7ada))
* Shifted to tailwind from .css ([ff9ab95](https://github.com/niks3089/pillar/commit/ff9ab956ba46f10f4e93c788a59a9083d2a853b8))

## [0.5.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.4.0...pillar-controller-v0.5.0) (2026-06-25)


### Features

* **branding:** add Pillar logo to README and web UI ([68b007a](https://github.com/niks3089/pillar/commit/68b007af186ed76557d13979b5e682c220a26d1e))


### Bug Fixes

* **controller:** allow loopback/private Grafana targets in SSRF guard ([6a7b72b](https://github.com/niks3089/pillar/commit/6a7b72b3026ae403929eb943b311854e8fa6449d))

## [0.4.0](https://github.com/niks3089/pillar/compare/pillar-controller-v0.3.1...pillar-controller-v0.4.0) (2026-06-23)


### Features

* **ui:** open Update Validator form in a modal instead of inline collapse ([682782a](https://github.com/niks3089/pillar/commit/682782ad0c8d9a85d6bf3c6a0b1aafbc44bf05ae))

## [0.3.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.3.1...pillar-controller-v0.3.1) (2026-06-20)


### Features

* add controller with web UI and dashboards ([2a6658c](https://github.com/niks3089/pillar/commit/2a6658c9bfd3dd3a1216a70d617943bcd05d2d62))
* add Surfpool as a client option (local test validator / fork) ([e6bc513](https://github.com/niks3089/pillar/commit/e6bc5134e3b6026ec0623aa1880771d524681afe))
* auto-release on push to main with release-please ([b047c0e](https://github.com/niks3089/pillar/commit/b047c0eb26c369d5e77031686038e03b858b2e94))
* **controller:** cluster-aware Jito MEV provisioning + ops docs ([f5bc52f](https://github.com/niks3089/pillar/commit/f5bc52f473d966c7d55e3e951bdbec5bcab3af2c))
* **controller:** cluster-aware Jito MEV provisioning + relayer support ([5f99b20](https://github.com/niks3089/pillar/commit/5f99b2087a0bdcedd0dce3f2eeeedf4550b1608e))
* **controller:** Jito + Firedancer source-build provisioning ([d1e56c8](https://github.com/niks3089/pillar/commit/d1e56c854b035401e541858eb06698d9a36b4a7c))
* **firedancer:** runnable provisioning — validated config + runtime setup ([8a98428](https://github.com/niks3089/pillar/commit/8a984287a72105d6800720c20a10d89ec1c39ce9))
* **firedancer:** runnable provisioning — validated config + runtime setup ([2da2e57](https://github.com/niks3089/pillar/commit/2da2e57e87d822da1a47183b84ccdd8b50aab736))
* rename crates for public distribution, add stop/cancel commands ([5b450de](https://github.com/niks3089/pillar/commit/5b450de098b770e61bdb34484392d29cea601847))
* separate release versions for agent and controller ([a5b9d92](https://github.com/niks3089/pillar/commit/a5b9d9216c51f787fb16461e878be9b46906fe5f))
* **ui:** in-app searchable Operations docs + per-row Grafana links + unique validator id ([77cd807](https://github.com/niks3089/pillar/commit/77cd807d189d73d9b46103abf6b0c1847a44823e))
* **ui:** modern design-system refresh ([dea0987](https://github.com/niks3089/pillar/commit/dea098721247de5f87da9dee2bd09fb36681bc01))
* **ui:** node-detail UX — validator terminology, per-node Grafana, ([28bc716](https://github.com/niks3089/pillar/commit/28bc716e1d50f4ce541df61918761ed1217fedda))
* update the ui ([e08bd85](https://github.com/niks3089/pillar/commit/e08bd85d8009e24f47a04acae151617675ef0860))


### Bug Fixes

* add creds ([51ea365](https://github.com/niks3089/pillar/commit/51ea365b4a927c12d00013e591aa41e88c5d6eb8))
* controller logs and service name width ([c829ba2](https://github.com/niks3089/pillar/commit/c829ba2ae0092aa83053be85d0ae02686abc5676))
* **firedancer:** build-tooling, sudoers, and TOML fixes from live test ([6a6f108](https://github.com/niks3089/pillar/commit/6a6f108ce5e74f244f656366242725ac22feffa5))
* first-run provisioning hardening from live bring-up ([1ddc560](https://github.com/niks3089/pillar/commit/1ddc5609dab49e0e4c74f61edaba78e7a4e17ea3))
* lazy pull ([a6cb053](https://github.com/niks3089/pillar/commit/a6cb0538de8b23d9fe8e26247f7abfcb4a4566d5))
* sync grafana dashboards and alert rules from dev machine ([fc51aa8](https://github.com/niks3089/pillar/commit/fc51aa82eded16d635b58727ca8aba918939a135))
* update checker manifest URL and CI manifest generation ([8e69a85](https://github.com/niks3089/pillar/commit/8e69a859b3ed339f2811625523b51bb725754b31))


### Miscellaneous Chores

* release 0.3.1 ([3101417](https://github.com/niks3089/pillar/commit/3101417a031caa69fd9976a081cd2d76aeb7160c))

## [0.3.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.3.1...pillar-controller-v0.3.1) (2026-06-16)


### Features

* add controller with web UI and dashboards ([2a6658c](https://github.com/niks3089/pillar/commit/2a6658c9bfd3dd3a1216a70d617943bcd05d2d62))
* auto-release on push to main with release-please ([b047c0e](https://github.com/niks3089/pillar/commit/b047c0eb26c369d5e77031686038e03b858b2e94))
* **controller:** cluster-aware Jito MEV provisioning + ops docs ([f5bc52f](https://github.com/niks3089/pillar/commit/f5bc52f473d966c7d55e3e951bdbec5bcab3af2c))
* **controller:** cluster-aware Jito MEV provisioning + relayer support ([5f99b20](https://github.com/niks3089/pillar/commit/5f99b2087a0bdcedd0dce3f2eeeedf4550b1608e))
* **controller:** Jito + Firedancer source-build provisioning ([d1e56c8](https://github.com/niks3089/pillar/commit/d1e56c854b035401e541858eb06698d9a36b4a7c))
* **firedancer:** runnable provisioning — validated config + runtime setup ([8a98428](https://github.com/niks3089/pillar/commit/8a984287a72105d6800720c20a10d89ec1c39ce9))
* **firedancer:** runnable provisioning — validated config + runtime setup ([2da2e57](https://github.com/niks3089/pillar/commit/2da2e57e87d822da1a47183b84ccdd8b50aab736))
* rename crates for public distribution, add stop/cancel commands ([5b450de](https://github.com/niks3089/pillar/commit/5b450de098b770e61bdb34484392d29cea601847))
* separate release versions for agent and controller ([a5b9d92](https://github.com/niks3089/pillar/commit/a5b9d9216c51f787fb16461e878be9b46906fe5f))
* update the ui ([e08bd85](https://github.com/niks3089/pillar/commit/e08bd85d8009e24f47a04acae151617675ef0860))


### Bug Fixes

* add creds ([51ea365](https://github.com/niks3089/pillar/commit/51ea365b4a927c12d00013e591aa41e88c5d6eb8))
* controller logs and service name width ([c829ba2](https://github.com/niks3089/pillar/commit/c829ba2ae0092aa83053be85d0ae02686abc5676))
* **firedancer:** build-tooling, sudoers, and TOML fixes from live test ([6a6f108](https://github.com/niks3089/pillar/commit/6a6f108ce5e74f244f656366242725ac22feffa5))
* first-run provisioning hardening from live bring-up ([1ddc560](https://github.com/niks3089/pillar/commit/1ddc5609dab49e0e4c74f61edaba78e7a4e17ea3))
* lazy pull ([a6cb053](https://github.com/niks3089/pillar/commit/a6cb0538de8b23d9fe8e26247f7abfcb4a4566d5))
* sync grafana dashboards and alert rules from dev machine ([fc51aa8](https://github.com/niks3089/pillar/commit/fc51aa82eded16d635b58727ca8aba918939a135))
* update checker manifest URL and CI manifest generation ([8e69a85](https://github.com/niks3089/pillar/commit/8e69a859b3ed339f2811625523b51bb725754b31))


### Miscellaneous Chores

* release 0.3.1 ([3101417](https://github.com/niks3089/pillar/commit/3101417a031caa69fd9976a081cd2d76aeb7160c))

## [0.3.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.3.1...pillar-controller-v0.3.1) (2026-02-23)


### Features

* add controller with web UI and dashboards ([2a6658c](https://github.com/niks3089/pillar/commit/2a6658c9bfd3dd3a1216a70d617943bcd05d2d62))
* auto-release on push to main with release-please ([b047c0e](https://github.com/niks3089/pillar/commit/b047c0eb26c369d5e77031686038e03b858b2e94))
* rename crates for public distribution, add stop/cancel commands ([5b450de](https://github.com/niks3089/pillar/commit/5b450de098b770e61bdb34484392d29cea601847))
* separate release versions for agent and controller ([a5b9d92](https://github.com/niks3089/pillar/commit/a5b9d9216c51f787fb16461e878be9b46906fe5f))
* update the ui ([e08bd85](https://github.com/niks3089/pillar/commit/e08bd85d8009e24f47a04acae151617675ef0860))


### Bug Fixes

* add creds ([51ea365](https://github.com/niks3089/pillar/commit/51ea365b4a927c12d00013e591aa41e88c5d6eb8))
* controller logs and service name width ([c829ba2](https://github.com/niks3089/pillar/commit/c829ba2ae0092aa83053be85d0ae02686abc5676))
* lazy pull ([a6cb053](https://github.com/niks3089/pillar/commit/a6cb0538de8b23d9fe8e26247f7abfcb4a4566d5))
* sync grafana dashboards and alert rules from dev machine ([fc51aa8](https://github.com/niks3089/pillar/commit/fc51aa82eded16d635b58727ca8aba918939a135))
* update checker manifest URL and CI manifest generation ([8e69a85](https://github.com/niks3089/pillar/commit/8e69a859b3ed339f2811625523b51bb725754b31))


### Miscellaneous Chores

* release 0.3.1 ([3101417](https://github.com/niks3089/pillar/commit/3101417a031caa69fd9976a081cd2d76aeb7160c))

## [0.3.1](https://github.com/niks3089/pillar/compare/pillar-controller-v0.3.1...pillar-controller-v0.3.1) (2026-02-23)


### Features

* add controller with web UI and dashboards ([2a6658c](https://github.com/niks3089/pillar/commit/2a6658c9bfd3dd3a1216a70d617943bcd05d2d62))
* auto-release on push to main with release-please ([b047c0e](https://github.com/niks3089/pillar/commit/b047c0eb26c369d5e77031686038e03b858b2e94))
* rename crates for public distribution, add stop/cancel commands ([5b450de](https://github.com/niks3089/pillar/commit/5b450de098b770e61bdb34484392d29cea601847))
* separate release versions for agent and controller ([a5b9d92](https://github.com/niks3089/pillar/commit/a5b9d9216c51f787fb16461e878be9b46906fe5f))
* update the ui ([e08bd85](https://github.com/niks3089/pillar/commit/e08bd85d8009e24f47a04acae151617675ef0860))


### Bug Fixes

* add creds ([51ea365](https://github.com/niks3089/pillar/commit/51ea365b4a927c12d00013e591aa41e88c5d6eb8))
* controller logs and service name width ([c829ba2](https://github.com/niks3089/pillar/commit/c829ba2ae0092aa83053be85d0ae02686abc5676))
* lazy pull ([a6cb053](https://github.com/niks3089/pillar/commit/a6cb0538de8b23d9fe8e26247f7abfcb4a4566d5))
* sync grafana dashboards and alert rules from dev machine ([fc51aa8](https://github.com/niks3089/pillar/commit/fc51aa82eded16d635b58727ca8aba918939a135))
* update checker manifest URL and CI manifest generation ([8e69a85](https://github.com/niks3089/pillar/commit/8e69a859b3ed339f2811625523b51bb725754b31))


### Miscellaneous Chores

* release 0.3.1 ([3101417](https://github.com/niks3089/pillar/commit/3101417a031caa69fd9976a081cd2d76aeb7160c))
