.PHONY: run-node node-configs clean
IP?=127.0.0.1:3054
DOCKER_IP=172.12.0.10

node:
	export RUST_LOG=INFO && cd node-configs/${IP} && cargo run -- --database ./database/${IP} 

node_configs:
	cargo run --bin localnet_config -- --input-addrs addresses.txt --output-dir node-configs

docker_node_configs:
	cargo run --release --bin localnet_config -- --input-addrs docker-config/addresses.txt --output-dir docker-config/node-configs

node_docker:
	mkdir -p docker-config
	cd docker-config && rm -rf addresses.txt && echo ${DOCKER_IP}:3054 >> addresses.txt
	cd .. && docker-compose up -d node-1

consenus_docker_example:
	mkdir -p docker-config
	cd docker-config && rm -rf addresses.txt && touch addresses.txt && echo 172.12.0.10:3054 >> addresses.txt &&  echo 172.12.0.11:3054 >> addresses.txt
	cd .. && docker-compose up -d

clean: clean_docker
	rm -rf node-configs
	rm -rf database

clean_docker:
	rm -rf docker-config
	docker rm test-consensus-node
	docker rm consensus-node-1
	docker rm consensus-node-2
	docker network rm node-net
	docker image rm consensus-node
