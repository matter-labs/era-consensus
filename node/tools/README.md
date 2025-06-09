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