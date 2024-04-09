.PHONY: node nodes_config docker_nodes_config node_docker consensus_docker_example clean clean_docker addresses_file blank_configs
IP?=127.0.0.1:3054
EXECUTABLE_NODE_DIR=node/tools
NODES=4
SEED_NODES=1

# Locally run commands

node:
	export RUST_LOG=INFO && cd ${EXECUTABLE_NODE_DIR}/nodes-config/${IP} && cargo run -- --database ../../database/node_${NODE}

nodes_config:
	cd ${EXECUTABLE_NODE_DIR} && cargo run --bin localnet_config -- --input-addrs addresses.txt --output-dir nodes-config

# Docker commands

docker_build_executor:
	docker build --output=node/tools/docker_binaries --target=executor-binary .

docker_node_image:
	docker build -t consensus-node --target=executor-runtime .

# Kubernetes commands

start_k8s_nodes:
	$(MAKE) docker_node_image
	minikube image load consensus-node:latest
	cd ${EXECUTABLE_NODE_DIR} && cargo run --release --bin deployer -- --nodes ${NODES} --seed-nodes ${SEED_NODES}

# Clean commands

clean: clean_docker clean_k8s
	rm -rf ${EXECUTABLE_NODE_DIR}/nodes-config
	rm -rf ${EXECUTABLE_NODE_DIR}/database

clean_k8s:
	rm -rf ${EXECUTABLE_NODE_DIR}/k8s_configs
	kubectl delete deployments --all -n consensus
	kubectl delete pods --all -n consensus
	kubectl delete services --all -n consensus
	kubectl delete namespaces consensus

clean_docker:
	docker image rm -f consensus-node:latest
	docker image rm -f test-suite
