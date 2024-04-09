# Running Test Consensus Nodes

These instructions guide you through the process of setting up and running a test consensus node in both local and Clustered environments.

## Local Setup

1. Edit the `addresses.txt` file located in the root directory of the tools crate. This file contains node addresses in the format `IP:PORT`. For a single node, use the example file. To run multiple nodes communicating with each other, write each node address on a separate line. This will run one node per address.
   
2. Move to the project root (era-consensus) and execute the following commands:

    ```bash
    make nodes_config
    ```

    This command creates a directory named `nodes-config` and generates a folder for each address listed in the `.txt` file with the ip as the directory name, providing essential configuration files for the corresponding node. 
    
    ```bash
    make node IP=<NODE_IP>
    ```

    The default value for this command is set to `127.0.0.1:3054` for launching the initial node, to run a different node just use the IP previously detailed in the `addresses.txt` file. Note that running this command will take control of the terminal.


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
make clean
```

This will remove all namespaces, deployments and pods from the minikube environment and the images generated in Docker.
If you want to stop the `minikube` environment just do:

```bash
minikube delete --all
```
