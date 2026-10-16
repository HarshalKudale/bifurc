//! Making a call: connecting, the codec that turns JSON into wire messages and back, and running a method
//! of any streaming kind.
//!
//! Nothing here is generated from a schema. A method is a `MethodDescriptor` found in a pool (from reflection
//! or a compiled `.proto`), and its messages are `DynamicMessage`s.

use std::{collections::BTreeMap, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use prost::Message;
use prost_reflect::{DynamicMessage, MessageDescriptor, MethodDescriptor, SerializeOptions};
use serde_json::Value;
use tonic::{
    Code, Request, Status,
    client::Grpc,
    codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder},
    metadata::{
        AsciiMetadataKey, AsciiMetadataValue, BinaryMetadataKey, BinaryMetadataValue, KeyAndValueRef, MetadataMap,
    },
    transport::{Channel, ClientTlsConfig, Endpoint},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Opens a channel to `address`. TLS is used for `https://` / `grpcs://`, for a bare host and for port 443;
/// anything else is plaintext HTTP/2, which is how local development servers are run.
pub async fn connect(address: &str) -> Result<Channel, String> {
    let address = address.trim();
    if address.is_empty() {
        return Err("Enter the server address.".to_owned());
    }
    let (scheme, rest) = match address.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => (String::new(), address),
    };
    let rest = rest.trim_end_matches('/');
    let port = rest.rsplit_once(':').filter(|(host, _)| !host.is_empty() && !host.ends_with(':')).map(|(_, port)| port);
    let explicit = match scheme.as_str() {
        "https" | "grpcs" => Some(true),
        "http" | "grpc" => Some(false),
        _ => None,
    };
    // Without a scheme a bare host or port 443 is tried as TLS first and, if the server does not speak TLS
    // there, as plaintext HTTP/2 (some public servers are cleartext on 443). Any other port is plaintext.
    let (first, second) = match explicit {
        Some(tls) => (tls, None),
        None if port.is_none_or(|port| port == "443") => (true, Some(false)),
        None => (false, None),
    };
    match open(rest, first).await {
        Ok(channel) => Ok(channel),
        Err(error) => match second {
            Some(tls) => {
                // The port a bare host was tried on as TLS is the one it is tried on in plaintext.
                let hostport = if port.is_some() { rest.to_owned() } else { format!("{rest}:443") };
                open(&hostport, tls).await.map_err(|_| format!("Could not connect to {address}: {error}"))
            }
            None => Err(format!("Could not connect to {address}: {error}")),
        },
    }
}

async fn open(hostport: &str, tls: bool) -> Result<Channel, String> {
    let uri = format!("{}://{hostport}", if tls { "https" } else { "http" });
    let mut endpoint = Endpoint::from_shared(uri).map_err(|e| format!("not a valid address: {e}"))?;
    endpoint = endpoint.connect_timeout(CONNECT_TIMEOUT).tcp_nodelay(true);
    if tls {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_native_roots())
            .map_err(|e| format!("Could not set up TLS: {e}"))?;
    }
    endpoint.connect().await.map_err(|e| chain(&e))
}

fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

/// Encodes any dynamic message and decodes messages of the one type it is given: the response type for a
/// client, the request type for a server.
pub struct DynamicCodec(pub MessageDescriptor);

pub struct DynamicEncoder;
pub struct DynamicDecoder(MessageDescriptor);

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> DynamicEncoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> DynamicDecoder {
        DynamicDecoder(self.0.clone())
    }
}

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: DynamicMessage, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        item.encode(dst).map_err(|e| Status::internal(format!("Could not encode the message: {e}")))
    }
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<DynamicMessage>, Status> {
        DynamicMessage::decode(self.0.clone(), src)
            .map(Some)
            .map_err(|e| Status::internal(format!("Could not decode the response: {e}")))
    }
}

/// The outcome of a call, whatever way it ended.
pub struct CallOutcome {
    pub responses: Vec<String>,
    pub metadata: BTreeMap<String, String>,
    pub status: Code,
    pub status_message: String,
}

/// Parses the request body into the messages to send. A method that streams from the client takes a JSON
/// array, one message per element (a single object is one message); any other takes one object.
pub fn parse_messages(body: &str, method: &MethodDescriptor) -> Result<Vec<DynamicMessage>, String> {
    let body = body.trim();
    let body = if body.is_empty() { "{}" } else { body };
    let value: Value = serde_json::from_str(body).map_err(|e| format!("The message is not valid JSON: {e}"))?;
    let items = match value {
        Value::Array(items) if method.is_client_streaming() => items,
        other => vec![other],
    };
    items
        .into_iter()
        .enumerate()
        .map(|(ix, item)| {
            DynamicMessage::deserialize(method.input(), item).map_err(|e| {
                if method.is_client_streaming() {
                    format!("Message {} does not match {}: {e}", ix + 1, method.input().full_name())
                } else {
                    format!("The message does not match {}: {e}", method.input().full_name())
                }
            })
        })
        .collect()
}

pub fn to_json(message: &DynamicMessage) -> String {
    let options = SerializeOptions::new().skip_default_fields(false);
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b"  "));
    match message.serialize_with_options(&mut serializer, &options) {
        Ok(()) => String::from_utf8(out).unwrap_or_default(),
        Err(_) => "{}".to_owned(),
    }
}

/// Request metadata as the wire wants it: lower-case names, and `-bin` names carrying base64 values.
pub fn to_metadata(pairs: &[(String, String)]) -> Result<MetadataMap, String> {
    let mut map = MetadataMap::new();
    for (name, value) in pairs {
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || name.starts_with(':') || name.starts_with("grpc-") {
            continue;
        }
        if name.ends_with("-bin") {
            let key = BinaryMetadataKey::from_bytes(name.as_bytes())
                .map_err(|_| format!("\"{name}\" is not a valid metadata name."))?;
            let bytes = STANDARD
                .decode(value.trim())
                .map_err(|_| format!("The value of \"{name}\" must be base64: names ending in -bin carry bytes."))?;
            map.insert_bin(key, BinaryMetadataValue::from_bytes(&bytes));
        } else {
            let key = AsciiMetadataKey::from_bytes(name.as_bytes())
                .map_err(|_| format!("\"{name}\" is not a valid metadata name."))?;
            let value = AsciiMetadataValue::try_from(value.trim())
                .map_err(|_| format!("The value of \"{name}\" is not valid metadata (printable ASCII only)."))?;
            map.insert(key, value);
        }
    }
    Ok(map)
}

/// Transport bookkeeping that every response carries and nobody wants to read.
const TRANSPORT_KEYS: [&str; 5] = ["content-type", "grpc-status", "grpc-message", "grpc-encoding", "grpc-accept-encoding"];

pub fn collect_metadata(into: &mut BTreeMap<String, String>, from: &MetadataMap) {
    for entry in from.iter() {
        let name = match &entry {
            KeyAndValueRef::Ascii(key, _) => key.as_str(),
            KeyAndValueRef::Binary(key, _) => key.as_str(),
        };
        if TRANSPORT_KEYS.contains(&name) {
            continue;
        }
        match entry {
            KeyAndValueRef::Ascii(key, value) => {
                into.insert(key.as_str().to_owned(), String::from_utf8_lossy(value.as_encoded_bytes()).into_owned());
            }
            KeyAndValueRef::Binary(key, value) => {
                if let Ok(bytes) = value.to_bytes() {
                    into.insert(key.as_str().to_owned(), STANDARD.encode(bytes));
                }
            }
        }
    }
}

/// Runs the method with the given messages and gathers everything it answers.
pub async fn invoke(
    channel: Channel,
    method: &MethodDescriptor,
    messages: Vec<DynamicMessage>,
    metadata: MetadataMap,
) -> CallOutcome {
    let mut outcome =
        CallOutcome { responses: Vec::new(), metadata: BTreeMap::new(), status: Code::Ok, status_message: String::new() };
    let run = run(channel, method, messages, metadata, &mut outcome);
    match tokio::time::timeout(CALL_TIMEOUT, run).await {
        Ok(Ok(())) => {
            outcome.status = Code::Ok;
            outcome.status_message = "OK".to_owned();
        }
        Ok(Err(status)) => {
            collect_metadata(&mut outcome.metadata, status.metadata());
            outcome.status = status.code();
            outcome.status_message = status.message().to_owned();
        }
        Err(_) => {
            outcome.status = Code::DeadlineExceeded;
            outcome.status_message = format!("No complete answer within {} seconds.", CALL_TIMEOUT.as_secs());
        }
    }
    outcome
}

async fn run(
    channel: Channel,
    method: &MethodDescriptor,
    messages: Vec<DynamicMessage>,
    metadata: MetadataMap,
    outcome: &mut CallOutcome,
) -> Result<(), Status> {
    let service = method.parent_service();
    let path = format!("/{}/{}", service.full_name(), method.name());
    let path = path.parse().map_err(|_| Status::invalid_argument(format!("\"{path}\" is not a valid method path.")))?;
    let mut grpc = Grpc::new(channel);
    grpc.ready().await.map_err(|e| Status::unavailable(format!("The server is not reachable: {}", chain(&e))))?;
    // Every method kind is sent as a stream of messages: on the wire a unary call is a stream of exactly one.
    let mut request = Request::new(futures::stream::iter(messages));
    *request.metadata_mut() = metadata;
    let response = grpc.streaming(request, path, DynamicCodec(method.output())).await?;
    collect_metadata(&mut outcome.metadata, response.metadata());
    let mut inbound = response.into_inner();
    while let Some(message) = inbound.message().await? {
        outcome.responses.push(to_json(&message));
    }
    if let Some(trailers) = inbound.trailers().await? {
        collect_metadata(&mut outcome.metadata, &trailers);
    }
    Ok(())
}
