//! A request written out as code someone can paste elsewhere: cURL, `fetch`, axios, Python `requests`.
//!
//! This is formatting, not sending. The request is written exactly as the editor holds it, `{{variables}}`
//! included; a snippet is something to read and adapt, and substituting values is the engine's job when it
//! actually sends.

use crate::i18n::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnippetKind {
    Curl,
    Fetch,
    Axios,
    Python,
}

impl SnippetKind {
    pub const ALL: [SnippetKind; 4] = [SnippetKind::Curl, SnippetKind::Fetch, SnippetKind::Axios, SnippetKind::Python];

    pub fn label(self) -> &'static str {
        t(match self {
            SnippetKind::Curl => "model.snippet.curl",
            SnippetKind::Fetch => "model.snippet.fetch",
            SnippetKind::Axios => "model.snippet.axios",
            SnippetKind::Python => "model.snippet.python",
        })
    }
}

/// The parts of a request a snippet is made from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestParts {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl RequestParts {
    fn has_body(&self) -> bool {
        !self.body.is_empty() && !matches!(self.method.as_str(), "GET" | "HEAD")
    }
}

/// Quotes `text` for a POSIX shell: single quotes, with an embedded single quote closed, escaped and reopened.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Quotes `text` as a JavaScript or Python string literal. JSON's escaping is valid in both.
fn code_quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned())
}

pub fn snippet(kind: SnippetKind, request: &RequestParts) -> String {
    match kind {
        SnippetKind::Curl => curl(request),
        SnippetKind::Fetch => fetch(request),
        SnippetKind::Axios => axios(request),
        SnippetKind::Python => python(request),
    }
}

fn curl(request: &RequestParts) -> String {
    let mut lines = vec![format!("curl -X {} {}", request.method, shell_quote(&request.url))];
    for (name, value) in &request.headers {
        lines.push(format!("  -H {}", shell_quote(&format!("{name}: {value}"))));
    }
    if request.has_body() {
        lines.push(format!("  --data-raw {}", shell_quote(&request.body)));
    }
    lines.join(" \\\n")
}

fn header_object(request: &RequestParts, indent: &str) -> String {
    let rows: Vec<String> = request
        .headers
        .iter()
        .map(|(name, value)| format!("{indent}  {}: {},", code_quote(name), code_quote(value)))
        .collect();
    format!("{{\n{}\n{indent}}}", rows.join("\n"))
}

fn fetch(request: &RequestParts) -> String {
    let mut options = vec![format!("  method: {},", code_quote(&request.method))];
    if !request.headers.is_empty() {
        options.push(format!("  headers: {},", header_object(request, "  ")));
    }
    if request.has_body() {
        options.push(format!("  body: {},", code_quote(&request.body)));
    }
    format!(
        "const response = await fetch({}, {{\n{}\n}});\nconst text = await response.text();",
        code_quote(&request.url),
        options.join("\n")
    )
}

fn axios(request: &RequestParts) -> String {
    let mut options = vec![
        format!("  method: {},", code_quote(&request.method.to_lowercase())),
        format!("  url: {},", code_quote(&request.url)),
    ];
    if !request.headers.is_empty() {
        options.push(format!("  headers: {},", header_object(request, "  ")));
    }
    if request.has_body() {
        options.push(format!("  data: {},", code_quote(&request.body)));
    }
    format!("const response = await axios({{\n{}\n}});", options.join("\n"))
}

fn python(request: &RequestParts) -> String {
    let mut arguments =
        vec![format!("    {},", code_quote(&request.method)), format!("    {},", code_quote(&request.url))];
    if !request.headers.is_empty() {
        let rows: Vec<String> = request
            .headers
            .iter()
            .map(|(name, value)| format!("        {}: {},", code_quote(name), code_quote(value)))
            .collect();
        arguments.push(format!("    headers={{\n{}\n    }},", rows.join("\n")));
    }
    if request.has_body() {
        arguments.push(format!("    data={},", code_quote(&request.body)));
    }
    format!("import requests\n\nresponse = requests.request(\n{}\n)\nprint(response.text)", arguments.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> RequestParts {
        RequestParts {
            method: "POST".into(),
            url: "{{HOST}}/users?active=1".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: r#"{"name":"O'Neil"}"#.into(),
        }
    }

    #[test]
    fn curl_quotes_for_the_shell_and_keeps_variables_as_written() {
        let text = snippet(SnippetKind::Curl, &post());
        assert_eq!(
            text,
            "curl -X POST '{{HOST}}/users?active=1' \\\n  -H 'Content-Type: application/json' \\\n  --data-raw '{\"name\":\"O'\\''Neil\"}'"
        );
    }

    #[test]
    fn a_get_has_no_body_even_if_the_editor_still_holds_one() {
        let get =
            RequestParts { method: "GET".into(), url: "http://x".into(), headers: vec![], body: "left over".into() };
        assert_eq!(snippet(SnippetKind::Curl, &get), "curl -X GET 'http://x'");
        assert!(!snippet(SnippetKind::Fetch, &get).contains("body"));
        assert!(!snippet(SnippetKind::Python, &get).contains("data="));
    }

    #[test]
    fn code_snippets_escape_strings_as_literals() {
        let text = snippet(SnippetKind::Fetch, &post());
        assert!(text.contains(r#"fetch("{{HOST}}/users?active=1", {"#));
        assert!(text.contains(r#"body: "{\"name\":\"O'Neil\"}","#));
        assert!(text.contains(r#""Content-Type": "application/json","#));
    }

    #[test]
    fn axios_names_the_method_in_lower_case_and_python_passes_it_first() {
        assert!(snippet(SnippetKind::Axios, &post()).contains(r#"method: "post","#));
        assert!(
            snippet(SnippetKind::Python, &post())
                .contains("requests.request(\n    \"POST\",\n    \"{{HOST}}/users?active=1\",")
        );
    }
}
