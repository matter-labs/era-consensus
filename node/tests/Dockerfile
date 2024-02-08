FROM rust:latest as builder
COPY . /test/
WORKDIR /test
RUN apt-get update && apt-get install -y libclang-dev
RUN cargo build --release

FROM scratch as binary
COPY --from=builder test/target/release/tester .

FROM debian:stable-slim as runtime
COPY /tools/docker_binaries/tester /test/

WORKDIR /test
