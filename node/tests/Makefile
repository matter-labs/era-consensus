.PHONY: start_k8s_tests 

docker_test_image:
	docker build -t test-suite --target=tester-runtime -f ../../Dockerfile ../../

docker_build_tester:
	docker build --output=../tools/docker_binaries --target=tester-binary -f ../../Dockerfile ../../

start_k8s_tests:
	$(MAKE) docker_test_image
	minikube image load test-suite:latest
	cargo run --bin tester start-pod
