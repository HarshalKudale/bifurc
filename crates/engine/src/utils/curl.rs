//! The cURL command parser, shared by `request.parseCurl`, the cURL importer and `curl.buildRequest` /
//! `curl.buildMock`.
//!
//! It reads a command the way a shell and curl would: bash, `$'…'`, Windows prompt (`^`) and PowerShell
//! (backtick) quoting, option clusters (`-sSL`, `-XPOST`), `--name=value`, and the options that shape the
//! request. Options that only shape the transfer (`--compressed`, `-o file`, `-x proxy`) are stepped over.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};

use crate::kernel::traffic::Headers;

/// Hop-by-hop and connection headers dropped when a cURL command or a captured request is turned into an
/// editable request. They describe one connection, not the request's intent.
pub const SKIP_CURL_HEADERS: [&str; 5] =
    ["host", "proxy-connection", "connection", "content-length", "transfer-encoding"];

/// Whether `name` (already lowercase) is one of [`SKIP_CURL_HEADERS`].
pub fn is_skipped_header(name: &str) -> bool {
    SKIP_CURL_HEADERS.contains(&name)
}

/// The request a cURL command describes. Header names are lowercase and unfiltered.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedCurl {
    pub method: String,
    pub url: String,
    pub headers: Headers,
    pub body: String,
}

impl ParsedCurl {
    /// The wire shape (`RequestParseCurlResult`).
    pub fn to_value(&self) -> Value {
        json!({ "method": self.method, "url": self.url, "headers": self.headers, "body": self.body })
    }
}

/// One `-F` field of a multipart body. A file field carries no content: the file is on the machine the
/// command was written for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CurlPart {
    pub name: String,
    pub value: String,
    pub file: bool,
    pub file_name: String,
    pub content_type: String,
}

/// Everything a cURL command says about its request. Header names are as they were typed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CurlRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub parts: Vec<CurlPart>,
}

impl CurlRequest {
    /// The value of a header, whatever its case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn lowercase_headers(&self) -> Headers {
        let mut headers = Headers::new();
        for (name, value) in &self.headers {
            headers.insert(name.to_lowercase(), value.clone());
        }
        headers
    }

    fn into_parsed(self) -> ParsedCurl {
        let headers = self.lowercase_headers();
        ParsedCurl { method: self.method, url: self.url, headers, body: self.body }
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// The length of a line continuation at `i`: a backslash (bash) or a backtick (PowerShell) and the line
/// break after it. `0` when there is none.
fn continuation_at(chars: &[char], i: usize) -> usize {
    if !matches!(chars.get(i), Some('\\' | '`')) {
        return 0;
    }
    match (chars.get(i + 1), chars.get(i + 2)) {
        (Some('\n'), _) => 2,
        (Some('\r'), Some('\n')) => 3,
        _ => 0,
    }
}

/// Up to `max` digits of `radix` at `i`, as a character.
fn numeric_escape(chars: &[char], i: &mut usize, radix: u32, max: usize) -> Option<char> {
    let mut code = 0u32;
    let mut read = 0;
    while read < max {
        let Some(digit) = chars.get(*i).and_then(|c| c.to_digit(radix)) else { break };
        code = code * radix + digit;
        *i += 1;
        read += 1;
    }
    if read == 0 { None } else { char::from_u32(code) }
}

/// The inside of a `$'…'` string, which is where a shell spells out line breaks and non-ASCII text.
fn read_ansi_c(chars: &[char], i: &mut usize, token: &mut String) {
    while *i < chars.len() && chars[*i] != '\'' {
        if chars[*i] != '\\' || *i + 1 >= chars.len() {
            token.push(chars[*i]);
            *i += 1;
            continue;
        }
        let escape = chars[*i + 1];
        *i += 2;
        let decoded = match escape {
            'n' => Some('\n'),
            't' => Some('\t'),
            'r' => Some('\r'),
            'a' => Some('\u{7}'),
            'b' => Some('\u{8}'),
            'e' | 'E' => Some('\u{1b}'),
            'f' => Some('\u{c}'),
            'v' => Some('\u{b}'),
            '\\' | '\'' | '"' | '?' => Some(escape),
            'x' => numeric_escape(chars, i, 16, 2),
            'u' => numeric_escape(chars, i, 16, 4),
            'U' => numeric_escape(chars, i, 16, 8),
            '0'..='7' => {
                *i -= 1;
                numeric_escape(chars, i, 8, 3)
            }
            _ => None,
        };
        match decoded {
            Some(c) => token.push(c),
            None => {
                token.push('\\');
                token.push(escape);
            }
        }
    }
    *i += 1;
}

/// A command copied for the Windows prompt escapes with carets (`curl ^"http://h/^" ^`). With the carets
/// taken out it reads like any other. `None` when the text is not written that way.
fn without_cmd_carets(input: &str) -> Option<String> {
    let after_curl = {
        let start = input.trim_start();
        let rest = start.strip_prefix("curl.exe").or_else(|| start.strip_prefix("curl")).unwrap_or(start);
        rest.trim_start()
    };
    let continued = input.lines().any(|line| line.trim_end().ends_with('^'));
    if !continued && !after_curl.starts_with("^\"") {
        return None;
    }
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '^' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        match (chars.get(i + 1), chars.get(i + 2)) {
            (Some('\n'), _) => {
                out.push(' ');
                i += 2;
            }
            (Some('\r'), Some('\n')) => {
                out.push(' ');
                i += 3;
            }
            (Some(next), _) => {
                out.push(*next);
                i += 2;
            }
            (None, _) => i += 1,
        }
    }
    Some(out)
}

/// Shell tokenizer: single quotes are literal, double quotes honour the escapes a shell does, `$'…'` is
/// decoded, quoted pieces that touch are one word, and a bare word ends at whitespace. A backslash or a
/// backtick before a line break continues the line.
pub fn tokenize_curl(input: &str) -> Vec<String> {
    let unescaped = without_cmd_carets(input);
    let chars: Vec<char> = unescaped.as_deref().unwrap_or(input).chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    loop {
        while i < chars.len() {
            let skip = continuation_at(&chars, i);
            if skip > 0 {
                i += skip;
            } else if is_space(chars[i]) {
                i += 1;
            } else {
                break;
            }
        }
        if i >= chars.len() {
            break;
        }
        let mut token = String::new();
        while i < chars.len() && !is_space(chars[i]) {
            let skip = continuation_at(&chars, i);
            if skip > 0 {
                i += skip;
                break;
            }
            match chars[i] {
                '\'' => {
                    i += 1;
                    while i < chars.len() && chars[i] != '\'' {
                        token.push(chars[i]);
                        i += 1;
                    }
                    i += 1;
                }
                '"' => {
                    i += 1;
                    while i < chars.len() && chars[i] != '"' {
                        let skip = continuation_at(&chars, i);
                        if chars[i] == '\\' && skip > 0 {
                            i += skip;
                            continue;
                        }
                        if chars[i] == '\\' && matches!(chars.get(i + 1), Some('"' | '\\' | '$' | '`')) {
                            i += 1;
                        }
                        token.push(chars[i]);
                        i += 1;
                    }
                    i += 1;
                }
                '$' if chars.get(i + 1) == Some(&'\'') => {
                    i += 2;
                    read_ansi_c(&chars, &mut i, &mut token);
                }
                '\\' if i + 1 < chars.len() => {
                    token.push(chars[i + 1]);
                    i += 2;
                }
                c => {
                    token.push(c);
                    i += 1;
                }
            }
        }
        tokens.push(token);
    }
    tokens
}

/// The short options that take a value, which is the rest of their word or the next one.
const SHORT_WITH_VALUE: &str = "AbcCdDeEFHKmoPQrtTuUwxXyYz";

/// The options this parser reads, by their long names.
const READ_WITH_VALUE: [&str; 16] = [
    "request",
    "header",
    "data",
    "data-raw",
    "data-binary",
    "data-ascii",
    "data-urlencode",
    "json",
    "form",
    "form-string",
    "user",
    "user-agent",
    "referer",
    "cookie",
    "url",
    "oauth2-bearer",
];

/// Options about the transfer rather than the request. Their value is stepped over so it is not taken for
/// the URL. An option in neither list is a switch.
const SKIPPED_WITH_VALUE: [&str; 70] = [
    "abstract-unix-socket",
    "alt-svc",
    "aws-sigv4",
    "cacert",
    "capath",
    "cert",
    "cert-type",
    "ciphers",
    "config",
    "connect-timeout",
    "connect-to",
    "continue-at",
    "cookie-jar",
    "crlfile",
    "curves",
    "dns-interface",
    "dns-servers",
    "doh-url",
    "dump-header",
    "engine",
    "etag-compare",
    "etag-save",
    "expect100-timeout",
    "happy-eyeballs-timeout-ms",
    "hsts",
    "interface",
    "keepalive-time",
    "key",
    "key-type",
    "libcurl",
    "limit-rate",
    "local-port",
    "max-filesize",
    "max-redirs",
    "max-time",
    "netrc-file",
    "noproxy",
    "output",
    "output-dir",
    "pass",
    "pinnedpubkey",
    "preproxy",
    "proto",
    "proto-default",
    "proto-redir",
    "proxy",
    "proxy-cacert",
    "proxy-cert",
    "proxy-header",
    "proxy-key",
    "proxy-pass",
    "proxy-user",
    "pubkey",
    "quote",
    "range",
    "rate",
    "request-target",
    "resolve",
    "retry",
    "retry-delay",
    "retry-max-time",
    "socks5",
    "socks5-hostname",
    "speed-limit",
    "speed-time",
    "stderr",
    "time-cond",
    "tls-max",
    "unix-socket",
    "upload-file",
];

fn takes_value(name: &str) -> bool {
    READ_WITH_VALUE.contains(&name)
        || SKIPPED_WITH_VALUE.contains(&name)
        || matches!(name, "trace" | "trace-ascii" | "write-out" | "telnet-option" | "ftp-port")
}

/// The long name of a short option this parser reads.
fn long_name(short: char) -> Option<&'static str> {
    Some(match short {
        'X' => "request",
        'H' => "header",
        'd' => "data",
        'F' => "form",
        'u' => "user",
        'A' => "user-agent",
        'e' => "referer",
        'b' => "cookie",
        'G' => "get",
        'I' => "head",
        _ => return None,
    })
}

/// Everything but letters, digits and `-._~` is escaped, a space as `%20`: what `--data-urlencode` sends.
const URLENCODE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

/// A `--data-urlencode` value: `content`, `=content` or `name=content`; only the content is encoded.
fn urlencoded(value: &str) -> String {
    let encode = |text: &str| utf8_percent_encode(text, URLENCODE).to_string();
    match value.split_once('=') {
        Some(("", content)) => encode(content),
        Some((name, content)) => format!("{name}={}", encode(content)),
        None => encode(value),
    }
}

fn unquoted(text: &str) -> &str {
    text.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')).unwrap_or(text)
}

/// A `-F name=value` field. `@path` names a file to send; `;type=` and `;filename=` describe it.
fn form_part(value: &str, literal: bool) -> CurlPart {
    let (name, rest) = value.split_once('=').unwrap_or((value, ""));
    let mut part = CurlPart { name: name.to_string(), ..CurlPart::default() };
    if literal {
        part.value = rest.to_string();
        return part;
    }
    let mut content = String::new();
    for (ix, piece) in rest.split(';').enumerate() {
        if ix == 0 {
            content.push_str(piece);
        } else if let Some(content_type) = piece.strip_prefix("type=") {
            part.content_type = unquoted(content_type).to_string();
        } else if let Some(file_name) = piece.strip_prefix("filename=") {
            part.file_name = unquoted(file_name).to_string();
        } else if !piece.starts_with("headers=") {
            content.push(';');
            content.push_str(piece);
        }
    }
    if let Some(path) = content.strip_prefix('@') {
        part.file = true;
        if part.file_name.is_empty() {
            part.file_name = unquoted(path).rsplit(['/', '\\']).next().unwrap_or("").to_string();
        }
    } else if !content.starts_with('<') {
        // `<path` is a text field read from a file that is not here; it stays empty.
        part.value = unquoted(&content).to_string();
    }
    part
}

/// The command as its options are read, before they are settled into one request.
#[derive(Default)]
struct Draft {
    method: Option<String>,
    url: Option<String>,
    positional: Vec<String>,
    headers: Vec<(String, String)>,
    data: Vec<String>,
    json: bool,
    parts: Vec<CurlPart>,
    get: bool,
    head: bool,
    user: Option<String>,
    bearer: Option<String>,
}

impl Draft {
    fn has_header(&self, name: &str) -> bool {
        self.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
    }

    /// A header named twice is sent with the later value.
    fn set_header(&mut self, name: &str, value: &str) {
        match self.headers.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some(header) => header.1 = value.to_string(),
            None => self.headers.push((name.to_string(), value.to_string())),
        }
    }

    fn option(&mut self, name: &str, value: &str) {
        match name {
            "request" => self.method = Some(value.to_uppercase()),
            "header" => {
                // `Name: value` sets a header, `Name;` sends it empty, `@file` reads headers from a file.
                if let Some(colon) = value.find(':').filter(|colon| *colon > 0) {
                    self.set_header(value[..colon].trim(), value[colon + 1..].trim());
                } else if let Some(name) = value.strip_suffix(';').filter(|name| !name.trim().is_empty()) {
                    self.set_header(name.trim(), "");
                }
            }
            "cookie" => {
                let cookie = match self.headers.iter().find(|(n, v)| n.eq_ignore_ascii_case("cookie") && !v.is_empty())
                {
                    Some((_, existing)) => format!("{existing}; {value}"),
                    None => value.to_string(),
                };
                let name = if self.has_header("cookie") { "cookie" } else { "Cookie" };
                self.set_header(name, &cookie);
            }
            "data" | "data-raw" | "data-binary" | "data-ascii" => self.data.push(value.to_string()),
            "data-urlencode" => self.data.push(urlencoded(value)),
            "json" => {
                self.data.push(value.to_string());
                self.json = true;
            }
            "form" => self.parts.push(form_part(value, false)),
            "form-string" => self.parts.push(form_part(value, true)),
            "user" => self.user = Some(value.to_string()),
            "oauth2-bearer" => self.bearer = Some(value.to_string()),
            "user-agent" => self.set_header("User-Agent", value),
            "referer" => self.set_header("Referer", value.split(";auto").next().unwrap_or(value)),
            "url" => self.url = Some(value.to_string()),
            _ => {}
        }
    }

    fn switch(&mut self, name: &str) {
        match name {
            "get" => self.get = true,
            "head" => self.head = true,
            _ => {}
        }
    }

    /// Settles the options into one request. `is_curl` is whether the text began with `curl`: only then is
    /// a word that is not a full URL taken for one.
    fn finish(mut self, is_curl: bool) -> CurlRequest {
        let is_full = |word: &String| word.contains("://") || word.starts_with("{{");
        let positional = if is_curl {
            let full = self.positional.iter().find(|word| is_full(word));
            full.or_else(|| self.positional.iter().find(|word| !word.is_empty())).cloned()
        } else {
            self.positional.iter().find(|word| word.starts_with("http://") || word.starts_with("https://")).cloned()
        };
        let mut url = self.url.take().or(positional).unwrap_or_default();
        // curl assumes HTTP for an address that names no scheme.
        if is_curl && !url.is_empty() && !is_full(&url) {
            url = format!("http://{url}");
        }

        let mut body = if self.json { self.data.concat() } else { self.data.join("&") };
        if self.json {
            for name in ["Content-Type", "Accept"] {
                if !self.has_header(name) {
                    self.set_header(name, "application/json");
                }
            }
        }
        if self.get && !body.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&body);
            body.clear();
        }
        if !self.has_header("authorization") {
            if let Some(user) = &self.user {
                let value = format!("Basic {}", STANDARD.encode(user));
                self.set_header("Authorization", &value);
            } else if let Some(token) = &self.bearer {
                let value = format!("Bearer {token}");
                self.set_header("Authorization", &value);
            }
        }
        let method = self.method.take().unwrap_or_else(|| {
            if self.head {
                "HEAD"
            } else if self.get || (body.is_empty() && self.parts.is_empty()) {
                "GET"
            } else {
                "POST"
            }
            .to_string()
        });
        CurlRequest { method, url, headers: self.headers, body, parts: self.parts }
    }
}

fn is_curl_word(word: &str) -> bool {
    word.eq_ignore_ascii_case("curl") || word.eq_ignore_ascii_case("curl.exe")
}

fn read_options(tokens: &[String], is_curl: bool) -> CurlRequest {
    let mut draft = Draft::default();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i].as_str();
        i += 1;
        if is_curl_word(token) {
            continue;
        }
        // What follows a pipe or a second command is not this request.
        if is_curl && matches!(token, "|" | ";" | "&&" | "||") {
            break;
        }
        if let Some(long) = token.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) if takes_value(name) => (name, Some(value.to_string())),
                _ => (long, None),
            };
            if !takes_value(name) {
                draft.switch(name);
                continue;
            }
            let value = inline.or_else(|| {
                i += 1;
                tokens.get(i - 1).cloned()
            });
            if let Some(value) = value {
                draft.option(name, &value);
            }
        } else if token.len() > 1 && token.starts_with('-') {
            let cluster: Vec<char> = token[1..].chars().collect();
            for (ix, short) in cluster.iter().enumerate() {
                if !SHORT_WITH_VALUE.contains(*short) {
                    if let Some(name) = long_name(*short) {
                        draft.switch(name);
                    }
                    continue;
                }
                let rest: String = cluster[ix + 1..].iter().collect();
                let value = if rest.is_empty() {
                    i += 1;
                    tokens.get(i - 1).cloned()
                } else {
                    Some(rest)
                };
                if let (Some(name), Some(value)) = (long_name(*short), value) {
                    draft.option(name, &value);
                }
                break;
            }
        } else {
            draft.positional.push(token.to_string());
        }
    }
    draft.finish(is_curl)
}

/// Parse a cURL command. Never fails: text that is not a cURL command parses to an empty request. Headers
/// are returned unfiltered; dropping [`SKIP_CURL_HEADERS`] is the caller's decision.
pub fn parse_curl_request(curl: &str) -> ParsedCurl {
    let tokens = tokenize_curl(curl);
    let is_curl = tokens.first().is_some_and(|first| is_curl_word(first));
    read_options(&tokens, is_curl).into_parsed()
}

/// [`parse_curl_request`] in its wire shape.
pub fn parse_curl(curl: &str) -> Value {
    parse_curl_request(curl).to_value()
}

/// Parse text that has to be a cURL command, or say why it is not one. A shell prompt before `curl` is
/// stepped over.
pub fn parse_curl_strict(curl: &str) -> Result<CurlRequest, &'static str> {
    let tokens = tokenize_curl(curl);
    let start = usize::from(tokens.first().is_some_and(|first| matches!(first.as_str(), "$" | ">" | "%" | "#")));
    let tokens = &tokens[start.min(tokens.len())..];
    let Some(first) = tokens.first() else { return Err("Paste a cURL command.") };
    if !is_curl_word(first) {
        return Err("This is not a cURL command: it has to start with \"curl\".");
    }
    let request = read_options(tokens, true);
    if request.url.is_empty() {
        return Err("The cURL command names no URL.");
    }
    Ok(request)
}

/// The importer's contract: a block is a cURL command only if it starts with `curl` and yields a URL; its
/// headers are filtered.
pub fn parse_curl_command(curl: &str) -> Option<ParsedCurl> {
    let mut parsed = parse_curl_strict(curl.trim()).ok()?.into_parsed();
    parsed.headers.retain(|name, _| !is_skipped_header(name));
    Some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_simple_get() {
        let parsed = parse_curl_request("curl https://api.example.com/users?id=1");
        assert_eq!(parsed.method, "GET");
        assert_eq!(parsed.url, "https://api.example.com/users?id=1");
        assert!(parsed.headers.is_empty());
        assert_eq!(parsed.body, "");
    }

    #[test]
    fn headers_body_and_method() {
        let parsed = parse_curl_request(
            "curl -X put 'https://x.dev/a' -H 'Content-Type: application/json' -H \"X-Token: a\\\"b\" -d '{\"a\": 1}'",
        );
        assert_eq!(parsed.method, "PUT");
        assert_eq!(parsed.url, "https://x.dev/a");
        assert_eq!(parsed.headers.get("content-type").unwrap(), "application/json");
        assert_eq!(parsed.headers.get("x-token").unwrap(), "a\"b");
        assert_eq!(parsed.body, "{\"a\": 1}");
    }

    #[test]
    fn data_implies_post_and_continuation_lines_join() {
        let parsed = parse_curl_request("curl 'http://h/p' \\\n  --data-raw 'a=1&b=2' \\\r\n  -b 'a=1' -b 'b=2'");
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.body, "a=1&b=2");
        assert_eq!(parsed.headers.get("cookie").unwrap(), "a=1; b=2");
    }

    #[test]
    fn url_flag_and_first_url_wins() {
        let parsed = parse_curl_request("curl --request DELETE --url http://one/ http://two/");
        assert_eq!(parsed.method, "DELETE");
        assert_eq!(parsed.url, "http://one/");
    }

    #[test]
    fn header_without_a_name_is_ignored_and_values_keep_colons() {
        let parsed = parse_curl_request("curl http://h -H ': nope' -H 'Referer: http://a:80/'");
        assert_eq!(parsed.headers.len(), 1);
        assert_eq!(parsed.headers.get("referer").unwrap(), "http://a:80/");
    }

    #[test]
    fn not_a_curl_command_parses_to_an_empty_request() {
        assert_eq!(parse_curl("hello"), json!({ "method": "GET", "url": "", "headers": {}, "body": "" }));
    }

    #[test]
    fn wire_shape_is_ordered() {
        let value = parse_curl("curl http://h -H 'A: b' -d x");
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"method":"POST","url":"http://h","headers":{"a":"b"},"body":"x"}"#
        );
    }

    #[test]
    fn importer_wrapper_requires_curl_and_filters_headers() {
        assert!(parse_curl_command("wget http://h").is_none());
        assert!(parse_curl_command("curl -X POST").is_none());
        let parsed = parse_curl_command("  curl http://h -H 'Host: h' -H 'Accept: */*'  ").unwrap();
        assert_eq!(parsed.headers.len(), 1);
        assert_eq!(parsed.headers.get("accept").unwrap(), "*/*");
    }

    fn strict(curl: &str) -> CurlRequest {
        parse_curl_strict(curl).unwrap()
    }

    #[test]
    fn quoting_follows_the_shell() {
        // Pieces that touch are one word, and a double-quoted backslash stays unless it escapes.
        assert_eq!(tokenize_curl(r#"-H'A: 'b"c" "x\ny\\z\"" a\ b"#), ["-HA: bc", r#"x\ny\z""#, "a b"]);
        assert_eq!(tokenize_curl(r"$'a\nb\'c\u00e9\x41\101'"), ["a\nb'c\u{e9}AA"]);
        assert_eq!(tokenize_curl("a ''  b"), ["a", "", "b"]);
    }

    #[test]
    fn a_command_copied_for_the_windows_prompt() {
        let request = strict(
            "curl ^\"https://h.test/a?x=1^&y=2^\" ^\r\n  -H ^\"accept: */*^\" ^\r\n  --data-raw ^\"^{^\\^\"a^\\^\":1^}^\"",
        );
        assert_eq!(request.url, "https://h.test/a?x=1&y=2");
        assert_eq!(request.header("accept"), Some("*/*"));
        assert_eq!(request.body, "{\"a\":1}");
        assert_eq!(request.method, "POST");
    }

    #[test]
    fn a_powershell_continuation_and_curl_exe() {
        let request = strict("curl.exe -X PATCH `\r\n  https://h.test/a `\n  -H 'A: 1'");
        assert_eq!((request.method.as_str(), request.url.as_str()), ("PATCH", "https://h.test/a"));
        assert_eq!(request.headers, [("A".to_string(), "1".to_string())]);
    }

    #[test]
    fn clustered_and_inline_options() {
        let request = strict("curl -sSLXPOST -H'A: 1' --header=B:2 --data='x=1' -dy=2 https://h.test/");
        assert_eq!(request.method, "POST");
        assert_eq!(request.headers, [("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())]);
        assert_eq!(request.body, "x=1&y=2");
    }

    #[test]
    fn the_value_of_a_transfer_option_is_not_the_url() {
        let request = strict("curl -x http://proxy:8080 -o out.json --max-time 5 -w '%{http_code}' --compressed h.test/a");
        assert_eq!(request.url, "http://h.test/a");
        assert_eq!(request.method, "GET");
        let request = strict("curl --retry 3 localhost:3000/x | jq .");
        assert_eq!(request.url, "http://localhost:3000/x");
    }

    #[test]
    fn a_url_keeps_its_own_scheme_or_variable() {
        assert_eq!(strict("curl wss://h.test/socket").url, "wss://h.test/socket");
        assert_eq!(strict("curl {{baseUrl}}/users").url, "{{baseUrl}}/users");
    }

    #[test]
    fn get_moves_the_data_into_the_query_and_head_is_a_method() {
        let request = strict("curl -G https://h.test/s?a=1 -d b=2 --data-urlencode 'q=a b&c' --data-urlencode '=x y'");
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, "https://h.test/s?a=1&b=2&q=a%20b%26c&x%20y");
        assert_eq!(request.body, "");
        assert_eq!(strict("curl -I https://h.test/").method, "HEAD");
    }

    #[test]
    fn credentials_and_agent_become_headers() {
        let request = strict("curl -u user:pass -A 'agent/1' -e 'http://from/' https://h.test/");
        assert_eq!(request.header("authorization"), Some("Basic dXNlcjpwYXNz"));
        assert_eq!(request.header("user-agent"), Some("agent/1"));
        assert_eq!(request.header("referer"), Some("http://from/"));
        let request = strict("curl --oauth2-bearer tok https://h.test/");
        assert_eq!(request.header("Authorization"), Some("Bearer tok"));
        // A header that was written out is not replaced.
        let request = strict("curl -u a:b -H 'Authorization: Bearer mine' https://h.test/");
        assert_eq!(request.header("authorization"), Some("Bearer mine"));
    }

    #[test]
    fn json_sets_its_headers_and_a_later_header_wins() {
        let request = strict("curl --json '{\"a\":1}' https://h.test/ -H 'accept: text/plain' -H 'X: 1' -H 'x: 2' -H 'Empty;'");
        assert_eq!(request.method, "POST");
        assert_eq!(request.body, "{\"a\":1}");
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(request.header("accept"), Some("text/plain"));
        assert_eq!(request.header("x"), Some("2"));
        assert_eq!(request.header("empty"), Some(""));
    }

    #[test]
    fn form_fields_become_parts() {
        let request = strict(
            "curl https://h.test/up -F 'name=Ann;x' -F 'doc=@/tmp/a b.pdf;type=application/pdf' -F 'pic=@c:\\p\\i.png;filename=me.png' --form-string 'raw=@not;type=x'",
        );
        assert_eq!(request.method, "POST");
        assert_eq!(request.parts.len(), 4);
        assert_eq!(request.parts[0], CurlPart { name: "name".into(), value: "Ann;x".into(), ..CurlPart::default() });
        assert!(request.parts[1].file);
        assert_eq!(request.parts[1].file_name, "a b.pdf");
        assert_eq!(request.parts[1].content_type, "application/pdf");
        assert_eq!(request.parts[2].file_name, "me.png");
        assert_eq!(request.parts[3].value, "@not;type=x");
        assert!(!request.parts[3].file);
    }

    #[test]
    fn strict_parsing_says_what_is_wrong() {
        assert_eq!(parse_curl_strict("  \n").unwrap_err(), "Paste a cURL command.");
        assert!(parse_curl_strict("wget http://h").unwrap_err().contains("not a cURL command"));
        assert_eq!(parse_curl_strict("curl -X POST").unwrap_err(), "The cURL command names no URL.");
        assert_eq!(strict("$ curl http://h/").url, "http://h/");
    }
}
