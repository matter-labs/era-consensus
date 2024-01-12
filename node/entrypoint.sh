#!/bin/bash

cd $(hostname -i):3054
export RUST_LOG=INFO
../executor
