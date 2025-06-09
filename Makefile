.PHONY: node nodes_config addresses_file blank_configs
IP?=127.0.0.1:3054
EXECUTABLE_NODE_DIR=node/tools
NODES=4
SEED_NODES=1

# Locally run commands

node:
	export RUST_LOG=INFO && cd ${EXECUTABLE_NODE_DIR}/nodes-config/${IP} && cargo run -- --database ../../database/node_${NODE}

nodes_config:
	cd ${EXECUTABLE_NODE_DIR} && cargo run --bin localnet_config -- --input-addrs addresses.txt --output-dir nodes-config
