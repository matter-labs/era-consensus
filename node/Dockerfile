# Build Stage
FROM rust:1.72.1 as build
COPY . /node/
WORKDIR /node
RUN apt-get update && apt-get install -y libclang-dev
RUN cargo build --release
RUN cd tools && make node_configs

# Runtime Stage
FROM debian:stable-slim as runtime

COPY --from=build /node/target/release/localnet_config /node/
COPY --from=build /node/target/release/executor /node/
COPY --from=build /node/tools/addresses.txt /node/
COPY --from=build /node/tools/node-configs /node/
COPY --from=build /node/entrypoint.sh /node/

WORKDIR /node
RUN chmod +x entrypoint.sh

ENTRYPOINT ["./entrypoint.sh"]

EXPOSE 3054
