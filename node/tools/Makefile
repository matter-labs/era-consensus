.PHONY: run-node node-configs clean
IP?=127.0.0.1:3054

run-node:
	export RUST_LOG=INFO && cd node-configs/${IP} && cargo run -- --database ./database/${IP} 

node-configs:
	cargo run --bin localnet_config -- --input-addrs config.txt --output-dir node-configs

clean:
	rm -rf node-configs
	rm -rf database
