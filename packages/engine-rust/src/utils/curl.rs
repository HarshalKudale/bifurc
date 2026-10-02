//! The cURL command parser, shared by `request.parseCurl` and the cURL importer.

use serde_json::{Value, json};

use crate::kernel::telemetry::Headers;

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

/// Shell-ish tokenizer: single quotes are literal, double quotes honour backslash escapes, bare words end at
/// a space or tab. A backslash-newline continues the line.
pub fn tokenize_curl(input: &str) -> Vec<String> {
    let joined = input.replace("\\\r\n", " ").replace("\\\n", " ");
    let chars: Vec<char> = joined.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut token = String::new();
        if chars[i] == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                token.push(chars[i]);
                i += 1;
            }
            i += 1;
        } else if chars[i] == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                token.push(chars[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < chars.len() && chars[i] != ' ' && chars[i] != '\t' {
                token.push(chars[i]);
                i += 1;
            }
        }
        tokens.push(token);
    }
    tokens
}

/// Parse a cURL command. Never fails: text that is not a cURL command parses to an empty request. Headers
/// are returned unfiltered; dropping [`SKIP_CURL_HEADERS`] is the caller's decision.
pub fn parse_curl_request(curl: &str) -> ParsedCurl {
    let tokens = tokenize_curl(curl);
    let mut method = String::new();
    let mut url = String::new();
    let mut headers = Headers::new();
    let mut body = String::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i].as_str();
        if token == "curl" {
            i += 1;
            continue;
        }
        match token {
            "-X" | "--request" => {
                i += 1;
                if let Some(value) = tokens.get(i) {
                    method = value.to_uppercase();
                }
            }
            "-H" | "--header" => {
                i += 1;
                if let Some(value) = tokens.get(i) {
                    if let Some(colon) = value.find(':').filter(|colon| *colon > 0) {
                        headers.insert(value[..colon].trim().to_lowercase(), value[colon + 1..].trim().to_string());
                    }
                }
            }
            "-b" | "--cookie" => {
                i += 1;
                if let Some(value) = tokens.get(i) {
                    let cookie = match headers.get("cookie").filter(|existing| !existing.is_empty()) {
                        Some(existing) => format!("{existing}; {value}"),
                        None => value.clone(),
                    };
                    headers.insert("cookie".to_string(), cookie);
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" => {
                i += 1;
                if let Some(value) = tokens.get(i) {
                    body = value.clone();
                }
            }
            "--url" => {
                i += 1;
                if let Some(value) = tokens.get(i) {
                    url = value.clone();
                }
            }
            _ => {
                if !token.starts_with('-')
                    && url.is_empty()
                    && (token.starts_with("http://") || token.starts_with("https://"))
                {
                    url = token.to_string();
                }
            }
        }
        i += 1;
    }
    if method.is_empty() {
        method = if body.is_empty() { "GET" } else { "POST" }.to_string();
    }
    ParsedCurl { method, url, headers, body }
}

/// [`parse_curl_request`] in its wire shape.
pub fn parse_curl(curl: &str) -> Value {
    parse_curl_request(curl).to_value()
}

/// The importer's contract: a block is a cURL command only if it starts with `curl` and yields a URL; its
/// headers are filtered.
pub fn parse_curl_command(curl: &str) -> Option<ParsedCurl> {
    let trimmed = curl.trim();
    let tokens = tokenize_curl(trimmed);
    if tokens.first().map(String::as_str) != Some("curl") {
        return None;
    }
    let mut parsed = parse_curl_request(trimmed);
    if parsed.url.is_empty() {
        return None;
    }
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
}
