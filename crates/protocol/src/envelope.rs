//! The wire envelope: `{id, action, payload}` -> `{id, ok, data}` | `{id, ok:false, error}`, and the handshake.
//!
//! A handler that resolves, with any shape, is an envelope-level success; only a failed handler is `ok: false`.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::Value;

use crate::errors::RpcError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub payload: Value,
}

/// The `ok` member as a literal, so an untagged response picks the member the frame's `ok` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flag<const VALUE: bool>;

impl<const VALUE: bool> Serialize for Flag<VALUE> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(VALUE)
    }
}

impl<'de, const VALUE: bool> Deserialize<'de> for Flag<VALUE> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? == VALUE { Ok(Flag) } else { Err(D::Error::custom("unexpected `ok`")) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcResponse {
    Ok { id: String, ok: Flag<true>, data: Value },
    Err { id: String, ok: Flag<false>, error: RpcError },
}

impl RpcResponse {
    pub fn ok(id: impl Into<String>, data: Value) -> Self {
        Self::Ok { id: id.into(), ok: Flag, data }
    }

    pub fn err(id: impl Into<String>, error: RpcError) -> Self {
        Self::Err { id: id.into(), ok: Flag, error }
    }
}

/// The session opener. Bare (no `.`) because every command is namespaced; the transport intercepts it,
/// so it is never a registry command.
pub const HELLO_ACTION: &str = "hello";

/// Transport control actions; never commands.
pub const RESERVED_ACTIONS: [&str; 2] = ["subscribe", "unsubscribe"];

pub const CLIENT_NAMES: [&str; 3] = ["desktop-ui", "cli", "bifurc-extension"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientName {
    #[serde(rename = "desktop-ui")]
    DesktopUi,
    #[serde(rename = "cli")]
    Cli,
    #[serde(rename = "bifurc-extension")]
    BifurcExtension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "blob")]
    Blob,
    #[serde(rename = "stdio")]
    Stdio,
    #[serde(rename = "socket")]
    Socket,
    #[serde(rename = "ws")]
    Ws,
    #[serde(rename = "events.replay")]
    EventsReplay,
    #[serde(rename = "tls.install")]
    TlsInstall,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Blob => capability::BLOB,
            Capability::Stdio => capability::STDIO,
            Capability::Socket => capability::SOCKET,
            Capability::Ws => capability::WS,
            Capability::EventsReplay => capability::EVENTS_REPLAY,
            Capability::TlsInstall => capability::TLS_INSTALL,
        }
    }
}

/// The capability names as plain strings, for code that keeps them in a `Vec<&str>`.
pub mod capability {
    pub const BLOB: &str = "blob";
    pub const STDIO: &str = "stdio";
    pub const SOCKET: &str = "socket";
    pub const WS: &str = "ws";
    pub const EVENTS_REPLAY: &str = "events.replay";
    pub const TLS_INSTALL: &str = "tls.install";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloRequest {
    pub protocol_version: String,
    pub client_name: ClientName,
    pub client_version: String,
    /// The highest event `seq` the client has seen; absent on a first connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seq: Option<u64>,
    /// What the client says about itself, for the engine to show: the browser extension sends `browser`,
    /// `browserVersion`, `os`, `arch`, `extensionId`, `userAgent` and `language` (all strings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_info: Option<Value>,
    /// The named session of the command line (cookie jar, runtime variables, environment of its own); absent
    /// for the default session, which is the app's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloResponse {
    pub protocol_version: String,
    pub engine_version: String,
    pub capabilities: Vec<Capability>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_domain: Option<String>,
    /// Present only when the client sent `lastSeq` and the engine can serve it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed_from: Option<u64>,
    /// The client asked for a `lastSeq` the engine cannot serve; it must re-fetch state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resync_required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeRequest {
    pub events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsubscribeRequest {
    pub events: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;
    use serde_json::json;

    #[test]
    fn responses_round_trip_and_discriminate_on_ok() {
        let ok = json!({ "id": "1", "ok": true, "data": { "a": 1 } });
        let parsed: RpcResponse = serde_json::from_value(ok.clone()).unwrap();
        assert_eq!(parsed, RpcResponse::ok("1", json!({ "a": 1 })));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), ok);

        let failure = RpcResponse::err("2", RpcError::new(ErrorCode::NotFound, "gone", None));
        let wire = serde_json::to_value(&failure).unwrap();
        assert_eq!(wire["ok"], false);
        assert_eq!(wire["error"]["code"], "NOT_FOUND");
        assert_eq!(serde_json::from_value::<RpcResponse>(wire).unwrap(), failure);
        assert!(serde_json::from_value::<RpcResponse>(json!({ "id": "3", "ok": false, "data": 1 })).is_err());
    }

    #[test]
    fn request_payload_may_be_absent() {
        let request: RpcRequest = serde_json::from_value(json!({ "id": "1", "action": "config.get" })).unwrap();
        assert_eq!(request.payload, Value::Null);
    }

    #[test]
    fn hello_uses_camel_case_and_closed_enums() {
        let hello: HelloRequest = serde_json::from_value(
            json!({ "protocolVersion": "1.0.0", "clientName": "desktop-ui", "clientVersion": "0.4.0", "lastSeq": 0 }),
        )
        .unwrap();
        assert_eq!(hello.client_name, ClientName::DesktopUi);
        assert_eq!(hello.last_seq, Some(0));
        let bad = json!({ "protocolVersion": "1.0.0", "clientName": "nope", "clientVersion": "x" });
        assert!(serde_json::from_value::<HelloRequest>(bad).is_err());
        let negative = json!({ "protocolVersion": "1.0.0", "clientName": "cli", "clientVersion": "x", "lastSeq": -1 });
        assert!(serde_json::from_value::<HelloRequest>(negative).is_err());
    }

    #[test]
    fn hello_response_omits_absent_members() {
        let response = HelloResponse {
            protocol_version: "1.0.0".into(),
            engine_version: "0.4.0".into(),
            capabilities: vec![Capability::Socket, Capability::EventsReplay],
            session_id: "s".into(),
            host_domain: None,
            replayed_from: None,
            resync_required: None,
        };
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["capabilities"], json!(["socket", "events.replay"]));
        assert!(wire.get("hostDomain").is_none());
    }

    #[test]
    fn name_lists_match_the_enums() {
        for name in CLIENT_NAMES {
            assert!(serde_json::from_value::<ClientName>(json!(name)).is_ok(), "{name}");
        }
        assert_eq!(Capability::TlsInstall.as_str(), capability::TLS_INSTALL);
    }
}
