use serde_json::Value;

/// An error the engine reported for one request: the `{code, message, details}` of a failed response.
///
/// `code` is the engine's `ErrorCode` string (`NOT_FOUND`, `BAD_REQUEST`, `FORBIDDEN`, ...). It is kept as
/// text because the set grows with the protocol and a client has no reason to refuse a code it does not know.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct EngineError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl EngineError {
    pub(crate) fn from_wire(raw: &Value) -> Self {
        let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_owned);
        match (text("code"), text("message")) {
            (Some(code), Some(message)) => Self { code, message, details: raw.get("details").cloned() },
            _ => Self {
                code: "ENGINE_ERROR".to_owned(),
                message: "the engine returned an error without a code and a message".to_owned(),
                details: Some(raw.clone()),
            },
        }
    }
}

/// Why a call to the engine did not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The engine understood the request and refused or failed it.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The connection is gone: the engine exited, or this client was closed.
    #[error("the connection to the engine is closed")]
    Closed,
    #[error("i/o error talking to the engine: {0}")]
    Io(#[from] std::io::Error),
    /// The engine sent something that is not the wire format.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// A result arrived but not in the shape the caller asked for.
    #[error("unexpected result shape: {0}")]
    Decode(#[from] serde_json::Error),
}

pub type Result<T, E = ClientError> = std::result::Result<T, E>;
