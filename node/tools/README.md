# Running Test Consensus Nodes

These instructions guide you through the process of setting up and running a test consensus node in both local and Dockerized environments. Additionally, examples are provided to demonstrate how to run two or more nodes, communicating and doing consensus between them.

## Local Setup

1. Edit the `addresses.txt` file located in the root directory of the tools crate. This file contains node addresses in the format `IP:PORT`. For a single node, use the example file. To run multiple nodes communicating with each other, write each node address on a separate line. This will run one node per address.
   
2. Move to the project root (era-consensus) and execute the following commands:

    ```bash
    make nodes_config
    ```

    This command creates a directory named `nodes-config` and generates a folder for each address listed in the `.txt` file with the name `node_{NODE_NUMBER}`, providing essential configuration files for the corresponding node. The `NODE_NUMBER` is simply a numerical identifier for each node.
    
    ```bash
    make node NODE=<NODE_NUMBER>
    ```

    The default value for this command is set to `0` for launching the initial node, and you can increment the number for subsequent nodes. Note that running this command will take control of the terminal.

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


## Running in minikube

To run a number of nodes locally in minikube, first we need to build the binary:

```bash
make docker_build_executor
```

This command will create the `executor` binary to be included in a docker image.

Before running the deployment script, ensure minikube is installed and running:

```bash
minikube start
```

Then run

```bash
make start_k8s_nodes NODES=<number> SEED_NODES=<number>
```

Here, `NODES` is the desired total amount of nodes to run in the k8s cluster (defaults to 4 if omitted), and `SEED_NODES` is the amount of those nodes to be deployed first as seed nodes (defaults to 1).

This command will:
- Generate the configuration files for each node (this step may change in the future, as we may not need to include configuration files in the images)
- Build a docker image with the configuration files and the binary created in previous step
- Deploy the initial seed nodes
- Obtain the internal IP addresses of the seed nodes
- Deploy the rest of the nodes providing the seed nodes IP addresses to establish the connections

You may run

```bash
minikube dashboard
```

To start the minikube dashboard in order to inspect the deployed pods. Remember to use `consensus` namespace to find all consensus related infrastructure.

Finally to clean everything up

```bash
minikube delete --all
```

will remove all namespaces, deployments and pods from the minikube environment.