# Architecture

## High-Level Overview

The architecture of era-bft loosely follows the [Actor model](https://en.wikipedia.org/wiki/Actor_model). We picked the actor model to have both concurrency and strong code encapsulation. All the crates in this repo can be divided into two categories: actors and libraries.

### Actor crates

The actor crates are where the vast majority of the work is done. Each of them maintains its own separate state and communicate with each other through message passing. We have the following actor crates:

- the `bft` crate implements the logic for the consensus algorithm.

- the `executor` crate is responsible for parsing the configuration parameters given by the user, and initializing the actors and the storage. It's basically the bootloader for the node. It also dispatches messages between the rest of the actors. They all send messages to the executor and it then converts and forwards the messages to the desired destination. This improves the encapsulation of the actors.

- the `network` crate which maintains a pool of outbound and inbound connections to other nodes. It also implements a syncing mechanism for nodes (for blocks, batches, attester signatures, etc).

### Library crates

The library crates are just crates that have some basic functionality that can be used by the other crates. They do not use message passing and are not initialized by the `executor` crate, they are just invoked directly by the other crates. So far, we have these crates:

- the `concurrency` crate, which provides concurrency primitives based on the [structured concurrency](https://en.wikipedia.org/wiki/Structured_concurrency) paradigm.

- the `crypto` crate, which provides several cryptographic primitives.

- the `protobuf` and `protbuf_build` crates, which contains all the code to create the protobuf schemas used by the other crates.

- the `roles` crate, which implements the types necessary for each role in the network. We have just two roles: `Node` and `Validator`.

- the `storage` crate is responsible for storing the current state of the system and providing an interface to access the stored data. It is a key component of the system that ensures the persistence of data and the ability to retrieve it when needed.

- the `utils` crate, which holds several small utilities and primitives.

## Low-Level Overview

This section provides a physical map of folders & files in this repository.

- `/infrastructure`: Infrastructure scripts that are needed to test the zkSync Era Consensus Layer.

- `/node`

  - `/actors`: Crates that implement specific actor components.

    - `/bft`: The consensus actor.
    - `/executor`: The actor orchestrator.
    - `/network`: The network actor.

  - `/lib`: All the library crates used as dependencies of the actor crates above.

    - `/concurrency`: Crate with essential primitives for structured concurrency.
    - `/crypto`: Cryptographic primitives used by the other crates.
    - `/protobuf`: Code generated from protobuf schema files and utilities for serialization.
    - `/protobuf_build`: Generates rust code from the proto files.
    - `/roles`: Essential types for the different node roles.
    - `/storage`: Storage layer for the node.
    - `/utils`: Collection of small utilities.

  - `/tools`: Utility binaries needed to work with and test the node.
