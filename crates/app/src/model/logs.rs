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

/// How a program's latest run is going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    Stopped,
    /// Ended with an exit code other than 0 (`None` when the code is not known).
    Crashed(Option<i64>),
}

impl RunStatus {
    /// Reads the `state` and `exitCode` of a dev server (`running`, `stopped`, `crashed`).
    pub fn of(value: &Value) -> Self {
        let code = value.get("exitCode").and_then(Value::as_i64);
        match value.get("state").and_then(Value::as_str) {
            Some("running") => Self::Running,
            Some("crashed") => Self::Crashed(code),
            _ => Self::Stopped,
        }
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

/// What a stretch of a log line is, for colouring it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Error,
    Warn,
    Info,
    Debug,
    Method,
    Url,
    Time,
    Number,
    Quoted,
}

/// The kind a word has: a level (`ERROR`, `Warning`), an HTTP method, or an error type (`TypeError`).
fn word_token(word: &str) -> Option<Token> {
    Some(match word {
        "ERROR" | "Error" | "ERR" | "FATAL" | "Fatal" | "PANIC" | "Panic" | "CRITICAL" | "Critical" | "Traceback" => {
            Token::Error
        }
        "WARN" | "Warn" | "WARNING" | "Warning" => Token::Warn,
        "INFO" | "Info" => Token::Info,
        "DEBUG" | "Debug" | "TRACE" | "Trace" => Token::Debug,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" => Token::Method,
        _ if word.ends_with("Error") || word.ends_with("Exception") => Token::Error,
        _ => return None,
    })
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The coloured stretches of a line, in order and apart: URLs, quoted text, times, numbers, levels and
/// HTTP methods. Everything else is plain.
pub fn tokens(text: &str) -> Vec<(std::ops::Range<usize>, Token)> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let Some(c) = rest.chars().next() else { break };
        let boundary = !text[..at].chars().next_back().is_some_and(is_word_char);
        let len = if rest.starts_with("http://") || rest.starts_with("https://") {
            let len = rest.find(|c: char| c.is_whitespace() || "\"'<>)],".contains(c)).unwrap_or(rest.len());
            // Punctuation that ends a sentence is not part of the address.
            let url = rest[..len].trim_end_matches(['.', ':', ';', '!', '?']);
            found.push((at..at + url.len(), Token::Url));
            len
        } else if c == '"' || (c == '\'' && boundary) {
            match rest[1..].find(c) {
                Some(end) => {
                    found.push((at..at + end + 2, Token::Quoted));
                    end + 2
                }
                None => 1,
            }
        } else if c.is_ascii_digit() && boundary {
            let run = rest.find(|c: char| !(c.is_ascii_digit() || ":.-+T".contains(c))).unwrap_or(rest.len());
            let number = rest[..run].trim_end_matches(|c: char| ".:-+T".contains(c));
            let unit = rest[number.len()..].chars().take_while(|c| c.is_ascii_alphabetic() || *c == '%').count();
            let end = number.len() + unit;
            if unit <= 2 && !rest[end..].chars().next().is_some_and(is_word_char) && !number.is_empty() {
                let time = number.contains(':') || (number.contains('-') && number.len() >= 8);
                let zulu = unit == 0 || &rest[number.len()..end] == "Z";
                found.push((at..at + end, if time && zulu { Token::Time } else { Token::Number }));
                end
            } else {
                // A hex id or the like: one plain word.
                rest.find(|c: char| !is_word_char(c)).unwrap_or(rest.len())
            }
        } else if c.is_alphabetic() {
            let len = rest.find(|c: char| !is_word_char(c)).unwrap_or(rest.len());
            if let Some(token) = word_token(&rest[..len]) {
                found.push((at..at + len, token));
            }
            len
        } else {
            c.len_utf8()
        };
        at += len.max(1);
    }
    found
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

    fn coloured(text: &str) -> Vec<(&str, Token)> {
        tokens(text).into_iter().map(|(range, token)| (&text[range], token)).collect()
    }

    #[test]
    fn a_line_is_coloured_by_its_level_urls_quotes_times_and_numbers() {
        assert_eq!(
            coloured(
                "2026-10-25T13:46:11.123Z ERROR GET http://localhost:3000/a?b=1 failed in 12ms: \"boom\" (id 3a7f)"
            ),
            [
                ("2026-10-25T13:46:11.123Z", Token::Time),
                ("ERROR", Token::Error),
                ("GET", Token::Method),
                ("http://localhost:3000/a?b=1", Token::Url),
                ("12ms", Token::Number),
                ("\"boom\"", Token::Quoted),
            ]
        );
        assert_eq!(coloured("warn: TypeError on port 8080"), [("TypeError", Token::Error), ("8080", Token::Number)]);
        assert_eq!(coloured("it's fine, don't panic"), []);
        assert_eq!(coloured("open https://x.dev/a?b=1."), [("https://x.dev/a?b=1", Token::Url)]);
        assert_eq!(coloured("[12:00:01] Info ready"), [("12:00:01", Token::Time), ("Info", Token::Info)]);
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
    fn a_run_is_running_stopped_or_ended_with_a_code() {
        let of = |value: Value| RunStatus::of(&value);
        assert_eq!(of(json!({"state": "running"})), RunStatus::Running);
        assert_eq!(of(json!({"state": "crashed", "exitCode": 2})), RunStatus::Crashed(Some(2)));
        assert_eq!(of(json!({"name": "api"})), RunStatus::Stopped);
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
