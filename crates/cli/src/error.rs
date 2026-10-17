//! The CLI's error type and how an error is shown: `bifurc: CODE: message`.

use std::fmt;

/// Why a command failed.
#[derive(Debug, Clone, PartialEq)]
pub enum CliError {
    /// The message is the whole story for the user.
    Message(String),
    /// The engine refused or failed a request; shown with its error code.
    Engine { code: String, message: String },
    /// The command ran and what it reports is a failure (a failed test, a service that does not answer). The
    /// report is printed like any other answer; only the exit code says it failed.
    Failed(String),
    /// A failure with a meaning of its own, shown like a message: the exit code is the one given (see [`CliError::code`]).
    Exit(i32, String),
}

/// The exit codes beyond 0 (ok) and 1 (the command ran and what it reports failed).
const USAGE: i32 = 2;
pub(crate) const NOT_FOUND: i32 = 3;
const NETWORK: i32 = 4;
const ENGINE_UNREACHABLE: i32 = 5;
const NEEDS_HUMAN: i32 = 6;

/// A result carrying a [`CliError`].
pub type Result<T> = std::result::Result<T, CliError>;

impl CliError {
    /// An error whose message is the whole story.
    pub fn msg(message: impl Into<String>) -> Self {
        CliError::Message(message.into())
    }

    /// A wrong command line: unknown command, verb or option, a missing or bad argument (exit 2).
    pub fn usage(message: impl Into<String>) -> Self {
        CliError::Exit(USAGE, message.into())
    }

    /// A named thing that does not exist (exit 3).
    pub fn not_found(message: impl Into<String>) -> Self {
        CliError::Exit(NOT_FOUND, message.into())
    }

    /// What was sent could not reach its target or got no answer (exit 4).
    pub fn network(message: impl Into<String>) -> Self {
        CliError::Exit(NETWORK, message.into())
    }

    /// The engine is not running and could not be started or reached (exit 5).
    pub fn engine_unreachable(message: impl Into<String>) -> Self {
        CliError::Exit(ENGINE_UNREACHABLE, message.into())
    }

    /// Only a person can go on, e.g. unseal a vault at a terminal (exit 6).
    pub fn needs_human(message: impl Into<String>) -> Self {
        CliError::Exit(NEEDS_HUMAN, message.into())
    }

    /// The process exit code of this error.
    pub fn code(&self) -> i32 {
        match self {
            CliError::Exit(code, _) => *code,
            CliError::Engine { code, .. } => match code.as_str() {
                "NOT_FOUND" => NOT_FOUND,
                "TIMEOUT" | "UPSTREAM_FAILED" => NETWORK,
                _ => 1,
            },
            CliError::Message(_) | CliError::Failed(_) => 1,
        }
    }

    /// The line written to stderr.
    pub fn line(&self) -> String {
        format!("bifurc: {self}")
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Message(message) | CliError::Failed(message) | CliError::Exit(_, message) => f.write_str(message),
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
    fn the_exit_code_follows_the_kind_of_error() {
        let engine = |code: &str| CliError::Engine { code: code.into(), message: "m".into() };
        assert_eq!(CliError::msg("x").code(), 1);
        assert_eq!(CliError::Failed("x".into()).code(), 1);
        assert_eq!(CliError::usage("x").code(), 2);
        assert_eq!(engine("NOT_FOUND").code(), 3);
        assert_eq!(engine("TIMEOUT").code(), 4);
        assert_eq!(CliError::engine_unreachable("x").code(), 5);
        assert_eq!(CliError::needs_human("x").code(), 6);
        assert_eq!(engine("ENGINE_ERROR").code(), 1);
    }

    #[test]
    fn an_engine_error_shows_its_code_first() {
        let err = CliError::Engine { code: "UNKNOWN_COMMAND".into(), message: "nope".into() };
        assert_eq!(err.line(), "bifurc: UNKNOWN_COMMAND: nope");
    }
}
