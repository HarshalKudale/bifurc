//! The `text/event-stream` wire format: bytes in, dispatched events out.
//!
//! Follows the WHATWG rules: lines end in `\n`, `\r` or `\r\n` (a chunk may split a `\r\n`), a line starting with
//! `:` is a comment, `data` lines are joined with `\n`, `event` names the event (`message` when absent), `id` is
//! remembered for the events after it, `retry` is a number of milliseconds, and a blank line dispatches. An event
//! with no `data` line is dropped, and one still open when the stream ends is discarded.

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` name; `message` when the server named none.
    pub event: String,
    pub data: String,
    /// The last `id:` the stream has set, including by an earlier event.
    pub id: Option<String>,
    pub retry: Option<u64>,
}

#[derive(Default)]
pub struct Parser {
    pending: Vec<u8>,
    started: bool,
    after_cr: bool,
    data: Vec<String>,
    event: Option<String>,
    id: Option<String>,
    retry: Option<u64>,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next bytes of the stream; returns the events they completed.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(bytes);
        if !self.started {
            // A byte-order mark may open the stream; wait until there are enough bytes to tell.
            if self.pending.len() < 3 && b"\xEF\xBB\xBF".starts_with(&self.pending) {
                return Vec::new();
            }
            self.started = true;
            if self.pending.starts_with(b"\xEF\xBB\xBF") {
                self.pending.drain(..3);
            }
        }
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            // The LF of a CR LF may arrive in a later chunk than its CR; it is not a line of its own.
            if self.after_cr && start < self.pending.len() {
                self.after_cr = false;
                if self.pending[start] == b'\n' {
                    start += 1;
                }
            }
            let rest = &self.pending[start..];
            let Some(at) = rest.iter().position(|b| *b == b'\n' || *b == b'\r') else { break };
            let line = String::from_utf8_lossy(&rest[..at]).into_owned();
            self.after_cr = rest[at] == b'\r';
            start += at + 1;
            if let Some(event) = self.line(&line) {
                out.push(event);
            }
        }
        self.pending.drain(..start);
        out
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => self.data.push(value.to_owned()),
            "event" => self.event = Some(value.to_owned()),
            // An id holding NUL is ignored.
            "id" if !value.contains('\0') => self.id = Some(value.to_owned()),
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                self.retry = value.parse().ok();
            }
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        let retry = self.retry.take();
        if self.data.is_empty() {
            return None;
        }
        let data = self.data.join("\n");
        self.data.clear();
        Some(SseEvent {
            event: event.filter(|name| !name.is_empty()).unwrap_or_else(|| "message".to_owned()),
            data,
            id: self.id.clone(),
            retry,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<SseEvent> {
        Parser::new().feed(text.as_bytes())
    }

    #[test]
    fn a_blank_line_dispatches_an_event() {
        let events = parse("data: hello\n\ndata: again\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], SseEvent { event: "message".into(), data: "hello".into(), id: None, retry: None });
        assert_eq!(events[1].data, "again");
    }

    #[test]
    fn data_lines_are_joined_with_newlines() {
        assert_eq!(parse("data: one\ndata: two\ndata:\ndata: four\n\n")[0].data, "one\ntwo\n\nfour");
    }

    #[test]
    fn only_one_leading_space_is_removed_from_a_value() {
        assert_eq!(parse("data:  two spaces\n\n")[0].data, " two spaces");
        assert_eq!(parse("data:none\n\n")[0].data, "none");
    }

    #[test]
    fn event_id_and_retry_are_read() {
        let events = parse("event: tick\nid: 7\nretry: 1500\ndata: x\n\n");
        assert_eq!(
            events[0],
            SseEvent { event: "tick".into(), data: "x".into(), id: Some("7".into()), retry: Some(1500) }
        );
    }

    #[test]
    fn the_last_id_carries_over_to_later_events() {
        let events = parse("id: 1\ndata: a\n\ndata: b\n\n");
        assert_eq!(events[1].id.as_deref(), Some("1"));
        // `event` and `retry` apply to one event only.
        let events = parse("event: x\nretry: 5\ndata: a\n\ndata: b\n\n");
        assert_eq!((events[1].event.as_str(), events[1].retry), ("message", None));
    }

    #[test]
    fn comments_unknown_fields_and_bad_retry_are_ignored() {
        let events = parse(": keep-alive\nfoo: bar\nretry: soon\ndata: x\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].retry, None);
    }

    #[test]
    fn an_event_without_data_is_dropped() {
        assert!(parse("event: ping\n\n").is_empty());
        assert!(parse(": comment\n\n").is_empty());
    }

    #[test]
    fn a_field_without_a_colon_is_a_field_with_an_empty_value() {
        assert_eq!(parse("data\n\n")[0].data, "");
    }

    #[test]
    fn every_line_ending_is_understood() {
        for text in ["data: a\r\n\r\n", "data: a\r\r", "data: a\n\n", "data: a\r\n\n"] {
            let events = parse(text);
            assert_eq!(events.len(), 1, "{text:?}");
            assert_eq!(events[0].data, "a");
        }
    }

    #[test]
    fn an_event_split_across_chunks_is_assembled() {
        let mut parser = Parser::new();
        assert!(parser.feed(b"da").is_empty());
        assert!(parser.feed(b"ta: hel").is_empty());
        assert!(parser.feed(b"lo\r").is_empty());
        // The `\n` that completes `\r\n` arrives in the next chunk and must not make an extra blank line.
        assert!(parser.feed(b"\n").is_empty());
        let events = parser.feed(b"\r\ndata: next\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "hello");
        assert_eq!(events[1].data, "next");
    }

    #[test]
    fn a_multibyte_character_split_across_chunks_survives() {
        let bytes = "data: héllo\n\n".as_bytes();
        let split = bytes.iter().position(|b| *b == 0xC3).unwrap() + 1;
        let mut parser = Parser::new();
        assert!(parser.feed(&bytes[..split]).is_empty());
        assert_eq!(parser.feed(&bytes[split..])[0].data, "héllo");
    }

    #[test]
    fn a_byte_order_mark_is_dropped() {
        assert_eq!(parse("\u{feff}data: a\n\n")[0].data, "a");
        let mut parser = Parser::new();
        assert!(parser.feed(b"\xEF").is_empty());
        assert_eq!(parser.feed(b"\xBB\xBFdata: b\n\n")[0].data, "b");
    }

    #[test]
    fn an_unfinished_event_is_never_dispatched() {
        assert!(parse("data: half\n").is_empty());
        assert!(parse("data: half").is_empty());
    }
}
