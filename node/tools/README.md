# Running Test Consensus Nodes

These instructions guide you through the process of setting up and running a test consensus node in both local and Dockerized environments. Additionally, examples are provided to demonstrate how to run two or more nodes, communicating and doing consensus between them.

## Local Setup

1. Edit the `addresses.txt` file located in the root directory of the tools crate. This file contains node addresses in the format `IP:PORT`. For a single node, use the example file. To run multiple nodes communicating with each other, write each node address on a separate line. This will run one node per address.
   
2. Move to the project root (era-consensus) and execute the following commands:

    ```bash
    make nodes_config
    ```

    This command establishes a directory named `nodes-config` and creates a folder for each address listed in the `.txt` file, providing necessary configuration files for the respective node.
    
    ```bash
    make node IP=<NODE_IP>
    ```

    The default value for this command is `127.0.0.1:3054`. Note that running this command will take control of the terminal.

## Dockerized Setup

To launch a standalone consensus node in a Docker container, run the following command in the project root (era-consensus):

```bash
make node_docker
```

This command creates a container running a single node that advances views and finalizes blocks.

For a simple example with two nodes communicating in different containers, use:

```bash
make consensus_docker_example
```

This sets up two containers, each hosting a consensus node, interlinked and progressing through views to finalize blocks, achieving consensus between them.

To stop the node containers, use:

```bash
make stop_docker_nodes
```

The node will resume the last viewed block from the previous session when initiated again.

To clean all states after running these commands, use:

```bash
make clean_docker
```

> This deletes the generated images and containers, requiring regeneration.
