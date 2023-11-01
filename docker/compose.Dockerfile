FROM rust:1.71

# Define a build-time argument called "node"
ARG node

# Set the working directory to /usr/src/myapp/artifacts
# Directory structure:
# artifacts
#   node0..4
#     config.toml
#     private_key.toml
#     executor
WORKDIR /usr/src/myapp/artifacts

# Copy the contents of the node0..4 artifacts directory from the "localenvc" image
# into the current working directory's
COPY --from=localenvc /usr/src/myapp/node/artifacts/$node/* $node/

# Make the executor binary in the node0..4 directory executable
RUN chmod +x $node/executor

# Expose public ports 3333, 3334, 3335, and 3336 where node tcp servers will be listening
EXPOSE $port

# Set the working directory to the directory specified by the "node" argument
# it will be node0..4
WORKDIR /usr/src/myapp/artifacts/$node/
RUN mkdir /usr/src/myapp/artifacts/$node/logs/

# You can ignore this command. In docker-compose.yml file we have specified the different command
CMD ["./executor", "0"]
