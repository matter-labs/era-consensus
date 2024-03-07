#!/bin/bash
# This file works as an entrypoint of the kubernetes cluster running the tester binary copied inside of it.

export RUST_LOG=INFO
./tester run
