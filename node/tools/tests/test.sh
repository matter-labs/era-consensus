cd ../../../ && make start_k8s_nodes NODES=1
cd node && cargo test sanity_test
