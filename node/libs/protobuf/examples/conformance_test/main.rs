//! Conformance test for our canonical encoding implemented according to
//! https://github.com/protocolbuffers/protobuf/blob/main/conformance/conformance.proto
//! Our implementation supports only a subset of proto functionality, so
//! `proto/conformance.proto` and
//! `proto/protobuf_test_messages.proto` contains only a
//! subset of original fields. Also we run only proto3 binary -> binary tests.
//! failure_list.txt contains tests which are expected to fail.

use self::proto::conformance_response::Result as ConformanceResult;
use anyhow::Context as _;
use prost::Message as _;
use prost_reflect::ReflectMessage;
use std::sync::Mutex;
use zksync_concurrency::{ctx, io};

mod proto;

/// Decodes a generated proto message from json for arbitrary ReflectMessage.
fn decode_json_proto<T: ReflectMessage + Default>(json: &str) -> anyhow::Result<T> {
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let proto: T = zksync_protobuf::serde::Deserialize{deny_unknown_fields:true}.proto(&mut deserializer)?;
    deserializer.end()?;
    Ok(proto)
}

/// Encodes a generated proto message to json for arbitrary ReflectMessage.
fn encode_json_proto<T: ReflectMessage>(proto: &T) -> String {
    let mut serializer = serde_json::Serializer::pretty(vec![]);
    zksync_protobuf::serde::Serialize{}.proto(proto, &mut serializer).unwrap();
    String::from_utf8(serializer.into_inner()).unwrap()
}

impl ConformanceResult {
    /// Creates a "skipped" result with the specified message.
    fn skipped(reason: &str) -> Self {
        Self::Skipped(reason.into())
    }

    /// Creates a "parse error" result.
    fn parse_error() -> Self {
        Self::ParseError("parsing failed".into())
    }
}

/// Processes a single conformance request.
fn process_request(req: proto::ConformanceRequest) -> anyhow::Result<ConformanceResult> {
    let message_type = req.message_type.context("missing message_type")?;
    if message_type != "protobuf_test_messages.proto3.TestAllTypesProto3" {
        return Ok(ConformanceResult::skipped("unsupported"));
    }

    // Decode.
    let payload = req.payload.context("missing payload")?;
    let payload = match payload {
        proto::conformance_request::Payload::JsonPayload(payload) => {
            match decode_json_proto(&payload) {
                Ok(payload) => payload,
                Err(_) => return Ok(ConformanceResult::skipped("unsupported fields")),
            }
        }
        proto::conformance_request::Payload::ProtobufPayload(payload) => {
            // First filter out incorrect encodings.
            let Ok(parsed) = proto::TestAllTypesProto3::decode(&payload[..]) else {
                return Ok(ConformanceResult::parse_error());
            };
            // Then check if there are any unknown fields in the original payload.
            if zksync_protobuf::canonical_raw(&payload[..], &parsed.descriptor()).is_err() {
                return Ok(ConformanceResult::skipped("unsupported fields"));
            }
            parsed
        }
        _ => return Ok(ConformanceResult::skipped("unsupported input format")),
    };

    // Encode.
    let format = req
        .requested_output_format
        .context("missing output format")?;
    match proto::WireFormat::try_from(format).context("unknown format")? {
        proto::WireFormat::Json => Ok(ConformanceResult::JsonPayload(encode_json_proto(&payload))),
        proto::WireFormat::Protobuf => {
            // Re-encode the parsed proto.
            let encoded =
                zksync_protobuf::canonical_raw(&payload.encode_to_vec(), &payload.descriptor())?;
            Ok(ConformanceResult::ProtobufPayload(encoded))
        }
        _ => Ok(ConformanceResult::skipped("unsupported output format")),
    }
}

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

        let req = proto::ConformanceRequest::decode(&msg[..])?;
        let res = process_request(req)?;
        let resp = proto::ConformanceResponse { result: Some(res) };

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
