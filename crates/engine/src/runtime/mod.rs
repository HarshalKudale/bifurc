//! The engine as a process: its arguments, how clients find it, the listeners it runs and its lifetime.

pub mod args;
pub mod desktop;
pub mod discovery;
pub mod entity_status;
pub mod host;
pub mod instances;
pub mod ports;
#[cfg(unix)]
pub mod shell_path;

use std::sync::atomic::{AtomicBool, Ordering};

static NOTICES_ON_STDERR: AtomicBool = AtomicBool::new(false);

/// Send notices to stderr: with the stdio transport, stdout carries frames.
pub fn route_notices_to_stderr() {
    NOTICES_ON_STDERR.store(true, Ordering::Relaxed);
}

/// Tell whoever started the engine something they may need to act on.
pub fn notice(message: &str) {
    if NOTICES_ON_STDERR.load(Ordering::Relaxed) {
        eprintln!("[host] {message}");
    } else {
        println!("[host] {message}");
    }
}

tokio::task_local! {
    /// The command a task is answering, so a failure it reports can be logged by name.
    static COMMAND: String;
}

/// Runs `work` as the answer to `command`, for [`log_failure`] and [`failed`].
pub async fn with_command<F: std::future::Future>(command: &str, work: F) -> F::Output {
    COMMAND.scope(command.to_owned(), work).await
}

/// One log line for a failure: the command (when known), the code and the key (`-` without one). Never the
/// message or params: they can hold URLs, names and tokens.
pub fn log_failure(command: Option<&str>, error: &crate::protocol::EngineError) {
    let current = COMMAND.try_with(Clone::clone).ok();
    let prefix = command.map(str::to_owned).or(current).map_or_else(String::new, |name| format!("{name} "));
    notice(&format!("{prefix}failed: {} {}", error.code.as_str(), error.key().unwrap_or("-")));
}

/// Writes `error`, `errorKey` and `errorParams` (the last two only for a keyed error) into the object
/// `result`, and logs the failure. Old clients and the CLI keep reading `error`.
pub fn set_failure(result: &mut serde_json::Value, error: &crate::protocol::EngineError) {
    log_failure(None, error);
    result["error"] = serde_json::json!(error.message);
    if let (Some(key), Some(params)) = (error.key(), error.params()) {
        result["errorKey"] = serde_json::json!(key);
        result["errorParams"] = params.clone();
    } else if let Some(map) = result.as_object_mut() {
        map.remove("errorKey");
        map.remove("errorParams");
    }
}

/// The `{ ok: false, error, errorKey?, errorParams? }` result of a command that failed in a way the caller
/// handles as a result rather than a wire error.
pub fn failed(error: &crate::protocol::EngineError) -> serde_json::Value {
    let mut result = serde_json::json!({ "ok": false });
    set_failure(&mut result, error);
    result
}

#[cfg(test)]
mod failure_tests {
    use serde_json::json;

    use super::*;
    use crate::protocol::EngineError;

    #[test]
    fn a_failed_result_carries_the_message_and_the_key_beside_it() {
        let keyed = EngineError::conflict("No remote configured").keyed("sync.no_remote", json!({}));
        assert_eq!(
            failed(&keyed),
            json!({ "ok": false, "error": "No remote configured", "errorKey": "sync.no_remote", "errorParams": {} })
        );
        let plain = failed(&EngineError::engine("boom"));
        assert_eq!(plain, json!({ "ok": false, "error": "boom" }));
    }

    #[test]
    fn set_failure_replaces_a_stale_key() {
        let mut result = json!({ "ok": false, "errorKey": "old.key", "errorParams": {} });
        set_failure(&mut result, &EngineError::engine("boom"));
        assert_eq!(result, json!({ "ok": false, "error": "boom" }));
    }
}
