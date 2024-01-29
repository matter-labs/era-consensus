# Build Stage
FROM rust:latest as builder
COPY /node/ /node/
WORKDIR /node
RUN apt-get update && apt-get install -y libclang-dev
RUN cargo build --release

# Binary copy stage
FROM scratch as binary
COPY --from=builder /node/target/release/executor .

# Runtime Stage
FROM debian:stable-slim as runtime

COPY /node/tools/docker_binaries/executor /node/
COPY /node/tools/k8s_configs/ /node/k8s_config
COPY /node/tools/docker-config/ /node/docker_config
COPY docker-entrypoint.sh /node/
COPY k8s_entrypoint.sh /node/

WORKDIR /node
RUN chmod +x docker-entrypoint.sh
RUN chmod +x k8s_entrypoint.sh

ENTRYPOINT ["./docker-entrypoint.sh"]

EXPOSE 3054
EXPOSE 3051
