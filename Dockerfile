# Build Stage
FROM rust:1.72.1 as build
COPY /node/ Makefile /node/
WORKDIR /node
RUN apt-get update && apt-get install -y libclang-dev
RUN cargo build --release
RUN make docker_node_configs

# Runtime Stage
FROM debian:stable-slim as runtime

COPY --from=build /node/target/release/executor /node/
COPY --from=build /node/tools/docker-config/node-configs /node/
COPY entrypoint.sh /node/

WORKDIR /node
RUN chmod +x entrypoint.sh

ENTRYPOINT ["./entrypoint.sh"]

EXPOSE 3054
