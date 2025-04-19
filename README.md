# zkSync Era Consensus Layer

This repo implements the consensus layer for ZK Stack chains. We implement all the necessary components for a set of sequencers to reach consensus over blocks. The zkSync Era Consensus Layer is designed to be used as a library, providing consensus and networking services to an execution layer.

## Prerequisites

In order for the project to run performantly (both as an application and a library), we need to disable TCP slow start on the machine operating system. This can be done in Linux with the following command:

```
sysctl -w net.ipv4.tcp_slow_start_after_idle=0
```

## Architecture

This section provides a physical map of folders & files in this repository.

- `/infrastructure`: Infrastructure scripts that are needed to test the zkSync Era Consensus Layer.

- `/node`

  - `/components`: Crates that implement specific components. Each of them maintains its own separate state and communicate with each other through message passing.

    - `/bft`: Implements the logic for the consensus algorithm.
    - `/executor`: Responsible for parsing the configuration parameters given by the user, and initializing the components and the interface with the execution layer. It's basically the bootloader for the node.
    - `/network`: Handles communication with other nodes and maintains a pool of outbound and inbound connections. It also implements a syncing mechanism (for blocks, etc).

  - `/lib`: All the library crates used as dependencies of the component crates above.

    - `/concurrency`: Crate with essential primitives for structured concurrency.
    - `/crypto`: Cryptographic primitives used by the other crates.
    - `/storage`: Provides an interface to the execution layer.
    - `/protobuf`: Code generated from protobuf schema files and utilities for serialization used by the other crates.
    - `/protobuf_build`: Generates rust code from the proto files.
    - `/roles`: Implements the types necessary for each role in the network. We have just two roles: `Node` and `Validator`.
    - `/utils`: Collection of small utilities and primitives.

  - `/tools`: Utility binaries needed to work with and test the node.


## Policies

- [Security policy](.github/SECURITY.md)
- [Contribution policy](.github/CONTRIBUTING.md)

## License

zkSync Era is distributed under the terms of either

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Official Links

- [Website](https://zksync.io/)
- [GitHub](https://github.com/matter-labs)
- [ZK Credo](https://github.com/zksync/credo)
- [Twitter](https://twitter.com/zksync)
- [Twitter for Devs](https://twitter.com/zkSyncDevs)
- [Discord](https://join.zksync.dev/)