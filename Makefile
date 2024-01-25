.PHONY: node nodes_config docker_node_configs node_docker consensus_docker_example clean clean_docker addresses_file blank_configs
NODE?=0
DOCKER_IP=172.12.0.10
EXECUTABLE_NODE_DIR=node/tools
NODES=4

# Locally run commands

node:
	export RUST_LOG=INFO && cd ${EXECUTABLE_NODE_DIR}/nodes-config/node_${NODE} && cargo run -- --database ../../database/node_${NODE}

nodes_config:
	cd ${EXECUTABLE_NODE_DIR} && cargo run --bin localnet_config -- --input-addrs addresses.txt --output-dir nodes-config

# Docker commands

docker_build_executor:
	docker build --output=node/tools/docker_binaries --target=binary .

docker_node_image:
	docker build -t consensus-node --target=runtime .

docker_nodes_config:
	cd ${EXECUTABLE_NODE_DIR} && cargo run --release --bin localnet_config -- --input-addrs docker-config/addresses.txt --output-dir docker-config/nodes-config

docker_node:
	$(MAKE) docker_node_image
	docker run -d --name consensus-node-${NODE} --env NODE_ID="node_${NODE}" consensus-node

consensus_docker_example:
	mkdir -p ${EXECUTABLE_NODE_DIR}/docker-config
	cd ${EXECUTABLE_NODE_DIR}/docker-config && rm -rf addresses.txt && echo 172.12.0.10:3054 >> addresses.txt &&  echo 172.12.0.11:3054 >> addresses.txt
	$(MAKE) docker_nodes_config
	$(MAKE) docker_node_image
	docker-compose up -d

stop_docker_nodes:
	docker stop consensus-node-1 consensus-node-2

# Clean commands

clean: clean_docker
	rm -rf ${EXECUTABLE_NODE_DIR}/nodes-config
	rm -rf ${EXECUTABLE_NODE_DIR}/database

clean_docker:
	rm -rf ${EXECUTABLE_NODE_DIR}/docker-config/nodes-config
	rm -rf ${EXECUTABLE_NODE_DIR}/docker_binaries
	docker rm -f consensus-node-1
	docker rm -f consensus-node-2
	docker network rm -f node-net
	docker image rm -f consensus-node

addresses_file:
	mkdir -p ${EXECUTABLE_NODE_DIR}/docker-config
	cd ${EXECUTABLE_NODE_DIR}/docker-config && \
	rm -rf addresses.txt && \
	touch addresses.txt && \
	for n in $$(seq 0 $$((${NODES} - 1))); do echo 0.0.0.$$n:3054 >> addresses.txt; done

blank_configs: addresses_file docker_node_configs
	for n in $$(seq 0 $$((${NODES} - 1))); do \
	   jq '.publicAddr = "0.0.0.0:3054"' node/tools/docker-config/nodes-config/node_$$n/config.json | \
	   jq '.gossipStaticOutbound = "[]"' > node/tools/docker-config/nodes-config/node_$$n/config.tmp && \
	   mv -f node/tools/docker-config/nodes-config/node_$$n/config.tmp node/tools/docker-config/nodes-config/node_$$n/config.json; \
	done