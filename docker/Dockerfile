# syntax=docker/dockerfile:experimental
FROM rust:1.71-buster as builder

RUN apt update
RUN apt install -y clang

WORKDIR /usr/src/zksync-bft/
COPY . .

WORKDIR /usr/src/zksync-bft/node/
RUN cargo build --release --target-dir ./

FROM debian:buster-slim
EXPOSE 3333
EXPOSE 3334
EXPOSE 3335
EXPOSE 3336
COPY --from=builder --chmod=+x /usr/src/zksync-bft/node/release/executor /usr/bin/executor

ENV RUST_LOG=DEBUG

CMD ["/usr/bin/executor"]
