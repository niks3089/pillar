# Changelog

## [0.8.1](https://github.com/niks3089/pillar/compare/pillar-agent-v0.8.0...pillar-agent-v0.8.1) (2026-07-29)


### Bug Fixes

* **agent:** re-read surfpool binary version each reconcile ([022e090](https://github.com/niks3089/pillar/commit/022e090af9a01dd411a72f8b78589dc86ad37d39))
* **agent:** re-read surfpool binary version each reconcile ([32f3df2](https://github.com/niks3089/pillar/commit/32f3df2bfeb6a182a51a95dac9d9be0aa643706c))

## [0.8.0](https://github.com/niks3089/pillar/compare/pillar-agent-v0.7.0...pillar-agent-v0.8.0) (2026-07-29)


### Features

* add Mithril as a fourth validator client ([0d43e67](https://github.com/niks3089/pillar/commit/0d43e67ef42851423f55ec84017304a6ac917433))

## [0.7.0](https://github.com/niks3089/pillar/compare/pillar-agent-v0.6.0...pillar-agent-v0.7.0) (2026-07-29)


### Features

* **agent:** report surfpool's own release version, exempt it from mismatch ([3f5884b](https://github.com/niks3089/pillar/commit/3f5884becb6e3779c8ede1357d5fa0fed37f9e19))
* config UX — version suggestions, dismissible outcome, surfpool version fix ([ba73593](https://github.com/niks3089/pillar/commit/ba7359341060f44e99378c54e0a5c5bd6d1577ef))

## [0.6.0](https://github.com/niks3089/pillar/compare/pillar-agent-v0.5.0...pillar-agent-v0.6.0) (2026-07-29)


### Features

* **agent:** report validator process start time ([67b0f1f](https://github.com/niks3089/pillar/commit/67b0f1fdc61c332ee66d0f288ac42a603cb62e11))
* real validator process uptime ([5a9441e](https://github.com/niks3089/pillar/commit/5a9441e3ec2e442a50c2e71cf7eb50ec24e6190d))

## [0.5.0](https://github.com/niks3089/pillar/compare/pillar-agent-v0.4.1...pillar-agent-v0.5.0) (2026-07-21)


### Features

* **controller:** export pillar_node_state one-hot gauge ([#33](https://github.com/niks3089/pillar/issues/33)) ([26312b4](https://github.com/niks3089/pillar/commit/26312b4c845487ba103ed801cd24f2a351e64952))


### Bug Fixes

* **install:** least-privilege sudoers policy, datadir helper, SECURITY.md ([#40](https://github.com/niks3089/pillar/issues/40)) ([61ece94](https://github.com/niks3089/pillar/commit/61ece949d15a9a34fb1b6aff49c20463f911595b))
* **install:** require or auto-detect --cluster; never silently default to mainnet-beta ([#38](https://github.com/niks3089/pillar/issues/38)) ([e661c84](https://github.com/niks3089/pillar/commit/e661c84c3b7404d541aacb0d6ea2c378b500812d))
* **scripts:** correct disk metric names in setup-grafana-alerts ([#34](https://github.com/niks3089/pillar/issues/34)) ([7c46303](https://github.com/niks3089/pillar/commit/7c4630324684f104e6fed2f0721775f6607118bc))

## [0.4.1](https://github.com/niks3089/pillar/compare/pillar-agent-v0.4.0...pillar-agent-v0.4.1) (2026-07-17)


### Bug Fixes

* close onboarding and from-source provisioning gaps ([0f08cea](https://github.com/niks3089/pillar/commit/0f08cea21fc7d5dee15dad4c6d3a0f8acc37c279))
* close onboarding and from-source provisioning gaps ([2828c89](https://github.com/niks3089/pillar/commit/2828c8966822b96f2db2a6311cb9b9b64b387470))

## [0.4.0](https://github.com/niks3089/pillar/compare/pillar-agent-v0.3.1...pillar-agent-v0.4.0) (2026-06-25)


### Features

* **security:** harden control plane (template injection, auth, mTLS) ([265897d](https://github.com/niks3089/pillar/commit/265897d1721b0e67db2f313e26c7e614459344c3))

## [0.3.1](https://github.com/niks3089/pillar/compare/pillar-agent-v0.3.1...pillar-agent-v0.3.1) (2026-06-20)


### Features

* add lot of improvments ([0e1b77b](https://github.com/niks3089/pillar/commit/0e1b77be61d53fccd1f10aafdd425f53c497caeb))
* add Surfpool as a client option (local test validator / fork) ([e6bc513](https://github.com/niks3089/pillar/commit/e6bc5134e3b6026ec0623aa1880771d524681afe))
* separate release versions for agent and controller ([a5b9d92](https://github.com/niks3089/pillar/commit/a5b9d9216c51f787fb16461e878be9b46906fe5f))
* update the ui ([e08bd85](https://github.com/niks3089/pillar/commit/e08bd85d8009e24f47a04acae151617675ef0860))


### Miscellaneous Chores

* release 0.3.1 ([3101417](https://github.com/niks3089/pillar/commit/3101417a031caa69fd9976a081cd2d76aeb7160c))

## [0.3.1](https://github.com/niks3089/pillar/compare/pillar-agent-v0.3.1...pillar-agent-v0.3.1) (2026-06-16)


### Features

* add lot of improvments ([0e1b77b](https://github.com/niks3089/pillar/commit/0e1b77be61d53fccd1f10aafdd425f53c497caeb))
* separate release versions for agent and controller ([a5b9d92](https://github.com/niks3089/pillar/commit/a5b9d9216c51f787fb16461e878be9b46906fe5f))
* update the ui ([e08bd85](https://github.com/niks3089/pillar/commit/e08bd85d8009e24f47a04acae151617675ef0860))


### Miscellaneous Chores

* release 0.3.1 ([3101417](https://github.com/niks3089/pillar/commit/3101417a031caa69fd9976a081cd2d76aeb7160c))

## [0.3.1](https://github.com/niks3089/pillar/compare/pillar-agent-v0.3.1...pillar-agent-v0.3.1) (2026-02-23)


### Features

* add lot of improvments ([0e1b77b](https://github.com/niks3089/pillar/commit/0e1b77be61d53fccd1f10aafdd425f53c497caeb))
* separate release versions for agent and controller ([a5b9d92](https://github.com/niks3089/pillar/commit/a5b9d9216c51f787fb16461e878be9b46906fe5f))
* update the ui ([e08bd85](https://github.com/niks3089/pillar/commit/e08bd85d8009e24f47a04acae151617675ef0860))


### Miscellaneous Chores

* release 0.3.1 ([3101417](https://github.com/niks3089/pillar/commit/3101417a031caa69fd9976a081cd2d76aeb7160c))

## [0.3.1](https://github.com/niks3089/pillar/compare/pillar-agent-v0.3.1...pillar-agent-v0.3.1) (2026-02-23)


### Features

* add lot of improvments ([0e1b77b](https://github.com/niks3089/pillar/commit/0e1b77be61d53fccd1f10aafdd425f53c497caeb))
* separate release versions for agent and controller ([a5b9d92](https://github.com/niks3089/pillar/commit/a5b9d9216c51f787fb16461e878be9b46906fe5f))
* update the ui ([e08bd85](https://github.com/niks3089/pillar/commit/e08bd85d8009e24f47a04acae151617675ef0860))


### Miscellaneous Chores

* release 0.3.1 ([3101417](https://github.com/niks3089/pillar/commit/3101417a031caa69fd9976a081cd2d76aeb7160c))
