//! The wire error: a code from a closed set, a message, and optional details.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    BadRequest,
    NotFound,
    Conflict,
    Unauthorized,
    Forbidden,
    UpstreamFailed,
    Timeout,
    Unsupported,
    UnknownCommand,
    EngineError,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 10] = [
        ErrorCode::BadRequest,
        ErrorCode::NotFound,
        ErrorCode::Conflict,
        ErrorCode::Unauthorized,
        ErrorCode::Forbidden,
        ErrorCode::UpstreamFailed,
        ErrorCode::Timeout,
        ErrorCode::Unsupported,
        ErrorCode::UnknownCommand,
        ErrorCode::EngineError,
    ];

    /// The code a wire string names.
    pub fn parse(code: &str) -> Option<ErrorCode> {
        Self::ALL.into_iter().find(|candidate| candidate.as_str() == code)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "BAD_REQUEST",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Conflict => "CONFLICT",
            ErrorCode::Unauthorized => "UNAUTHORIZED",
            ErrorCode::Forbidden => "FORBIDDEN",
            ErrorCode::UpstreamFailed => "UPSTREAM_FAILED",
            ErrorCode::Timeout => "TIMEOUT",
            ErrorCode::Unsupported => "UNSUPPORTED",
            ErrorCode::UnknownCommand => "UNKNOWN_COMMAND",
            ErrorCode::EngineError => "ENGINE_ERROR",
        }
    }

    /// Whether a client may retry a request that failed with this code.
    pub fn retryable(self) -> bool {
        matches!(self, ErrorCode::UpstreamFailed | ErrorCode::Timeout | ErrorCode::EngineError)
    }
}

/// The `error` member of a failed response frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    pub retryable: bool,
}

impl RpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>, details: Option<Value>) -> Self {
        Self { code, message: message.into(), details, retryable: code.retryable() }
    }
}

#[derive(Debug, Clone)]
pub struct EngineError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<Value>,
}

pub type Result<T> = std::result::Result<T, EngineError>;

impl EngineError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), details: None }
    }

    pub fn with_details(code: ErrorCode, message: impl Into<String>, details: Value) -> Self {
        Self { code, message: message.into(), details: Some(details) }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }

    pub fn upstream(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::UpstreamFailed, message)
    }

    /// The catch-all: what a thrown plain `Error` becomes on the wire.
    pub fn engine(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::EngineError, message)
    }

    /// The `error` member of a failed response frame.
    pub fn to_rpc(&self) -> Value {
        serde_json::to_value(RpcError::new(self.code, self.message.clone(), self.details.clone()))
            .expect("an RpcError is always JSON")
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineError {}

impl From<std::io::Error> for EngineError {
    fn from(err: std::io::Error) -> Self {
        Self::engine(err.to_string())
    }
}

impl From<serde_json::Error> for EngineError {
    fn from(err: serde_json::Error) -> Self {
        Self::engine(err.to_string())
    }
}

impl From<String> for EngineError {
    fn from(message: String) -> Self {
        Self::engine(message)
    }
}

impl From<&str> for EngineError {
    fn from(message: &str) -> Self {
        Self::engine(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codes_round_trip_by_wire_name() {
        for code in ErrorCode::ALL {
            assert_eq!(ErrorCode::parse(code.as_str()), Some(code));
            assert_eq!(serde_json::to_value(code).unwrap(), json!(code.as_str()));
        }
        assert_eq!(ErrorCode::parse("NOPE"), None);
    }

    #[test]
    fn only_transient_codes_retry() {
        let retryable: Vec<_> = ErrorCode::ALL.into_iter().filter(|c| c.retryable()).map(ErrorCode::as_str).collect();
        assert_eq!(retryable, ["UPSTREAM_FAILED", "TIMEOUT", "ENGINE_ERROR"]);
    }

    #[test]
    fn rpc_frame_orders_members_and_omits_absent_details() {
        let plain = EngineError::not_found("gone").to_rpc();
        assert_eq!(plain, json!({ "code": "NOT_FOUND", "message": "gone", "retryable": false }));
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let detailed = EngineError::with_details(ErrorCode::Timeout, "slow", json!({ "ms": 5 })).to_rpc();
        assert_eq!(keys(&detailed), ["code", "message", "details", "retryable"]);
        assert_eq!(detailed["retryable"], true);
    }
}
