//! The small HTML pages the proxies write to a raw socket (home, 404, unserved, 502).

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "",
    }
}

/// The page chrome. The stylesheet is inline on purpose: the page is served on a connection that is about
/// to close, with no static file root to fetch a `<link>` from.
fn wrap_page(body: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\"><meta name=\"viewport\" \
         content=\"width=device-width,initial-scale=1\"><title>Bifurc</title><style>*{{box-sizing:border-box;margin:0;\
         padding:0}}body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif;\
         background:#0f1117;color:#c9d1e9;min-height:100vh;display:flex;align-items:center;justify-content:center;\
         padding:2rem}}.wrap{{background:#171b27;border:1px solid #2d3550;border-radius:12px;padding:2.5rem 3rem;\
         max-width:560px;width:100%}}.logo{{display:flex;align-items:center;gap:10px;margin-bottom:1.5rem}}\
         .logo-icon{{width:28px;height:28px;flex-shrink:0}}\
         .logo-name{{font-size:13px;font-weight:700;color:#e8eaf6;letter-spacing:.02em}}.badge{{display:inline-flex;\
         padding:3px 10px;border-radius:20px;border:1px solid #2d3550;font-size:10px;font-weight:700;\
         letter-spacing:.1em;text-transform:uppercase;color:#6b7799;margin-bottom:1rem}}h1{{font-size:22px;\
         font-weight:700;color:#e8eaf6;margin-bottom:.6rem;word-break:break-all}}p{{font-size:13px;color:#6b7799;\
         line-height:1.6;margin-bottom:.75rem}}a{{color:#5b8dee;text-decoration:none}}a:hover{{text-decoration:\
         underline}}code{{font-family:monospace;background:#262d3f;padding:2px 6px;border-radius:4px;font-size:12px;\
         color:#c9d1e9}}ul{{list-style:none;margin-top:1rem}}li{{padding:10px 0;border-bottom:1px solid #2d3550;\
         display:flex;align-items:center;gap:10px;font-size:13px}}li:last-child{{border-bottom:none}}.dot{{width:6px;\
         height:6px;border-radius:50%;background:#3ecf8e;flex-shrink:0;box-shadow:0 0 6px \
         #3ecf8e}}.domain{{font-family:monospace;color:#e8eaf6;font-weight:600}}.arr{{color:#2d3550;margin:0 \
         4px}}.tgt{{font-family:monospace;font-size:12px;color:#6b7799}}</style></head><body><div \
         class=\"wrap\">{body}</div></body></html>"
    )
}

/// The complete HTTP/1.1 response for a small HTML page: status line, headers (`connection: close`) and
/// the page wrapped in the standard chrome. The caller writes it to the socket and closes it.
pub fn html_response(status: u16, html_body: &str) -> Vec<u8> {
    let page = wrap_page(html_body);
    let mut out = format!(
        "HTTP/1.1 {status} {}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: \
         close\r\n\r\n",
        reason_phrase(status),
        page.len()
    )
    .into_bytes();
    out.extend_from_slice(page.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_is_head_then_wrapped_page() {
        let bytes = html_response(404, "<h1>nope</h1>");
        let text = String::from_utf8(bytes).unwrap();
        let (head, page) = text.split_once("\r\n\r\n").unwrap();
        let lines: Vec<&str> = head.split("\r\n").collect();
        assert_eq!(lines[0], "HTTP/1.1 404 Not Found");
        assert_eq!(lines[1], "content-type: text/html; charset=utf-8");
        assert_eq!(lines[2], format!("content-length: {}", page.len()));
        assert_eq!(lines[3], "connection: close");
        assert!(page.starts_with("<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"UTF-8\">"));
        assert!(page.contains("*{box-sizing:border-box;margin:0;padding:0}body{font-family:-apple-system,"));
        assert!(page.contains("a:hover{text-decoration:underline}code{"));
        assert!(page.contains("box-shadow:0 0 6px #3ecf8e}.domain{"));
        assert!(page.ends_with("<div class=\"wrap\"><h1>nope</h1></div></body></html>"));
    }

    #[test]
    fn content_length_counts_bytes_and_unknown_status_has_no_reason() {
        let text = String::from_utf8(html_response(418, "é")).unwrap();
        let (head, page) = text.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 418 \r\n"));
        assert!(head.contains(&format!("content-length: {}", page.len())));
    }
}
