.PHONY: run-node node-configs clean
IP?=127.0.0.1:8000

run-node:
	export RUST_LOG=INFO && cargo run -- --config-file node-configs/${IP}/config.json --node-key node-configs/${IP}/node_key --validator-key node-configs/${IP}/validator_key --database ./database/${IP} 

node-configs:
	cargo run --bin localnet_config -- --input-addrs config.txt --output-dir node-configs

clean:
	rm -rf node-configs
	rm -rf database
