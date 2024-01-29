#!/bin/bash
# This file works as an entrypoint of the kubernetes cluster running the node binary copied inside of it.

cd k8s_config/${NODE_ID}
export RUST_LOG=INFO
../../executor
