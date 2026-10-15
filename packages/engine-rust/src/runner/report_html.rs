//! The HTML a collection run report is rendered to, for `runner.exportReport`.

use serde_json::Value;
use time::OffsetDateTime;

use crate::utils::json::{array_of, is_truthy};

/// A JSON value the way a JavaScript template literal prints it; a missing field prints `undefined`.
fn js_text(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => match number.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            _ => number.to_string(),
        },
        Some(other) => other.to_string(),
    }
}

fn esc(value: Option<&Value>) -> String {
    js_text(value).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `new Date(ms).toISOString()`.
pub fn iso_string(ms: i64) -> String {
    let at =
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).unwrap_or(OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

fn number_of(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}

fn result_html(result: &Value) -> String {
    let mut out = String::from("\n<div class=\"request\">\n<div class=\"req-header\">\n");
    out.push_str(&format!("<span class=\"method\">{}</span>\n", esc(result.get("method"))));
    out.push_str(&format!("<span class=\"name\">{}</span>\n", esc(result.get("requestName"))));
    match result.get("status") {
        None | Some(Value::Null) => out.push('\n'),
        status => out.push_str(&format!("<span class=\"status\">{}</span>\n", js_text(status))),
    }
    out.push_str(&format!("<span class=\"time\">{}ms</span>\n</div>\n", js_text(result.get("responseTime"))));
    let tests = array_of(result, "tests");
    let error = result.get("error").filter(|v| is_truthy(v));
    if !tests.is_empty() || error.is_some() {
        out.push_str("<div class=\"tests\">\n");
        match error {
            Some(error) => {
                out.push_str(&format!("<div class=\"test-item failed\">\u{2717} Error: {}</div>\n", esc(Some(error))))
            }
            None => out.push('\n'),
        }
        for test in tests {
            let passed = is_truthy(test.get("passed").unwrap_or(&Value::Null));
            let detail = match test.get("error").filter(|v| is_truthy(v)) {
                Some(error) => format!(" \u{2014} {}", esc(Some(error))),
                None => String::new(),
            };
            out.push_str(&format!(
                "<div class=\"test-item {}\">{} {}{}</div>",
                if passed { "passed" } else { "failed" },
                if passed { "\u{2713}" } else { "\u{2717}" },
                esc(test.get("name")),
                detail
            ));
        }
        out.push_str("\n</div>");
    }
    out.push_str("\n</div>");
    out
}

/// The report as a standalone HTML page.
pub fn render_runner_html(report: &Value) -> String {
    let started = number_of(report.get("startedAt"));
    let completed = report.get("completedAt").and_then(Value::as_f64).unwrap_or(started);
    let duration = format!("{:.2}", (completed - started) / 1000.0);
    let timestamp = iso_string(started as i64);
    let folder = esc(report.get("folderName"));
    let results: String = array_of(report, "results").iter().map(result_html).collect();
    format!(
        "<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"UTF-8\">
<title>Run Report - {folder}</title>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ font-family: system-ui, sans-serif; background: #1a1a2e; color: #e0e0e0; padding: 24px; }}
.header {{ margin-bottom: 24px; padding-bottom: 16px; border-bottom: 1px solid #333; }}
.header h1 {{ font-size: 20px; color: #fff; margin-bottom: 8px; }}
.meta {{ font-size: 12px; color: #888; }}
.summary {{ display: flex; gap: 24px; margin-bottom: 24px; padding: 16px; background: #222; border-radius: 8px; }}
.stat {{ text-align: center; }}
.stat .value {{ font-size: 24px; font-weight: bold; }}
.stat .label {{ font-size: 11px; color: #888; text-transform: uppercase; }}
.passed {{ color: #4caf50; }}
.failed {{ color: #f44336; }}
.request {{ margin-bottom: 12px; border: 1px solid #333; border-radius: 6px; overflow: hidden; }}
.req-header {{ display: flex; align-items: center; gap: 8px; padding: 10px 12px; background: #252535; }}
.method {{ font-size: 11px; font-weight: bold; color: #64b5f6; font-family: monospace; }}
.name {{ font-size: 13px; flex: 1; }}
.status {{ font-weight: bold; font-family: monospace; }}
.time {{ font-size: 11px; color: #888; }}
.tests {{ padding: 8px 12px; }}
.test-item {{ display: flex; gap: 8px; padding: 4px 0; font-size: 12px; font-family: monospace; }}
</style>
</head>
<body>
<div class=\"header\">
<h1>Collection Run: {folder}</h1>
<div class=\"meta\">{timestamp} &bull; Duration: {duration}s</div>
</div>
<div class=\"summary\">
<div class=\"stat\"><div class=\"value\">{total}</div><div class=\"label\">Requests</div></div>
<div class=\"stat\"><div class=\"value passed\">{passed}</div><div class=\"label\">Passed</div></div>
<div class=\"stat\"><div class=\"value failed\">{failed}</div><div class=\"label\">Failed</div></div>
<div class=\"stat\"><div class=\"value\">{duration}s</div><div class=\"label\">Duration</div></div>
</div>
{results}
</body>
</html>",
        total = js_text(report.get("totalRequests")),
        passed = js_text(report.get("passedTests")),
        failed = js_text(report.get("failedTests")),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn iso_string_matches_to_iso_string() {
        assert_eq!(iso_string(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_string(1_700_000_123_456), "2023-11-14T22:15:23.456Z");
    }

    #[test]
    fn the_page_shows_requests_tests_and_escapes_text() {
        let html = render_runner_html(&json!({
            "folderId": "f", "folderName": "A & <B>", "startedAt": 1000, "completedAt": 3500,
            "totalRequests": 2, "passedTests": 1, "failedTests": 1,
            "results": [
                { "requestName": "one", "method": "GET", "status": 200, "responseTime": 12,
                  "tests": [{ "name": "ok", "passed": true }, { "name": "bad", "passed": false, "error": "x<y" }] },
                { "requestName": "two", "method": "POST", "status": null, "responseTime": 0, "error": "boom" },
            ],
        }));
        assert!(html.contains("<title>Run Report - A &amp; &lt;B&gt;</title>"));
        assert!(html.contains("Duration: 2.50s"));
        assert!(html.contains("<span class=\"status\">200</span>"));
        assert!(html.contains("<span class=\"time\">12ms</span>"));
        assert!(html.contains("<div class=\"test-item passed\">\u{2713} ok</div>"));
        assert!(html.contains("<div class=\"test-item failed\">\u{2717} bad \u{2014} x&lt;y</div>"));
        assert!(html.contains("<div class=\"test-item failed\">\u{2717} Error: boom</div>"));
        assert_eq!(html.matches("<span class=\"status\">").count(), 1);
    }

    #[test]
    fn missing_fields_print_as_undefined_like_the_template_does() {
        let html = render_runner_html(&json!({ "folderId": "f", "startedAt": 0 }));
        assert!(html.contains("<title>Run Report - undefined</title>"));
        assert!(html.contains("<div class=\"value\">undefined</div><div class=\"label\">Requests</div>"));
        assert!(html.contains("Duration: 0.00s"));
    }
}
