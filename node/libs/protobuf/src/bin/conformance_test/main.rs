//! Conformance test for our canonical encoding implemented according to
//! https://github.com/protocolbuffers/zksync_protobuf/blob/main/conformance/conformance.proto
//! Our implementation supports only a subset of proto functionality, so
//! `proto/conformance.proto` and
//! `proto/protobuf_test_messages.proto` contains only a
//! subset of original fields. Also we run only proto3 binary -> binary tests.
//! failure_list.txt contains tests which are expected to fail.
use anyhow::Context as _;
use concurrency::{ctx, io};
use prost::Message as _;
use prost_reflect::ReflectMessage;
use std::sync::Mutex;

mod proto;
use proto::zksync::protobuf::conformance_test as proto_;

/// Runs the test server.
async fn run() -> anyhow::Result<()> {
    let ctx = &ctx::root();
    let stdin = &mut tokio::io::stdin();
    let stdout = &mut tokio::io::stdout();
    loop {
        // Read the request.
        let mut msg_size = [0u8; 4];
        if io::read_exact(ctx, stdin, &mut msg_size).await?.is_err() {
            return Ok(());
        }
        let msg_size = u32::from_le_bytes(msg_size);
        let mut msg = vec![0u8; msg_size as usize];
        io::read_exact(ctx, stdin, &mut msg[..]).await??;

        use proto_::conformance_response::Result as R;
        let req = proto_::ConformanceRequest::decode(&msg[..])?;
        let res = async {
            let t = req.message_type.context("missing message_type")?;
            if t != *"protobuf_test_messages.proto3.TestAllTypesProto3" {
                return Ok(R::Skipped("unsupported".to_string()));
            }

            // Decode.
            let payload = req.payload.context("missing payload")?;
            use proto_::TestAllTypesProto3 as T;
            let p = match payload {
                proto_::conformance_request::Payload::JsonPayload(payload) => {
                    match zksync_protobuf::decode_json_proto(&payload) {
                        Ok(p) => p,
                        Err(_) => return Ok(R::Skipped("unsupported fields".to_string())),
                    }
                }
                proto_::conformance_request::Payload::ProtobufPayload(payload) => {
                    // First filter out incorrect encodings.
                    let Ok(p) = T::decode(&payload[..]) else {
                        return Ok(R::ParseError("parsing failed".to_string()));
                    };
                    // Then check if there are any unknown fields in the original payload.
                    if zksync_protobuf::canonical_raw(&payload[..], &p.descriptor()).is_err() {
                        return Ok(R::Skipped("unsupported fields".to_string()));
                    }
                    p
                }
                _ => return Ok(R::Skipped("unsupported input format".to_string())),
            };

            // Encode.
            let format = req
                .requested_output_format
                .context("missing output format")?;
            match proto_::WireFormat::try_from(format).context("unknown format")? {
                proto_::WireFormat::Json => {
                    anyhow::Ok(R::JsonPayload(zksync_protobuf::encode_json_proto(&p)))
                }
                proto_::WireFormat::Protobuf => {
                    // Reencode the parsed proto.
                    anyhow::Ok(R::ProtobufPayload(zksync_protobuf::canonical_raw(
                        &p.encode_to_vec(),
                        &p.descriptor(),
                    )?))
                }
                _ => Ok(R::Skipped("unsupported output format".to_string())),
            }
        }
        .await?;
        let resp = proto_::ConformanceResponse { result: Some(res) };

        // Write the response.
        let msg = resp.encode_to_vec();
        io::write_all(ctx, stdout, &u32::to_le_bytes(msg.len() as u32)).await??;
        io::write_all(ctx, stdout, &msg).await??;
        io::flush(ctx, stdout).await??;
    }
}

#[tokio::main]
async fn main() {
    let sub = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env());
    match std::env::var("LOG_FILE") {
        Err(_) => sub.with_writer(std::io::stderr).init(),
        Ok(path) => sub
            .with_writer(Mutex::new(
                std::fs::File::options()
                    .create(true)
                    .append(true)
                    .open(path)
                    .unwrap(),
            ))
            .init(),
    };
    if let Err(err) = run().await {
        tracing::error!("run(): {err:#}");
    }
}
