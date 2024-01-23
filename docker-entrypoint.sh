#!/bin/bash
# This file works as an entrypoint of the docker container running the node binary copied inside of it.

cd $(hostname -i):3054
export RUST_LOG=INFO
../executor
