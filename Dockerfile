# Build Stage
FROM rust:latest as builder
COPY /node/ /app/
WORKDIR /app
RUN apt-get update && apt-get install -y libclang-dev
RUN cargo build --release

# Binary copy stage
FROM scratch as executor-binary
COPY --from=builder /app/target/release/executor .

# Binary copy stage
FROM scratch as tester-binary
COPY --from=builder /app/target/release/tester .

# Executor runtime Stage
FROM debian:stable-slim as executor-runtime

COPY /node/tools/docker_binaries/executor /node/
COPY k8s_entrypoint.sh /node/

WORKDIR /node
RUN chmod +x k8s_entrypoint.sh

ENTRYPOINT ["./docker-entrypoint.sh"]

EXPOSE 3054
EXPOSE 3051

# Tester runtime Stage
FROM debian:stable-slim as tester-runtime
COPY node/tools/docker_binaries/tester /test/
COPY node/tests/tester_entrypoint.sh /test/
COPY node/tests/config.txt /test/

WORKDIR /test

RUN chmod +x tester_entrypoint.sh

