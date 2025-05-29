# Changelog

## [0.11.1](https://github.com/matter-labs/era-consensus/compare/v0.11.0...v0.11.1) (2025-05-29)


### Bug Fixes

* Fix test and several bugs for validator schedule rotation ([#256](https://github.com/matter-labs/era-consensus/issues/256)) ([1769ec7](https://github.com/matter-labs/era-consensus/commit/1769ec71c5171c68a95261b54129edd76e6885f3))
* Only generate supported protocol versions for tests. ([#254](https://github.com/matter-labs/era-consensus/issues/254)) ([c3ff672](https://github.com/matter-labs/era-consensus/commit/c3ff6729c8eec6adc2aefcf335b2bd8e1fb4c359))

## [0.11.0](https://github.com/matter-labs/era-consensus/compare/v0.10.0...v0.11.0) (2025-05-22)


### Features

* Dynamic validator sets ([#252](https://github.com/matter-labs/era-consensus/issues/252)) ([ab66412](https://github.com/matter-labs/era-consensus/commit/ab66412360b45292c3fa9821dcd3c2523d696a57))
* Improve leader selection ([#251](https://github.com/matter-labs/era-consensus/issues/251)) ([e64f65a](https://github.com/matter-labs/era-consensus/commit/e64f65a53184efda52fcb181b703e2a9878411bf))

## [0.10.0](https://github.com/matter-labs/era-consensus/compare/v0.9.0...v0.10.0) (2025-04-15)


### Features

* Simplified specification in Quint including the inductive invariant ([#245](https://github.com/matter-labs/era-consensus/issues/245)) ([0e78acf](https://github.com/matter-labs/era-consensus/commit/0e78acfb7054776f244ba2de9b26f2c6c41fad01))


### Bug Fixes

* manually upgrade to 0.9 ([#241](https://github.com/matter-labs/era-consensus/issues/241)) ([7c1e937](https://github.com/matter-labs/era-consensus/commit/7c1e937b1958726ba120c94973308d6ad0795ebe))

## [0.8.0](https://github.com/matter-labs/era-consensus/compare/v0.7.0...v0.8.0) (2025-01-20)


### Features

* documented the dangers of ordering in message encoding ([#225](https://github.com/matter-labs/era-consensus/issues/225)) ([c1b8e6e](https://github.com/matter-labs/era-consensus/commit/c1b8e6e021c5b958b047ac1478a337a6efe9e8cd))


### Bug Fixes

* Replicas forget messages and halt on restart ([#228](https://github.com/matter-labs/era-consensus/issues/228)) ([c723fbe](https://github.com/matter-labs/era-consensus/commit/c723fbe2453a52bab8fba9c202a404e5fc4fb532))

## [0.7.0](https://github.com/matter-labs/era-consensus/compare/v0.6.0...v0.7.0) (2024-12-16)


### Features

* Added tool to recover public keys from secret keys ([#224](https://github.com/matter-labs/era-consensus/issues/224)) ([f1522f8](https://github.com/matter-labs/era-consensus/commit/f1522f8b23ef1a5450e626d187accac6bc637eb1))
* Added view timeout duration as a config parameter ([#222](https://github.com/matter-labs/era-consensus/issues/222)) ([f07fcfa](https://github.com/matter-labs/era-consensus/commit/f07fcfa67e298d53ddeb801ce20c3ea2571e92da))
* loadtest improvements ([#223](https://github.com/matter-labs/era-consensus/issues/223)) ([69f11c7](https://github.com/matter-labs/era-consensus/commit/69f11c7396e4980c3db7999fb8dbb6bc7cff1fe5))


### Bug Fixes

* Fix high vote on informal spec ([#215](https://github.com/matter-labs/era-consensus/issues/215)) ([c586f85](https://github.com/matter-labs/era-consensus/commit/c586f850674517975e2c97b9e2a61f6eca25bdf9))
* TimeoutQC aggregation ([#220](https://github.com/matter-labs/era-consensus/issues/220)) ([8a49824](https://github.com/matter-labs/era-consensus/commit/8a498246b2c2d88d63e51049bb3acd20a8166479))

## [0.6.0](https://github.com/matter-labs/era-consensus/compare/v0.5.0...v0.6.0) (2024-11-14)


### Features

* Implement ChonkyBFT ([#211](https://github.com/matter-labs/era-consensus/issues/211)) ([f4cc128](https://github.com/matter-labs/era-consensus/commit/f4cc128114027188e34a355e20f084777041480d))


### Bug Fixes

* Add wrap around to table cells on Debug page ([#214](https://github.com/matter-labs/era-consensus/issues/214)) ([6d52340](https://github.com/matter-labs/era-consensus/commit/6d523401b73a431a24f565dadf0471611c0c220b))

## [0.5.0](https://github.com/matter-labs/era-consensus/compare/v0.4.0...v0.5.0) (2024-10-09)


### Features

* support for syncing pre-genesis blocks ([#203](https://github.com/matter-labs/era-consensus/issues/203)) ([6a4a695](https://github.com/matter-labs/era-consensus/commit/6a4a69511b5c0611603eb881e9e3f443e69949bc))

## [0.4.0](https://github.com/matter-labs/era-consensus/compare/v0.3.0...v0.4.0) (2024-10-07)


### Features

* Expand metrics in HTTP debug page ([#205](https://github.com/matter-labs/era-consensus/issues/205)) ([2ef11bc](https://github.com/matter-labs/era-consensus/commit/2ef11bc0bc0ef9b332c4a4c2715c523143e844bd))

## [0.3.0](https://github.com/matter-labs/era-consensus/compare/v0.2.0...v0.3.0) (2024-09-26)


### Features

* dummy pr to trigger release-please ([#201](https://github.com/matter-labs/era-consensus/issues/201)) ([fd78177](https://github.com/matter-labs/era-consensus/commit/fd781776efb8d68b6a4c16380f7ce154ad321141))

## [0.2.0](https://github.com/matter-labs/era-consensus/compare/v0.1.1...v0.2.0) (2024-09-24)


### Features

* human readable information for handshake failures ([#196](https://github.com/matter-labs/era-consensus/issues/196)) ([a298f50](https://github.com/matter-labs/era-consensus/commit/a298f504ac7f5c89e9dbc201721a89b1eeaa7663))

## [0.1.1](https://github.com/matter-labs/era-consensus/compare/v0.1.0...v0.1.1) (2024-09-06)


### Features

* Remove pairing crate dependency ([#190](https://github.com/matter-labs/era-consensus/issues/190)) ([9c654a4](https://github.com/matter-labs/era-consensus/commit/9c654a4333b7864fd704e941b5eafefec5e830cf))
