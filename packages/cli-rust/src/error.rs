//! The CLI's error type and how an error is shown: `bifurc: CODE: message`.

use std::fmt;

/// Why a command failed.
#[derive(Debug, Clone, PartialEq)]
pub enum CliError {
    /// The message is the whole story for the user.
    Message(String),
    /// The engine refused or failed a request; shown with its error code.
    Engine { code: String, message: String },
}

/// A result carrying a [`CliError`].
pub type Result<T> = std::result::Result<T, CliError>;

impl CliError {
    /// An error whose message is the whole story.
    pub fn msg(message: impl Into<String>) -> Self {
        CliError::Message(message.into())
    }

    /// The line written to stderr.
    pub fn line(&self) -> String {
        format!("bifurc: {self}")
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Message(message) => f.write_str(message),
            CliError::Engine { code, message } => write!(f, "{code}: {message}"),
        }
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_error_shows_its_message() {
        assert_eq!(CliError::msg("missing workspace id").line(), "bifurc: missing workspace id");
    }

    #[test]
    fn an_engine_error_shows_its_code_first() {
        let err = CliError::Engine { code: "UNKNOWN_COMMAND".into(), message: "nope".into() };
        assert_eq!(err.line(), "bifurc: UNKNOWN_COMMAND: nope");
    }
}
