# Running a test consensus node

## Local

1. Generate a file named `addresses.txt` in the root directory of the tools crate, containing node addresses in the format `IP:PORT`, with each address on a separate line.
2. Run `make node_configs`. This command will establish a directory named `node-configs` and create a folder for each address listed in the `.txt` file, providing the necessary configuration files for the respective node.
3. Execute `make node IP=<NODE_IP>`. The default value for this command would be `127.0.0.1:3054`. Note that running this command will take control of the terminal.

## Dockerized

To get up a standalone consensus node running in a docker container just run the following command inside the tools crate:

`make node_docker`

This will create a container running a single node advancing views and finalizing blocks.

To set up a simple example with two different nodes communicating with each other in running in different containers run the following command:

`make consenus_docker_example`

This will set up two distinct containers, each hosting a consensus node. These nodes will be interlinked, progressing through views and finalizing blocks achieving consensus between them.

To clean all the state after running these commands use:

`make clean_docker`

> This will delete the generated images and containers, requiring them to be regenerated.
