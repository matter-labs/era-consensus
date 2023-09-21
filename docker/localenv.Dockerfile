# Start from the official Rust image with version 1.71
# and give it the alias "localenvc"
FROM rust:1.71 as localenvc

ARG nodes

WORKDIR /usr/src/myapp

# Copy the current directory (.) into the working directory of the image
COPY . .

# Install
# clang package required for rocksdb
# cmake package required for capnp
RUN apt update
RUN apt install -y clang
RUN apt install -y cmake

# Build tools crate and create the config files
WORKDIR /usr/src/myapp/node
RUN cargo run -p tools --bin localnet_config -- --nodes=$nodes

# Build binary file in release mode and create a main release binary
WORKDIR /usr/src/myapp/node
RUN cargo build -p executor --release

# Create the artifacts directory 
WORKDIR /usr/src/myapp/node/
RUN i=0; \
    while [ $i -lt $nodes ]; do \
        mkdir -p "artifacts/node$i"; \
        i=$(expr $i + 1); \
    done

# Copy the binary file to the artifacts directory
RUN i=0; \
    while [ $i -lt $nodes ]; do \
        cp target/release/executor "artifacts/node$i/"; \
        i=$(expr $i + 1); \
    done

# Copy the config file to the artifacts directory
RUN i=0; \
    while [ $i -lt $nodes ]; do \
        cp ../configs/localnet/node$i/* "artifacts/node$i/"; \
        i=$(expr $i + 1); \
    done

# Check config files for each node
WORKDIR /usr/src/myapp/node/artifacts/
RUN i=0; \
    while [ $i -lt $nodes ]; do \
        chmod +x "node$i/executor"; \
        cd "node$i/"; \
        ./executor $i --verify-config; \
        cd "../"; \
        i=$(expr $i + 1); \
    done

# Some rundom command
CMD ["echo", "Done"]