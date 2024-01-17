.PHONY: node node_configs docker_node_configs node_docker consensus_docker_example clean clean_docker
IP?=127.0.0.1:3054
DOCKER_IP=172.12.0.10
EXECUTABLE_NODE_DIR=node/tools

# Locally run commands

node:
	export RUST_LOG=INFO && cd ${EXECUTABLE_NODE_DIR}/node-configs/${IP} && cargo run -- --database ../../database/${IP} 

node_configs:
	cd ${EXECUTABLE_NODE_DIR} && cargo run --bin localnet_config -- --input-addrs addresses.txt --output-dir node-configs

# Docker commands

docker_node_configs:
	cd ${EXECUTABLE_NODE_DIR} && cargo run --release --bin localnet_config -- --input-addrs docker-config/addresses.txt --output-dir docker-config/node-configs

node_docker:
	mkdir -p ${EXECUTABLE_NODE_DIR}/docker-config
	cd ${EXECUTABLE_NODE_DIR}/docker-config && rm -rf addresses.txt && echo ${DOCKER_IP}:3054 >> addresses.txt
	docker-compose up -d node-1

consensus_docker_example:
	mkdir -p ${EXECUTABLE_NODE_DIR}/docker-config
	cd ${EXECUTABLE_NODE_DIR}/docker-config && rm -rf addresses.txt && touch addresses.txt && echo 172.12.0.10:3054 >> addresses.txt &&  echo 172.12.0.11:3054 >> addresses.txt
	docker-compose up -d

# Clean commands

clean: clean_docker
	rm -rf ${EXECUTABLE_NODE_DIR}/node-configs
	rm -rf ${EXECUTABLE_NODE_DIR}/database

clean_docker:
	rm -rf ${EXECUTABLE_NODE_DIR}/docker-config
	docker rm -f consensus-node
	docker rm -f consensus-node-1
	docker rm -f consensus-node-2
	docker network rm -f node-net
	docker image rm -f consensus-node
