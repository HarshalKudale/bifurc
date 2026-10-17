//! The lines of a run of many requests (`runner run`, `flows run`): one per step, a failed step's body under it.

use crate::{
    args::ParsedArgs,
    error::{CliError, Result},
    format::aligned,
    response::{preview, size},
};

/// A body of a failed step is cut here, as a one-shot response's is.
const FAILED_BODY: usize = 384;

/// One request a run sent.
#[derive(Default)]
pub struct Step {
    /// What it ran: `POST /login`, or the name of a node that is not a request.
    pub label: String,
    /// The status, as the engine gave it; `None` when nothing answered.
    pub status: Option<String>,
    pub ms: Option<u64>,
    /// Why it failed; `None` when it did not.
    pub failure: Option<String>,
    pub body: Option<String>,
}

/// `--max-body`: how many bytes of a body `--verbose` shows (default 2048; 0 = all).
pub fn max_body(args: &ParsedArgs) -> Result<usize> {
    args.flag("max-body").map_or(Ok(2048), |raw| {
        raw.parse().map_err(|_| CliError::usage(format!("--max-body must be a whole number, not \"{raw}\"")))
    })
}

/// The path (and query) of `url`: without its scheme and host, or without a leading `{{variable}}`.
pub fn path_of(url: &str) -> String {
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/", |at| &rest[at..]),
        None => match url.strip_prefix("{{").and_then(|rest| rest.split_once("}}")) {
            Some((_, rest)) if !rest.is_empty() => rest,
            _ => url,
        },
    };
    if rest.is_empty() { "/".to_string() } else { rest.to_string() }
}

/// `first line of text`, cut to one line.
pub fn one_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}

/// The steps as aligned lines, `1 POST /login  200 · 88ms  ✓`. A failed step shows up to 384 bytes of its body
/// under it, and with `verbose` every step shows up to `max_body` bytes (0 = all).
pub fn lines(steps: &[Step], verbose: bool, max_body: usize) -> Vec<String> {
    let cells: Vec<Vec<String>> = steps
        .iter()
        .enumerate()
        .map(|(at, step)| {
            let took = step.ms.map_or(String::new(), |ms| format!(" · {ms}ms"));
            let mark = match &step.failure {
                Some(why) => format!("✗ {why}"),
                None => "✓".to_string(),
            };
            vec![format!("{} {}", at + 1, step.label), format!("{}{took}", step.status.as_deref().unwrap_or("-")), mark]
        })
        .collect();
    let mut out = Vec::new();
    for (row, step) in aligned(&cells).lines().zip(steps) {
        out.push(row.to_string());
        let limit = if verbose { max_body } else { FAILED_BODY };
        let body = step.body.as_deref().filter(|body| !body.trim().is_empty());
        if let Some(body) = body.filter(|_| verbose || step.failure.is_some()) {
            let (shown, left) = preview(body, limit);
            out.push(format!("  → {}", shown.split_whitespace().collect::<Vec<_>>().join(" ")));
            if left > 0 {
                out.push(format!("  … {} truncated", size(left)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_drop_the_host_or_a_leading_variable() {
        assert_eq!(path_of("http://127.0.0.1:80/carts/c_91?a=1"), "/carts/c_91?a=1");
        assert_eq!(path_of("https://x.dev"), "/");
        assert_eq!(path_of("{{baseUrl}}/login"), "/login");
        assert_eq!(path_of("{{baseUrl}}"), "{{baseUrl}}");
    }

    #[test]
    fn only_a_failed_step_shows_its_body_unless_verbose() {
        let step = |failure: Option<&str>, body: &str| Step {
            label: "GET /a".into(),
            status: Some("200".into()),
            ms: Some(5),
            failure: failure.map(str::to_string),
            body: Some(body.into()),
        };
        let steps = [step(None, "{\"a\":1}"), step(Some("boom"), &"x".repeat(600))];
        let shown = lines(&steps, false, 2048);
        assert_eq!(shown[0], "1 GET /a  200 · 5ms  ✓");
        assert_eq!(shown[1], "2 GET /a  200 · 5ms  ✗ boom");
        assert_eq!(shown[2].len(), "  → ".len() + 384);
        assert_eq!(shown[3], "  … 216B truncated");
        assert_eq!(lines(&steps, true, 2048).len(), 4);
    }
}
