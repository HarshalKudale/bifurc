//! The edge's three HTML pages: the home page, the "not mapped" 404 and the "not serving" 503. They only build
//! the page body; `utils::send_html` wraps and sends it.

use serde_json::Value;

use crate::utils::json::{array_of, str_of, truthy};

/// Text safe to place inside HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `:port`, or nothing for the port a scheme is served on by default.
fn port_suffix(scheme: &str, port: u16) -> String {
    let default = if scheme == "https" { 443 } else { 80 };
    if port == default { String::new() } else { format!(":{port}") }
}

const LOGO: &str = "<div class=\"logo\"><div class=\"logo-icon\"></div><span class=\"logo-name\">Bifurc</span></div>";

/// The home page: the enabled mappings of `cfg`, each linking to its domain on the port the page is served on.
pub fn build_home_page(cfg: &Value, scheme: &str, current_port: u16) -> String {
    let suffix = port_suffix(scheme, current_port);
    let rows: String = array_of(cfg, "mappings")
        .iter()
        .filter(|m| truthy(m, "enabled"))
        .map(|m| {
            let (domain, target) = (escape(str_of(m, "domain")), escape(str_of(m, "target")));
            format!(
                "<li><span class=\"dot\"></span><a href=\"{scheme}://{domain}{suffix}/\" class=\"domain\">{domain}</a>\
                 <span class=\"arr\">→</span><span class=\"tgt\">{target}</span></li>"
            )
        })
        .collect();
    let listing = if rows.is_empty() {
        "<p style=\"margin-top:1rem;color:#5b8dee\">No mappings yet. Open the Bifurc app to add one.</p>".to_string()
    } else {
        format!("<ul>{rows}</ul>")
    };
    format!(
        "{LOGO}\n<div class=\"badge\">Port {current_port}</div>\n<h1>Local proxy is active</h1>\n<p>Requests to a mapped \
         domain are routed to that mapping's target. <code>*.localhost{suffix}</code> needs no setup; any other \
         domain needs a DNS or hosts entry pointing at this engine.</p>\n{listing}"
    )
}

/// The 404 for a host no mapping serves.
pub fn build_not_mapped_page(host: &str, scheme: &str, current_port: u16) -> String {
    let (host, suffix) = (escape(host), port_suffix(scheme, current_port));
    format!(
        "{LOGO}\n<div class=\"badge\">Not Mapped</div>\n<h1>{host}</h1>\n<p>This domain is not mapped to any local \
         service.</p>\n<p>Open <a href=\"{scheme}://localhost{suffix}/\">Bifurc</a> and add a mapping for \
         <code>{host}</code>.</p>"
    )
}

/// The 503 for an edge whose host is not serving the active workspace.
pub fn build_unserved_page() -> String {
    format!(
        "{LOGO}\n<div class=\"badge\">Not Serving</div>\n<h1>No workspace is being served</h1>\n<p>The edge is listening, \
         but this host is not running the active workspace — it was skipped (a port override that was taken) or \
         filtered out of the workspace list.</p>\n<p>The host's startup log names the workspace it declined to \
         start, and why.</p>"
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn home_page_lists_enabled_mappings_with_the_port() {
        let cfg = json!({ "mappings": [
            { "domain": "a.localhost", "target": "localhost:3000", "enabled": true },
            { "domain": "off.localhost", "target": "localhost:1", "enabled": false },
        ] });
        let page = build_home_page(&cfg, "http", 8081);
        assert!(page.contains("<div class=\"badge\">Port 8081</div>"));
        assert!(page.contains("<a href=\"http://a.localhost:8081/\" class=\"domain\">a.localhost</a>"));
        assert!(page.contains("<span class=\"tgt\">localhost:3000</span>"));
        assert!(!page.contains("off.localhost"));
        assert!(page.contains("<code>*.localhost:8081</code>"));
    }

    #[test]
    fn port_80_has_no_suffix_and_an_empty_list_says_so() {
        let page = build_home_page(&json!({ "mappings": [] }), "http", 80);
        assert!(page.contains("<code>*.localhost</code>"));
        assert!(page.contains("No mappings yet. Open the Bifurc app to add one."));
        assert!(build_not_mapped_page("x.localhost", "http", 80).contains("<a href=\"http://localhost/\">Bifurc</a>"));
    }

    #[test]
    fn a_host_is_escaped() {
        let page = build_not_mapped_page("<script>", "http", 8080);
        assert!(page.contains("<h1>&lt;script&gt;</h1>"));
    }

    #[test]
    fn unserved_page_says_why() {
        assert!(build_unserved_page().contains("No workspace is being served"));
    }
}
