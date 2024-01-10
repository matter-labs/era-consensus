# Build Stage
FROM rust:1.72.1 as build
COPY . /node/
WORKDIR /node
RUN apt-get update && apt-get install -y libclang-dev
RUN cargo build --release

# Runtime Stage
FROM debian:stable-slim as runtime

COPY --from=build /node/target/release/localnet_config /node/
COPY --from=build /node/target/release/executor /node/
COPY --from=build /node/tools/addresses.txt /node/

EXPOSE 3054
