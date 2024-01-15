.PHONY: run-node node-configs clean
IP?=127.0.0.1:3054
DOCKER_IP=172.12.0.10

node:
	export RUST_LOG=INFO && cd node-configs/${IP} && cargo run -- --database ./database/${IP} 

node_configs:
	cargo run --bin localnet_config -- --input-addrs addresses.txt --output-dir node-configs

node_docker:
	rm -rf addresses.txt
	touch addresses.txt
	echo ${DOCKER_IP}:3054 >> addresses.txt
	docker build -t consensus-node ../
	docker network create --subnet=172.12.0.0/16 node-net
	docker run --name test-consensus-node --net node-net --ip ${DOCKER_IP} -itd consensus-node

consenus_docker_example:
	rm -rf addresses.txt
	echo 172.12.0.10:3054 >> addresses.txt
	echo 172.12.0.11:3054 >> addresses.txt
	cd .. && docker-compose up -d

clean: clean_docker
	rm -rf node-configs
	rm -rf database

clean_docker:
	docker rm test-consensus-node
	docker rm consensus-node-1
	docker rm consensus-node-2
	docker network rm node-net
	docker image rm consensus-node
