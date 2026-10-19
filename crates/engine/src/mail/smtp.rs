//! The receiving end of the mail sink: one SMTP conversation. No authentication and no TLS (the port is on
//! loopback and only a test app talks to it); every message is accepted for every recipient.

use std::time::Duration;

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::timeout,
};

/// How long a client may stay silent before the connection is dropped.
const IDLE: Duration = Duration::from_secs(60);

/// The longest line read; the SMTP limit is 998, but html mails often ignore it.
const MAX_LINE: u64 = 64 * 1024;

async fn say(io: &mut BufReader<TcpStream>, text: &str) -> bool {
    io.get_mut().write_all(format!("{text}\r\n").as_bytes()).await.is_ok()
}

/// The next line without its line ending; `None` when the client left, went silent or sent too long a line.
async fn line(io: &mut BufReader<TcpStream>) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let read = timeout(IDLE, (&mut *io).take(MAX_LINE).read_until(b'\n', &mut buf)).await.ok()?.ok()?;
    if read == 0 || buf.last() != Some(&b'\n') {
        return None;
    }
    buf.pop();
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Some(buf)
}

/// The address in `MAIL FROM:<a@b>` / `RCPT TO:<a@b> SIZE=1`, lower case; empty for the bounce address `<>`.
fn address_of(rest: &str) -> String {
    let rest = rest.trim();
    let inner = match (rest.find('<'), rest.find('>')) {
        (Some(open), Some(close)) if open < close => &rest[open + 1..close],
        _ => rest.split_whitespace().next().unwrap_or_default(),
    };
    inner.trim().to_lowercase()
}

/// The message body of a DATA command: lines up to the lone `.`, dot-unstuffed. `None` when the client left
/// first; `Some(None)` when it went past `max` bytes (the rest is read and dropped, so the reply stays in step).
async fn data(io: &mut BufReader<TcpStream>, max: usize) -> Option<Option<Vec<u8>>> {
    let (mut body, mut big) = (Vec::new(), false);
    loop {
        let line = line(io).await?;
        if line == b"." {
            return Some((!big).then_some(body));
        }
        let line = line.strip_prefix(b".").unwrap_or(&line);
        big = big || body.len() + line.len() + 2 > max;
        if !big {
            body.extend_from_slice(line);
            body.extend_from_slice(b"\r\n");
        }
    }
}

/// Serve one client until it quits. A message of more than `max` bytes is refused with 552.
pub async fn session(socket: TcpStream, max: usize) {
    let mut io = BufReader::new(socket);
    if !say(&mut io, "220 bifurc ESMTP ready").await {
        return;
    }
    let (mut from, mut rcpts): (Option<String>, Vec<String>) = (None, Vec::new());
    while let Some(raw) = line(&mut io).await {
        let text = String::from_utf8_lossy(&raw).trim().to_string();
        let upper = text.to_uppercase();
        let reply = match upper.split_whitespace().next().unwrap_or_default() {
            "HELO" => "250 bifurc".to_string(),
            "EHLO" => format!("250-bifurc\r\n250 SIZE {max}"),
            "MAIL" if upper.starts_with("MAIL FROM:") => {
                let declared =
                    upper.split("SIZE=").nth(1).and_then(|n| n.split_whitespace().next()?.parse::<usize>().ok());
                if declared.is_some_and(|n| n > max) {
                    "552 message too large".to_string()
                } else {
                    (from, rcpts) = (Some(address_of(&text[10..])), Vec::new());
                    "250 ok".to_string()
                }
            }
            "RCPT" if upper.starts_with("RCPT TO:") && from.is_some() => {
                rcpts.push(address_of(&text[8..]));
                "250 ok".to_string()
            }
            "MAIL" | "RCPT" => "503 bad sequence of commands (MAIL FROM:<a>, then RCPT TO:<b>)".to_string(),
            "DATA" if rcpts.is_empty() => "503 need RCPT TO first".to_string(),
            "DATA" => {
                if !say(&mut io, "354 end data with <CR><LF>.<CR><LF>").await {
                    return;
                }
                let Some(body) = data(&mut io, max).await else { return };
                let envelope = (from.take().unwrap_or_default(), std::mem::take(&mut rcpts));
                match body {
                    Some(body) => format!("250 ok queued as {}", super::receive(&envelope.0, &envelope.1, &body)),
                    None => "552 message too large".to_string(),
                }
            }
            "RSET" => {
                (from, rcpts) = (None, Vec::new());
                "250 ok".to_string()
            }
            "NOOP" => "250 ok".to_string(),
            "QUIT" => {
                say(&mut io, "221 bye").await;
                return;
            }
            "AUTH" | "STARTTLS" => "502 not supported".to_string(),
            _ => "500 unknown command".to_string(),
        };
        if !say(&mut io, &reply).await {
            return;
        }
    }
}
