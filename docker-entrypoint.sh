#!/bin/bash
# This file works as an entrypoint of the docker container running the node binary copied inside of it.

cd docker_config/${NODE_ID}
export RUST_LOG=INFO
../../executor
