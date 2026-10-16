//! The Socket.IO wire format (protocol v5, carried by Engine.IO v4 text frames over a WebSocket).
//!
//! An Engine.IO frame is one digit (open, close, ping, pong, message, ...) and a payload. A `message` frame
//! carries a Socket.IO packet: `<type>[<attachments>-][<namespace>,][<ack id>][<json>]`. Nothing here touches
//! the network: [`read_frame`] says what a frame the server sent means, the `*_frame` functions build the
//! frames this side sends.

use std::time::Duration;

use serde_json::{Value, json};

/// The heartbeat a server is assumed to keep until its open packet says otherwise: `pingInterval + pingTimeout`
/// of a default Socket.IO server.
pub const DEFAULT_HEARTBEAT: Duration = Duration::from_millis(45_000);

/// The Engine.IO pong, the answer to a server's ping.
pub const PONG: &str = "3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketType {
    Connect,
    Disconnect,
    Event,
    Ack,
    ConnectError,
    BinaryEvent,
    BinaryAck,
}

impl PacketType {
    fn from_digit(digit: char) -> Option<Self> {
        Some(match digit {
            '0' => PacketType::Connect,
            '1' => PacketType::Disconnect,
            '2' => PacketType::Event,
            '3' => PacketType::Ack,
            '4' => PacketType::ConnectError,
            '5' => PacketType::BinaryEvent,
            '6' => PacketType::BinaryAck,
            _ => return None,
        })
    }

    fn digit(self) -> char {
        match self {
            PacketType::Connect => '0',
            PacketType::Disconnect => '1',
            PacketType::Event => '2',
            PacketType::Ack => '3',
            PacketType::ConnectError => '4',
            PacketType::BinaryEvent => '5',
            PacketType::BinaryAck => '6',
        }
    }
}

/// One Socket.IO packet.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub kind: PacketType,
    /// `/` is the main namespace, which the wire leaves out.
    pub namespace: String,
    /// The acknowledgement id: on an event it asks for an ack, on an ack it names the event answered.
    pub id: Option<u64>,
    pub data: Option<Value>,
}

/// The text of a packet, without the Engine.IO digit in front.
pub fn encode(packet: &Packet) -> String {
    let mut out = String::new();
    out.push(packet.kind.digit());
    if packet.namespace != "/" {
        out.push_str(&packet.namespace);
        out.push(',');
    }
    if let Some(id) = packet.id {
        out.push_str(&id.to_string());
    }
    if let Some(data) = &packet.data {
        out.push_str(&data.to_string());
    }
    out
}

/// Read a packet from the text after the Engine.IO digit.
pub fn decode(text: &str) -> Result<Packet, String> {
    let mut chars = text.chars();
    let kind = chars.next().and_then(PacketType::from_digit).ok_or_else(|| format!("Unknown packet \"{text}\""))?;
    let mut rest = chars.as_str();

    // A binary packet says how many binary frames follow it: `<n>-`.
    if matches!(kind, PacketType::BinaryEvent | PacketType::BinaryAck) {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        match rest[digits..].strip_prefix('-') {
            Some(after) if digits > 0 => rest = after,
            _ => return Err(format!("A binary packet without its attachment count: \"{text}\"")),
        }
    }

    let mut namespace = "/".to_string();
    if rest.starts_with('/') {
        let end = rest.find(',').unwrap_or(rest.len());
        namespace = rest[..end].to_string();
        rest = rest.get(end + 1..).unwrap_or("");
    }

    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let id = match digits {
        0 => None,
        _ => Some(rest[..digits].parse::<u64>().map_err(|_| format!("Invalid ack id in \"{text}\""))?),
    };
    rest = &rest[digits..];

    let data = match rest {
        "" => None,
        json => Some(serde_json::from_str(json).map_err(|_| format!("Invalid JSON in packet \"{text}\""))?),
    };
    Ok(Packet { kind, namespace, id, data })
}

/// What a frame the server sent means to a client connected (or connecting) to `namespace`.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// The Engine.IO open packet: how long the server may stay silent before it is given up on.
    Open { heartbeat: Duration },
    /// The server's ping, to be answered with [`PONG`].
    Ping,
    /// The namespace accepted the connection.
    Connected { sid: Option<String> },
    /// The namespace refused the connection, with the server's message.
    Refused(String),
    /// An event, with the id to acknowledge it by when the server asked for an ack.
    Event { name: String, args: Vec<Value>, ack_id: Option<u64> },
    /// The server's acknowledgement of the event emitted with this id.
    Ack { id: u64, args: Vec<Value> },
    /// The server disconnected this client from the namespace.
    Disconnected,
    /// The Engine.IO close packet: the whole connection is over.
    Closed,
    /// Nothing this side acts on: another namespace's packet, a pong, a noop, a frame that cannot be read.
    Ignored,
}

pub fn read_frame(frame: &str, namespace: &str) -> Incoming {
    let mut chars = frame.chars();
    match chars.next() {
        Some('0') => {
            let open: Value = serde_json::from_str(chars.as_str()).unwrap_or(Value::Null);
            let millis = |key: &str| open.get(key).and_then(Value::as_u64);
            let heartbeat = match (millis("pingInterval"), millis("pingTimeout")) {
                (Some(interval), Some(timeout)) => Duration::from_millis(interval.saturating_add(timeout)),
                _ => DEFAULT_HEARTBEAT,
            };
            Incoming::Open { heartbeat }
        }
        Some('1') => Incoming::Closed,
        Some('2') => Incoming::Ping,
        Some('4') => match decode(chars.as_str()) {
            Ok(packet) if packet.namespace == namespace => read_packet(packet),
            _ => Incoming::Ignored,
        },
        _ => Incoming::Ignored,
    }
}

fn read_packet(packet: Packet) -> Incoming {
    let items = |data: Option<Value>| match data {
        Some(Value::Array(items)) => items,
        Some(other) => vec![other],
        None => Vec::new(),
    };
    match packet.kind {
        PacketType::Connect => {
            let sid = packet.data.as_ref().and_then(|d| d.get("sid")).and_then(Value::as_str).map(str::to_string);
            Incoming::Connected { sid }
        }
        PacketType::ConnectError => Incoming::Refused(match &packet.data {
            Some(data) => match data.get("message").and_then(Value::as_str) {
                Some(message) => message.to_string(),
                None => data.as_str().map(str::to_string).unwrap_or_else(|| data.to_string()),
            },
            None => "The server refused the connection.".to_string(),
        }),
        PacketType::Disconnect => Incoming::Disconnected,
        PacketType::Event | PacketType::BinaryEvent => {
            let mut args = items(packet.data);
            if args.is_empty() {
                return Incoming::Ignored;
            }
            let name = match args.remove(0) {
                Value::String(name) => name,
                other => other.to_string(),
            };
            Incoming::Event { name, args, ack_id: packet.id }
        }
        PacketType::Ack | PacketType::BinaryAck => match packet.id {
            Some(id) => Incoming::Ack { id, args: items(packet.data) },
            None => Incoming::Ignored,
        },
    }
}

/// A namespace as the wire names it: `/` when none is given, always with its leading slash.
pub fn normalize_namespace(namespace: &str) -> String {
    let namespace = namespace.trim();
    match namespace {
        "" | "/" => "/".to_string(),
        named if named.starts_with('/') => named.to_string(),
        named => format!("/{named}"),
    }
}

/// The arguments an emit carries, from the text the user wrote: a JSON array is the list of arguments, any
/// other JSON value is the one argument, text that is not JSON is sent as a string, and nothing is no argument.
pub fn parse_args(text: &str) -> Vec<Value> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => items,
        Ok(value) => vec![value],
        Err(_) => vec![json!(text)],
    }
}

/// The frame that asks `namespace` for a connection, with the `auth` payload when there is one.
pub fn connect_frame(namespace: &str, auth: Option<&Value>) -> String {
    let packet = Packet { kind: PacketType::Connect, namespace: namespace.to_string(), id: None, data: auth.cloned() };
    format!("4{}", encode(&packet))
}

/// The frame that leaves `namespace`.
pub fn disconnect_frame(namespace: &str) -> String {
    let packet = Packet { kind: PacketType::Disconnect, namespace: namespace.to_string(), id: None, data: None };
    format!("4{}", encode(&packet))
}

/// The frame that emits `event` with `args`; `ack_id` asks the server to acknowledge it.
pub fn event_frame(namespace: &str, event: &str, args: &[Value], ack_id: Option<u64>) -> String {
    let mut data = vec![json!(event)];
    data.extend(args.iter().cloned());
    let packet = Packet {
        kind: PacketType::Event,
        namespace: namespace.to_string(),
        id: ack_id,
        data: Some(Value::Array(data)),
    };
    format!("4{}", encode(&packet))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(kind: PacketType, namespace: &str, id: Option<u64>, data: Option<Value>) -> Packet {
        Packet { kind, namespace: namespace.to_string(), id, data }
    }

    #[test]
    fn packets_round_trip_through_the_wire_form() {
        for (text, expected) in [
            ("0", packet(PacketType::Connect, "/", None, None)),
            ("0{\"token\":\"abc\"}", packet(PacketType::Connect, "/", None, Some(json!({ "token": "abc" })))),
            ("0/admin,{\"sid\":\"x\"}", packet(PacketType::Connect, "/admin", None, Some(json!({ "sid": "x" })))),
            ("1/admin,", packet(PacketType::Disconnect, "/admin", None, None)),
            ("2[\"hello\",1]", packet(PacketType::Event, "/", None, Some(json!(["hello", 1])))),
            ("2/admin,12[\"hello\"]", packet(PacketType::Event, "/admin", Some(12), Some(json!(["hello"])))),
            ("37[\"ok\"]", packet(PacketType::Ack, "/", Some(7), Some(json!(["ok"])))),
            ("4{\"message\":\"no\"}", packet(PacketType::ConnectError, "/", None, Some(json!({ "message": "no" })))),
        ] {
            assert_eq!(decode(text).unwrap(), expected, "{text}");
            assert_eq!(encode(&expected), text);
        }
    }

    #[test]
    fn a_binary_packet_is_read_without_its_attachment_count() {
        let event = decode("51-[\"file\",{\"_placeholder\":true,\"num\":0}]").unwrap();
        assert_eq!(event.kind, PacketType::BinaryEvent);
        assert_eq!(event.data, Some(json!(["file", { "_placeholder": true, "num": 0 }])));
        let ack = decode("62-/admin,3[{\"_placeholder\":true,\"num\":0},{\"_placeholder\":true,\"num\":1}]").unwrap();
        assert_eq!((ack.kind, ack.namespace.as_str(), ack.id), (PacketType::BinaryAck, "/admin", Some(3)));
        assert!(decode("5[\"file\"]").is_err());
    }

    #[test]
    fn a_namespace_without_its_comma_and_text_that_is_no_packet_are_handled() {
        assert_eq!(decode("1/admin").unwrap(), packet(PacketType::Disconnect, "/admin", None, None));
        for bad in ["", "9", "x[]", "2[\"unterminated\"", "2not json"] {
            assert!(decode(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_open_packet_gives_the_heartbeat() {
        let open = "0{\"sid\":\"abc\",\"upgrades\":[],\"pingInterval\":300,\"pingTimeout\":200,\"maxPayload\":1000000}";
        assert_eq!(read_frame(open, "/"), Incoming::Open { heartbeat: Duration::from_millis(500) });
        assert_eq!(read_frame("0{}", "/"), Incoming::Open { heartbeat: DEFAULT_HEARTBEAT });
        assert_eq!(read_frame("0", "/"), Incoming::Open { heartbeat: DEFAULT_HEARTBEAT });
    }

    #[test]
    fn engine_io_frames_are_told_apart() {
        assert_eq!(read_frame("2", "/"), Incoming::Ping);
        assert_eq!(read_frame("2probe", "/"), Incoming::Ping);
        assert_eq!(read_frame("1", "/"), Incoming::Closed);
        for ignored in ["3", "6", "5", "", "4", "4nonsense", "hello"] {
            assert_eq!(read_frame(ignored, "/"), Incoming::Ignored, "{ignored}");
        }
    }

    #[test]
    fn a_namespace_answers_a_connect_with_a_sid_or_a_refusal() {
        assert_eq!(read_frame("40{\"sid\":\"s1\"}", "/"), Incoming::Connected { sid: Some("s1".into()) });
        assert_eq!(read_frame("40/chat,{\"sid\":\"s2\"}", "/chat"), Incoming::Connected { sid: Some("s2".into()) });
        assert_eq!(read_frame("40", "/"), Incoming::Connected { sid: None });
        assert_eq!(read_frame("44{\"message\":\"Not authorized\"}", "/"), Incoming::Refused("Not authorized".into()));
        assert_eq!(read_frame("44/chat,\"Invalid namespace\"", "/chat"), Incoming::Refused("Invalid namespace".into()));
        assert_eq!(read_frame("44", "/"), Incoming::Refused("The server refused the connection.".into()));
    }

    #[test]
    fn events_and_acks_carry_their_arguments() {
        assert_eq!(
            read_frame("42[\"chat\",\"hi\",{\"n\":1}]", "/"),
            Incoming::Event { name: "chat".into(), args: vec![json!("hi"), json!({ "n": 1 })], ack_id: None }
        );
        assert_eq!(
            read_frame("42/chat,5[\"ask\"]", "/chat"),
            Incoming::Event { name: "ask".into(), args: vec![], ack_id: Some(5) }
        );
        assert_eq!(
            read_frame("451-[\"file\",{\"_placeholder\":true,\"num\":0}]", "/"),
            Incoming::Event {
                name: "file".into(),
                args: vec![json!({ "_placeholder": true, "num": 0 })],
                ack_id: None
            }
        );
        assert_eq!(read_frame("433[\"ok\",2]", "/"), Incoming::Ack { id: 3, args: vec![json!("ok"), json!(2)] });
        assert_eq!(read_frame("43/chat,3", "/chat"), Incoming::Ack { id: 3, args: vec![] });
        assert_eq!(read_frame("41", "/"), Incoming::Disconnected);
        assert_eq!(read_frame("41/chat,", "/chat"), Incoming::Disconnected);
        // An event without a name and an ack without an id are not acted on.
        assert_eq!(read_frame("42[]", "/"), Incoming::Ignored);
        assert_eq!(read_frame("43[\"ok\"]", "/"), Incoming::Ignored);
    }

    #[test]
    fn another_namespaces_packets_are_ignored() {
        assert_eq!(read_frame("42/other,[\"chat\"]", "/"), Incoming::Ignored);
        assert_eq!(read_frame("42[\"chat\"]", "/chat"), Incoming::Ignored);
        assert_eq!(read_frame("41/other,", "/chat"), Incoming::Ignored);
    }

    #[test]
    fn a_namespace_always_has_its_leading_slash() {
        for (given, expected) in [("", "/"), ("  ", "/"), ("/", "/"), ("chat", "/chat"), (" /chat ", "/chat")] {
            assert_eq!(normalize_namespace(given), expected, "{given:?}");
        }
    }

    #[test]
    fn the_arguments_of_an_emit_come_from_what_was_written() {
        assert_eq!(parse_args(""), Vec::<Value>::new());
        assert_eq!(parse_args("  "), Vec::<Value>::new());
        assert_eq!(parse_args("[\"a\", 1]"), vec![json!("a"), json!(1)]);
        assert_eq!(parse_args("[[1, 2]]"), vec![json!([1, 2])]);
        assert_eq!(parse_args("{\"a\": 1}"), vec![json!({ "a": 1 })]);
        assert_eq!(parse_args("42"), vec![json!(42)]);
        assert_eq!(parse_args("\"quoted\""), vec![json!("quoted")]);
        assert_eq!(parse_args("hello there"), vec![json!("hello there")]);
    }

    #[test]
    fn the_frames_this_side_sends() {
        assert_eq!(connect_frame("/", None), "40");
        assert_eq!(connect_frame("/", Some(&json!({ "token": "abc" }))), "40{\"token\":\"abc\"}");
        assert_eq!(connect_frame("/chat", None), "40/chat,");
        assert_eq!(connect_frame("/chat", Some(&json!({ "t": 1 }))), "40/chat,{\"t\":1}");
        assert_eq!(disconnect_frame("/"), "41");
        assert_eq!(disconnect_frame("/chat"), "41/chat,");
        assert_eq!(event_frame("/", "hello", &[], None), "42[\"hello\"]");
        assert_eq!(event_frame("/", "hello", &[json!("a"), json!({ "n": 1 })], None), "42[\"hello\",\"a\",{\"n\":1}]");
        assert_eq!(event_frame("/chat", "ask", &[json!(1)], Some(7)), "42/chat,7[\"ask\",1]");
    }
}
