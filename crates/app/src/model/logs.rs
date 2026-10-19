//! Dev server output, and the lines of it that belong to a captured request.
//!
//! The rules are the CLI's (`bifurc logs --errors`, `bifurc capture show --logs`), so the app and an agent
//! see the same lines: change them there first.

use serde_json::{Value, json};

/// One line a dev server printed.
#[derive(Debug, Clone, PartialEq)]
pub struct LogLine {
    pub ts: u64,
    pub stderr: bool,
    pub text: String,
}

impl LogLine {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            ts: value.get("ts").and_then(Value::as_u64).unwrap_or_default(),
            stderr: value.get("stream").and_then(Value::as_str) == Some("stderr"),
            text: value.get("text")?.as_str()?.to_owned(),
        })
    }

    /// The `lines` of a `devservice.logs` answer or an `event.devservice.output` payload.
    pub fn list(value: &Value) -> Vec<Self> {
        let lines = value.get("lines").and_then(Value::as_array);
        lines.map(|lines| lines.iter().filter_map(Self::from_value).collect()).unwrap_or_default()
    }
}

/// How many lines the panel keeps: the engine's own ring.
pub const KEPT: usize = 5000;

/// Adds `new` at the end, dropping the oldest lines beyond [`KEPT`].
pub fn append(lines: &mut Vec<LogLine>, new: Vec<LogLine>) {
    lines.extend(new);
    if lines.len() > KEPT {
        lines.drain(..lines.len() - KEPT);
    }
}

/// A line as the panel shows it: consecutive repeats are one line with a count.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub ts: u64,
    pub stderr: bool,
    pub text: String,
    pub count: usize,
}

impl Shown {
    /// The line as copied: `text ×N` for a repeat, as the CLI prints it.
    pub fn copied(&self) -> String {
        if self.count > 1 { format!("{} \u{d7}{}", self.text, self.count) } else { self.text.clone() }
    }
}

/// Whether a line is at warn or error level.
fn is_error(line: &str) -> bool {
    let lower = line.to_lowercase();
    ["error", "err!", "warn", "fatal", "exception", "panic", "critical", "traceback"].iter().any(|w| lower.contains(w))
}

/// Whether a line continues a stack trace: indented, `at …`, `File "…"`, `Caused by`.
fn is_continuation(line: &str) -> bool {
    line.starts_with(char::is_whitespace) || ["at ", "File \"", "Caused by", "... "].iter().any(|p| line.starts_with(p))
}

/// The lines to show: all of them, or with `errors_only` the warn/error lines with the stack traces under
/// them and consecutive repeats collapsed (as `--errors`); then those with `needle` (lowercase), if any.
pub fn visible(lines: &[LogLine], errors_only: bool, needle: &str) -> Vec<Shown> {
    let mut kept: Vec<Shown> = Vec::new();
    let mut previous = false;
    for line in lines {
        if errors_only {
            previous = is_error(&line.text) || (previous && is_continuation(&line.text));
            if !previous {
                continue;
            }
        }
        match kept.last_mut() {
            Some(last) if errors_only && last.text == line.text => last.count += 1,
            _ => kept.push(Shown { ts: line.ts, stderr: line.stderr, text: line.text.clone(), count: 1 }),
        }
    }
    kept.retain(|line| needle.is_empty() || line.text.to_lowercase().contains(needle));
    kept
}

/// The path of a URL without its query.
fn path_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.find('/').map_or("/", |at| &rest[at..]);
    path.split(['?', '#']).next().unwrap_or("/").to_string()
}

/// The `devservice.grep` for the lines that carry a request's id or trace id.
pub fn exact_query(workspace: &str, id: &str, trace_id: Option<&str>) -> Value {
    let mut needles = vec![id];
    needles.extend(trace_id);
    json!({
        "workspaceId": workspace,
        "needles": needles,
        "probes": ["x-bifurc-trace", &id[..id.len().min(5)]],
    })
}

/// The `devservice.grep` for the fallback: lines that mention the request's path from a second before it came
/// in to two after it was answered. `None` for the home page, which has no path to match on.
pub fn near_query(workspace: &str, url: &str, ts: u64, duration_ms: u64) -> Option<Value> {
    let path = path_of(url);
    (path.len() > 1).then(|| {
        let start = i64::try_from(ts).unwrap_or(i64::MAX);
        let ms = i64::try_from(duration_ms).unwrap_or_default();
        json!({
            "workspaceId": workspace,
            "needles": [path],
            "sinceTs": start - 1000,
            "untilTs": start + ms + 2000,
        })
    })
}

/// One line of a `devservice.grep` answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub service: String,
    pub text: String,
    /// `false` for the context around a match.
    pub matched: bool,
    /// Not the line after the one before it: something was left out in between.
    pub gap: bool,
}

/// The lines of a `devservice.grep` answer; `None` when no line matched (context alone is not a hit).
pub fn found(answer: &Value) -> Option<Vec<Found>> {
    let lines = answer.get("lines")?.as_array()?;
    if !lines.iter().any(|l| l["match"] == json!(true)) {
        return None;
    }
    let mut last: Option<(&str, u64)> = None;
    Some(
        lines
            .iter()
            .map(|line| {
                let service = line["service"].as_str().unwrap_or_default();
                let seq = line["seq"].as_u64().unwrap_or_default();
                let gap = last.is_some_and(|(s, n)| s != service || n + 1 != seq);
                last = Some((service, seq));
                Found {
                    service: service.to_owned(),
                    text: line["text"].as_str().unwrap_or_default().to_owned(),
                    matched: line["match"] == json!(true),
                    gap,
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<LogLine> {
        texts.iter().map(|text| LogLine { ts: 1, stderr: false, text: (*text).to_owned() }).collect()
    }

    fn texts(shown: &[Shown]) -> Vec<String> {
        shown.iter().map(Shown::copied).collect()
    }

    #[test]
    fn errors_keep_warn_and_error_lines_with_their_stack_and_collapse_repeats() {
        let all = lines(&[
            "ready on 3000",
            "GET /a 200",
            "Error: boom",
            "    at handler (app.js:1:2)",
            "    indented info, not part of a trace",
            "WARN slow query",
            "WARN slow query",
            "WARN slow query",
            "Traceback (most recent call last):",
            "  File \"app.py\", line 3, in run",
            "ValueError: bad",
        ]);
        assert_eq!(
            texts(&visible(&all, true, "")),
            [
                "Error: boom",
                "    at handler (app.js:1:2)",
                "    indented info, not part of a trace",
                "WARN slow query \u{d7}3",
                "Traceback (most recent call last):",
                "  File \"app.py\", line 3, in run",
                "ValueError: bad",
            ]
        );
        assert!(visible(&lines(&["    at lonely (x.js:1:1)", "all fine"]), true, "").is_empty());
    }

    #[test]
    fn without_errors_only_every_line_stays_and_the_search_ignores_case() {
        let all = lines(&["a", "a", "Listening on PORT 3000"]);
        assert_eq!(visible(&all, false, "").len(), 3);
        assert_eq!(texts(&visible(&all, false, "port")), ["Listening on PORT 3000"]);
        assert_eq!(texts(&visible(&lines(&["warn x", "ok", "warn y"]), true, "y")), ["warn y"]);
    }

    #[test]
    fn the_buffer_keeps_the_newest_lines() {
        let mut buffer = lines(&["old"]);
        append(&mut buffer, (0..KEPT).map(|n| LogLine { ts: 0, stderr: false, text: n.to_string() }).collect());
        assert_eq!((buffer.len(), buffer[0].text.as_str()), (KEPT, "0"));
    }

    #[test]
    fn output_lines_are_read_with_their_stream() {
        let read = LogLine::list(&json!({"lines": [
            {"ts": 5, "stream": "stderr", "text": "bad"}, {"ts": 6, "stream": "file", "text": "ok"}, {"ts": 7}]}));
        assert_eq!(read.iter().map(|l| (l.ts, l.stderr)).collect::<Vec<_>>(), [(5, true), (6, false)]);
    }

    #[test]
    fn the_exact_search_has_the_id_and_the_trace_id_and_the_fallback_a_window_around_the_path() {
        let exact = exact_query("w1", "mgv2k3f9abcd", Some("4bf9"));
        assert_eq!(exact["needles"], json!(["mgv2k3f9abcd", "4bf9"]));
        assert_eq!(exact["probes"], json!(["x-bifurc-trace", "mgv2k"]));
        assert_eq!(exact_query("w1", "ab", None)["needles"], json!(["ab"]));
        let near = near_query("w1", "http://api.localhost/orders?x=1", 10_000, 40).unwrap();
        assert_eq!(
            (&near["needles"], &near["sinceTs"], &near["untilTs"]),
            (&json!(["/orders"]), &json!(9000), &json!(12040))
        );
        assert!(near_query("w1", "http://api.localhost/", 1, 1).is_none());
    }

    #[test]
    fn a_hit_needs_a_matching_line_and_gaps_mark_what_was_left_out() {
        let line = |service: &str, seq: u64, matched: bool| json!({"service": service, "seq": seq, "text": "t", "match": matched});
        assert!(found(&json!({"lines": [line("api", 1, false)]})).is_none());
        let answer = json!({"lines": [line("api", 3, true), line("api", 4, false), line("api", 9, true), line("web", 10, true)]});
        let got = found(&answer).unwrap();
        assert_eq!(got.iter().map(|l| l.gap).collect::<Vec<_>>(), [false, false, true, true]);
        assert_eq!(got.iter().map(|l| l.matched).collect::<Vec<_>>(), [true, false, true, true]);
    }
}
