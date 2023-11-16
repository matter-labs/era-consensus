# Running the project

The zkSync Era Consensus Layer is designed to be used as a library, providing consensus and networking services to an execution layer. That being said, there are still situations where one might want to run the consensus layer stand-alone, as an application. Namely, we might want to run just the consensus on a real network for testing and benchmarking. We might also want to run the project as a local testnet, which is useful for development.

## Prerequisites

In order for the project to run performantly (both as an application and a library), we need to disable TCP slow start on the machine operating system. This can be done in Linux with the following command:

```
sysctl -w net.ipv4.tcp_slow_start_after_idle=0
```

## Running as an application

TBD