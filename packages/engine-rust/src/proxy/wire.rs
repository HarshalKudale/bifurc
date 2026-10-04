//! HTTP/1.1 on a raw socket: reading a request head and body, and parsing its header lines.
//!
//! Both listeners answer one request per connection and close, so there is no keep-alive state to carry.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

use crate::{kernel::telemetry::Headers, utils::http::is_hop_by_hop};

/// The largest request head accepted.
const MAX_HEAD_BYTES: usize = 1024 * 1024;

/// A request's first line and header lines.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestHead {
    pub method: String,
    pub target: String,
    pub header_lines: Vec<String>,
}

/// Request headers that, sent twice, keep only the first value (the same set Node's HTTP parser applies).
const FIRST_WINS: [&str; 17] = [
    "age",
    "authorization",
    "content-length",
    "content-type",
    "etag",
    "expires",
    "from",
    "host",
    "if-modified-since",
    "if-unmodified-since",
    "last-modified",
    "location",
    "max-forwards",
    "proxy-authorization",
    "referer",
    "retry-after",
    "user-agent",
];

/// How repeated request headers combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duplicates {
    /// The last one wins (a plain record assignment).
    LastWins,
    /// Joined with `, ` (`; ` for cookies), except the headers that keep only their first value.
    Joined,
}

/// The header lines as lowercase names to trimmed values, without hop-by-hop headers.
pub fn parse_header_lines(lines: &[String], duplicates: Duplicates) -> Headers {
    let mut headers = Headers::new();
    for line in lines {
        let Some(index) = line.find(':') else { continue };
        let name = line[..index].trim().to_lowercase();
        if is_hop_by_hop(&name) {
            continue;
        }
        let value = line[index + 1..].trim().to_string();
        match (duplicates, headers.get_mut(&name)) {
            (Duplicates::Joined, Some(existing)) => {
                if FIRST_WINS.contains(&name.as_str()) {
                    continue;
                }
                existing.push_str(if name == "cookie" { "; " } else { ", " });
                existing.push_str(&value);
            }
            _ => {
                headers.insert(name, value);
            }
        }
    }
    headers
}

/// A header's value from raw lines, whatever its case.
pub fn raw_header(lines: &[String], name: &str) -> Option<String> {
    lines.iter().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
    })
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// Read until the blank line that ends a request head. Returns the head and the bytes already read past it,
/// or `None` when the peer closed first or sent more than a head can be.
pub async fn read_head<R: AsyncRead + Unpin>(stream: &mut R) -> Option<(RequestHead, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut searched_from = 0;
    loop {
        if let Some(position) = find_subsequence(&buf[searched_from..], b"\r\n\r\n") {
            let separator = searched_from + position;
            let section = String::from_utf8_lossy(&buf[..separator]).into_owned();
            let leftover = buf[separator + 4..].to_vec();
            let mut lines = section.split("\r\n");
            let first = lines.next().unwrap_or("");
            let mut parts = first.split(' ');
            let method = parts.next().filter(|m| !m.is_empty()).unwrap_or("GET").to_string();
            let target = parts.next().unwrap_or("/").to_string();
            let header_lines = lines.map(str::to_string).collect();
            return Some((RequestHead { method, target, header_lines }, leftover));
        }
        searched_from = buf.len().saturating_sub(3);
        if buf.len() > MAX_HEAD_BYTES {
            return None;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..read]);
    }
}

/// Bytes already read, then the stream.
struct Buffered<'a, S> {
    stream: &'a mut S,
    buf: Vec<u8>,
}

impl<S: AsyncRead + Unpin> Buffered<'_, S> {
    async fn fill(&mut self) -> io::Result<()> {
        let mut chunk = [0u8; 8192];
        let read = self.stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.buf.extend_from_slice(&chunk[..read]);
        Ok(())
    }

    async fn take(&mut self, count: usize) -> io::Result<Vec<u8>> {
        while self.buf.len() < count {
            self.fill().await?;
        }
        Ok(self.buf.drain(..count).collect())
    }

    async fn line(&mut self) -> io::Result<String> {
        loop {
            if let Some(end) = find_subsequence(&self.buf, b"\r\n") {
                let line: Vec<u8> = self.buf.drain(..end + 2).collect();
                return Ok(String::from_utf8_lossy(&line[..end]).into_owned());
            }
            if self.buf.len() > MAX_HEAD_BYTES {
                return Err(io::ErrorKind::InvalidData.into());
            }
            self.fill().await?;
        }
    }
}

/// The request body that follows a head: `content-length` bytes, or the chunks of a chunked body. A request
/// that announces neither has none. An `Expect: 100-continue` is answered before the body is waited for.
pub async fn read_body<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    leftover: Vec<u8>,
    header_lines: &[String],
) -> io::Result<Vec<u8>> {
    let chunked = raw_header(header_lines, "transfer-encoding").is_some_and(|v| v.to_lowercase().contains("chunked"));
    let length = raw_header(header_lines, "content-length").and_then(|v| v.parse::<usize>().ok());
    if !chunked && length.is_none_or(|n| n == 0) {
        return Ok(Vec::new());
    }
    if leftover.is_empty() && raw_header(header_lines, "expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue"))
    {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await?;
        stream.flush().await?;
    }
    let mut reader = Buffered { stream, buf: leftover };
    if !chunked {
        return reader.take(length.unwrap_or(0)).await;
    }
    let mut body = Vec::new();
    loop {
        let size_line = reader.line().await?;
        let size_text = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        if size == 0 {
            while !reader.line().await?.is_empty() {}
            return Ok(body);
        }
        body.extend(reader.take(size).await?);
        reader.take(2).await?;
    }
}

/// A stream that yields `prefix` before reading from `inner`: what a socket looks like after its request head
/// was read off it and some of what followed was read too.
pub struct Prefixed<S> {
    prefix: Vec<u8>,
    position: usize,
    inner: S,
}

impl<S> Prefixed<S> {
    pub fn new(prefix: Vec<u8>, inner: S) -> Self {
        Self { prefix, position: 0, inner }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        if self.position < self.prefix.len() {
            let available = &self.prefix[self.position..];
            let count = available.len().min(buf.remaining());
            buf.put_slice(&available[..count]);
            self.position += count;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn header_lines_lowercase_and_skip_hop_by_hop() {
        let parsed = parse_header_lines(
            &lines(&["Host: A.test", "Connection: keep-alive", "X-Dup: 1", "x-dup: 2", "no colon", "Cookie: a=1"]),
            Duplicates::LastWins,
        );
        let pairs: Vec<(&str, &str)> = parsed.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(pairs, [("host", "A.test"), ("x-dup", "2"), ("cookie", "a=1")]);
    }

    #[test]
    fn joined_duplicates() {
        let parsed = parse_header_lines(
            &lines(&["Accept: a", "accept: b", "Cookie: x=1", "cookie: y=2", "Host: one", "host: two"]),
            Duplicates::Joined,
        );
        assert_eq!(parsed["accept"], "a, b");
        assert_eq!(parsed["cookie"], "x=1; y=2");
        assert_eq!(parsed["host"], "one");
    }

    #[tokio::test]
    async fn reads_a_head_and_keeps_what_followed() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        client
            .write_all(b"POST http://a.test/x?y=1 HTTP/1.1\r\nHost: a.test\r\nContent-Length: 3\r\n\r\nab")
            .await
            .unwrap();
        let (head, leftover) = read_head(&mut server).await.unwrap();
        assert_eq!(head.method, "POST");
        assert_eq!(head.target, "http://a.test/x?y=1");
        assert_eq!(head.header_lines, ["Host: a.test", "Content-Length: 3"]);
        assert_eq!(leftover, b"ab");
        client.write_all(b"c").await.unwrap();
        let body = read_body(&mut server, leftover, &head.header_lines).await.unwrap();
        assert_eq!(body, b"abc");
    }

    #[tokio::test]
    async fn a_closed_connection_has_no_head() {
        let (client, mut server) = tokio::io::duplex(64);
        drop(client);
        assert!(read_head(&mut server).await.is_none());
    }

    #[tokio::test]
    async fn decodes_a_chunked_body() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        client.write_all(b"3;x=1\r\nabc\r\n2\r\nde\r\n0\r\nTrailer: t\r\n\r\n").await.unwrap();
        let body = read_body(&mut server, Vec::new(), &lines(&["Transfer-Encoding: chunked"])).await.unwrap();
        assert_eq!(body, b"abcde");
    }

    #[tokio::test]
    async fn a_request_without_a_length_has_no_body() {
        let (_client, mut server) = tokio::io::duplex(64);
        let body = read_body(&mut server, b"GET /next".to_vec(), &lines(&["Host: a"])).await.unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn prefixed_yields_the_prefix_first() {
        let (mut client, server) = tokio::io::duplex(64);
        client.write_all(b"def").await.unwrap();
        drop(client);
        let mut stream = Prefixed::new(b"abc".to_vec(), server);
        let mut out = Vec::new();
        stream.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"abcdef");
    }
}
