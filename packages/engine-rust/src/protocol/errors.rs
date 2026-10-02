//! The wire error: a code from a closed set, a message, and optional details.

use std::fmt;

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        let mut out = json!({ "code": self.code.as_str(), "message": self.message });
        if let Some(details) = &self.details {
            out["details"] = details.clone();
        }
        out["retryable"] = Value::Bool(self.code.retryable());
        out
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
