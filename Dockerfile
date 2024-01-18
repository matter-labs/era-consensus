# Build Stage
FROM rust:latest as build
COPY /node/ /node/
COPY Makefile .
WORKDIR /node
RUN apt-get update && apt-get install -y libclang-dev
RUN cargo build --release
RUN cd .. && make docker_node_configs

# Runtime Stage
FROM debian:stable-slim as runtime

COPY --from=build /node/target/release/executor /node/
COPY --from=build /node/tools/docker-config/nodes-config /node/
COPY docker-entrypoint.sh /node/

WORKDIR /node
RUN chmod +x docker-entrypoint.sh

ENTRYPOINT ["./docker-entrypoint.sh"]

EXPOSE 3054
