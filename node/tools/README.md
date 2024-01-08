# Running a test node

1. Generate a file named `config.txt` in the root directory of the tools crate, containing node addresses in the format `IP:PORT`, with each address on a separate line.
2. Run `make node-configs`. This command will establish a directory named `node-configs` and create a folder for each address listed in the `.txt` file, providing the necessary configuration files for the respective node.
3. Execute `make run-node IP=<NODE_IP>`. The default value for this command would be `127.0.0.1:8000`. Note that running this command will take control of the terminal.
