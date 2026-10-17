//! `contract check`: does each mock still look like what the real endpoint answers? Every mock is paired with a
//! saved request of the same method and path, the request is sent (or the newest capture of that call is
//! taken), and status, content type and body shape are compared (never values, never headers).
//!
//! Mocks answer proxied traffic and `mocks serve`, never `request.send`, so a send here always reaches the real
//! server.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::{
    capture::{self, mock_side, response_side},
    commands::{Context, find_one, id_of, list_entities, workspace_id},
    error::{CliError, Result},
    format::aligned,
    requests::{self, TYPES, folder_paths, saved, text},
    response::Response,
    shape::{self, Line, Side},
};

/// How many captured entries `--from-capture` looks through, newest first.
const CAPTURE_WINDOW: u64 = 500;

/// A saved request as pairing sees it.
struct Req {
    id: String,
    name: String,
    method: String,
    path: String,
}

/// The path of a URL or URL pattern: no scheme and host (a leading `{{baseUrl}}` counts as one), no query or
/// fragment, no trailing slash. `{{var}}` segments stay.
pub(crate) fn path_of(url: &str) -> String {
    let mut rest = url.trim();
    rest = rest.find(['?', '#']).map_or(rest, |at| &rest[..at]);
    if rest.starts_with("{{") {
        rest = rest.find("}}").map_or(rest, |end| &rest[end + 2..]);
    }
    let host_first = !rest.starts_with('/') && rest.split('/').next().is_some_and(|h| h.contains(['.', ':']));
    if let Some((_, after)) = rest.split_once("://").or(rest.strip_prefix("://").map(|r| ("", r))) {
        rest = after;
        rest = rest.find('/').map_or("", |at| &rest[at..]);
    } else if host_first {
        rest = rest.find('/').map_or("", |at| &rest[at..]);
    }
    let rest = rest.trim_end_matches('/');
    if rest.starts_with('/') { rest.to_string() } else { format!("/{rest}") }
}

fn is_var(segment: &str) -> bool {
    segment.starts_with("{{") && segment.ends_with("}}")
}

/// Whether two paths are the same call: equal segment by segment, a `{{var}}` segment on either side matching
/// any one segment. Case-sensitive.
pub(crate) fn paths_match(a: &str, b: &str) -> bool {
    let (a, b): (Vec<&str>, Vec<&str>) = (a.split('/').collect(), b.split('/').collect());
    a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x == y || is_var(x) || is_var(y))
}

fn method_matches(mock: &str, request: &str) -> bool {
    mock == "*" || mock.eq_ignore_ascii_case(request)
}

/// The saved request a mock is paired with, or why there is none.
fn pair_of<'a>(mock: &Value, requests: &'a [Req]) -> std::result::Result<&'a Req, String> {
    if mock.get("useRegex").is_some_and(|v| v == &json!(true)) {
        // shortcut: a regex pattern is JavaScript's and the CLI has no regex engine, so it is never paired;
        // to upgrade, evaluate it with the path of each request once the CLI carries a regex dependency.
        return Err("regex pattern".into());
    }
    let (method, path) = (text(mock, "method"), path_of(&text(mock, "urlPattern")));
    let hits: Vec<&Req> =
        requests.iter().filter(|r| method_matches(&method, &r.method) && paths_match(&path, &r.path)).collect();
    match hits.as_slice() {
        [] => Err("no saved request".into()),
        [one] => Ok(one),
        many => {
            let exact: Vec<&&Req> = many.iter().filter(|r| r.path == path).collect();
            match exact.as_slice() {
                [one] => Ok(one),
                _ => Err(format!("ambiguous: {}", many.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "))),
            }
        }
    }
}

/// Whether `text` matches `glob`, where `*` stands for any run of characters.
pub(crate) fn glob_matches(glob: &str, text: &str) -> bool {
    let (g, t): (Vec<char>, Vec<char>) = (glob.chars().collect(), text.chars().collect());
    let (mut gi, mut ti, mut star) = (0, 0, None);
    while ti < t.len() {
        if gi < g.len() && g[gi] == '*' {
            star = Some((gi, ti));
            gi += 1;
        } else if gi < g.len() && g[gi] == t[ti] {
            gi += 1;
            ti += 1;
        } else if let Some((sg, st)) = star {
            gi = sg + 1;
            star = Some((sg, st + 1));
            ti = st + 1;
        } else {
            return false;
        }
    }
    g[gi..].iter().all(|c| *c == '*')
}

/// The summary line, and the exit code rule: any drift fails (1), else any failed real request (4), else 0.
fn finish(ctx: &Context, out: String, drifted: usize, failed: usize) -> Result<String> {
    let out = if ctx.args.has("quiet") { String::new() } else { out };
    match (drifted, failed) {
        (0, 0) => Ok(out),
        (0, _) => Err(CliError::Reported(4, out)),
        _ => Err(CliError::Failed(out)),
    }
}

/// The real side of every pair, fetched once per request: sent now, or read from the buffer.
struct Real {
    from_capture: bool,
    entries: Option<Vec<Value>>,
    cache: HashMap<String, std::result::Result<Side, String>>,
}

impl Real {
    fn side(&mut self, ctx: &mut Context, workspace: &str, request: &Req) -> Result<std::result::Result<Side, String>> {
        if let Some(done) = self.cache.get(&request.id) {
            return Ok(done.clone());
        }
        let done =
            if self.from_capture { self.captured(ctx, workspace, request)? } else { sent(ctx, workspace, request)? };
        self.cache.insert(request.id.clone(), done.clone());
        Ok(done)
    }

    fn captured(
        &mut self,
        ctx: &mut Context,
        workspace: &str,
        request: &Req,
    ) -> Result<std::result::Result<Side, String>> {
        if self.entries.is_none() {
            let all = capture::summaries(ctx, workspace, json!({ "last": CAPTURE_WINDOW }))?;
            self.entries = Some(all["entries"].as_array().cloned().unwrap_or_default());
        }
        let hit = self.entries.iter().flatten().find(|entry| {
            entry["via"] != "mock"
                && entry["status"].is_u64()
                && text(entry, "method").eq_ignore_ascii_case(&request.method)
                && paths_match(&request.path, &path_of(&text(entry, "url")))
        });
        let Some(id) = hit.map(|entry| text(entry, "id")) else { return Ok(Err("no real capture".into())) };
        let got = ctx.request("capture.get", json!({ "workspaceId": workspace, "id": id }))?;
        let entry = got.get("entry").filter(|entry| entry.is_object());
        Ok(entry.map(|entry| response_side(&Response::from_capture(entry))).ok_or_else(|| "no real capture".into()))
    }
}

/// Send the request. A request that failed on the wire is that pair's result; any other error stops the command.
fn sent(ctx: &mut Context, workspace: &str, request: &Req) -> Result<std::result::Result<Side, String>> {
    match requests::send_saved(ctx, workspace, &request.id) {
        Ok(response) if response.status == 0 => Ok(Err(response.failure.unwrap_or_else(|| "no response".into()))),
        Ok(response) => Ok(Ok(response_side(&response))),
        Err(err) if matches!(err.code(), 1 | 4) => Ok(Err(err.to_string())),
        Err(other) => Err(other),
    }
}

/// What a mock's check came to.
enum Outcome {
    Drift(Vec<Line>, usize),
    Failed(String),
}

struct Check<'a> {
    mock: &'a Value,
    request: &'a Req,
    result: Outcome,
}

fn one_line(error: &str) -> String {
    error.lines().next().unwrap_or_default().trim().trim_start_matches("ENGINE_ERROR: ").to_string()
}

/// `contract check [--mock <name>] [--folder <folder>] [--all] [--ignore <glob>]... [--from-capture]`.
pub fn check(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["mock", "folder", "all", "ignore", "from-capture", "var", "env", "fresh-session", "quiet"], 0)?;
    let args = ctx.args;
    let workspace = workspace_id(ctx)?;
    let mut mocks = crate::entities::all_mocks(ctx, &workspace)?;
    if let Some(wanted) = args.flag("mock") {
        let rows = list_entities(ctx, &workspace, "mocks")?;
        let id = id_of(find_one(&rows, wanted, &["name"], "mock", "mocks")?);
        mocks.retain(|mock| id_of(mock) == id);
    } else if !args.has("all") {
        mocks.retain(|mock| mock.get("enabled").is_some_and(|v| v == &json!(true)));
    }
    if let Some(folder) = args.flag("folder") {
        let folders = folder_paths(ctx, "mockFolders")?;
        let wanted = folder.trim_matches('/').to_lowercase();
        mocks.retain(|mock| {
            let path = folders.get(&text(mock, "folderId")).map(|p| p.to_lowercase()).unwrap_or_default();
            path.trim_end_matches('/') == wanted || path.starts_with(&format!("{wanted}/"))
        });
    }
    let reqs: Vec<Req> = saved(ctx, &workspace, &[&TYPES[0]])?
        .into_iter()
        .map(|(_, r)| Req {
            id: id_of(&r),
            name: text(&r, "name"),
            method: text(&r, "method"),
            path: path_of(&text(&r, "url")),
        })
        .collect();
    let ignore = args.flags("ignore");
    let mut real = Real { from_capture: args.has("from-capture"), entries: None, cache: HashMap::new() };
    let (mut checks, mut unpaired): (Vec<Check>, Vec<(&Value, String)>) = (Vec::new(), Vec::new());
    for mock in &mocks {
        let request = match pair_of(mock, &reqs) {
            Ok(request) => request,
            Err(reason) => {
                unpaired.push((mock, reason));
                continue;
            }
        };
        let result = match real.side(ctx, &workspace, request)? {
            Err(reason) if reason == "no real capture" => {
                unpaired.push((mock, reason));
                continue;
            }
            Err(reason) => Outcome::Failed(reason),
            Ok(mut right) => {
                let mut left = mock_side(mock);
                (left.headers, right.headers) = (Vec::new(), Vec::new());
                let mut lines = shape::diff(&left, &right, false);
                lines.retain(|line| !ignore.iter().any(|glob| glob_matches(glob, &line.path)));
                let (lines, more) = shape::capped(lines);
                Outcome::Drift(lines, more)
            }
        };
        checks.push(Check { mock, request, result });
    }
    let drifted =
        checks.iter().filter(|c| matches!(&c.result, Outcome::Drift(l, m) if !l.is_empty() || *m > 0)).count();
    let failed = checks.iter().filter(|c| matches!(c.result, Outcome::Failed(_))).count();
    let ok = checks.len() - drifted - failed;
    let mut out = json!({
        "schemaVersion": 1,
        "checks": checks.iter().map(|c| {
            let mut row = json!({
                "mock": text(c.mock, "name"), "mockId": id_of(c.mock), "method": text(c.mock, "method").to_uppercase(),
                "path": path_of(&text(c.mock, "urlPattern")), "request": c.request.name,
            });
            match &c.result {
                Outcome::Drift(lines, more) => {
                    row["ok"] = json!(lines.is_empty() && *more == 0);
                    row["differences"] = Value::Array(lines.iter().map(Line::to_json).collect());
                    row["truncated"] = json!(more);
                }
                Outcome::Failed(error) => {
                    row["ok"] = json!(false);
                    row["differences"] = json!([]);
                    row["error"] = json!(error);
                }
            }
            row
        }).collect::<Vec<_>>(),
        "unpaired": unpaired.iter().map(|(m, reason)| json!({"mock": text(m, "name"), "mockId": id_of(m), "reason": reason})).collect::<Vec<_>>(),
        "drifted": drifted, "ok": ok, "failed": failed,
    });
    out["unpairedCount"] = json!(unpaired.len());
    let human = || {
        let rows: Vec<Vec<String>> = checks
            .iter()
            .map(|c| {
                let mark = match &c.result {
                    Outcome::Failed(error) => format!("✗ real request failed: {}", one_line(error)),
                    Outcome::Drift(l, m) if !l.is_empty() || *m > 0 => "✗".to_string(),
                    Outcome::Drift(..) => "✓".to_string(),
                };
                vec![
                    text(c.mock, "method").to_uppercase(),
                    path_of(&text(c.mock, "urlPattern")),
                    text(c.mock, "name"),
                    mark,
                ]
            })
            .collect();
        let mut lines = Vec::new();
        for (row, c) in aligned(&rows).lines().zip(&checks) {
            lines.push(row.to_string());
            if let Outcome::Drift(diff, more) = &c.result {
                lines.extend(shape::render(diff, *more).lines().filter(|l| !l.is_empty()).map(|l| format!("  {l}")));
            }
        }
        if args.has("mock") {
            for (mock, reason) in &unpaired {
                lines.push(format!(
                    "{} {}  unpaired: {reason}",
                    text(mock, "method").to_uppercase(),
                    text(mock, "name")
                ));
            }
        }
        let mut parts = Vec::new();
        for (n, word) in [(drifted, "drifted"), (ok, "ok"), (failed, "failed")] {
            if n > 0 {
                parts.push(format!("{n} {word}"));
            }
        }
        if checks.is_empty() {
            parts.push("0 checked".into());
        }
        if !unpaired.is_empty() {
            parts.push(format!("{} unpaired", unpaired.len()));
        }
        lines.push(parts.join(" · "));
        lines.join("\n")
    };
    let printed = ctx.print(&out, human);
    finish(ctx, printed, drifted, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run};
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    fn req(name: &str, method: &str, url: &str) -> Req {
        Req { id: name.into(), name: name.into(), method: method.into(), path: path_of(url) }
    }

    #[test]
    fn paths_lose_host_query_and_trailing_slash() {
        assert_eq!(path_of("https://api.test:8080/orders/?x=1#f"), "/orders");
        assert_eq!(path_of("{{baseUrl}}/users/{{id}}"), "/users/{{id}}");
        assert_eq!(path_of("https://{{host}}/a/b"), "/a/b");
        assert_eq!(path_of("api.localhost/orders"), "/orders");
        assert_eq!(path_of("http://api.test"), "/");
        assert_eq!(path_of("/orders"), "/orders");
    }

    #[test]
    fn variable_segments_match_one_segment_and_case_counts() {
        assert!(paths_match("/users/{{id}}", "/users/7"));
        assert!(paths_match("/users/7", "/users/{{id}}"));
        assert!(!paths_match("/users/{{id}}", "/users/7/posts"));
        assert!(!paths_match("/Users", "/users"));
    }

    #[test]
    fn pairing_prefers_an_exact_path_and_names_why_not() {
        let reqs = vec![
            req("Get orders", "GET", "{{baseUrl}}/orders"),
            req("Get order", "GET", "{{baseUrl}}/orders/{{id}}"),
            req("Get order 7", "GET", "{{baseUrl}}/orders/7"),
            req("Get order x", "GET", "{{baseUrl}}/orders/{{x}}"),
            req("Make order", "POST", "{{baseUrl}}/orders/"),
        ];
        let mock = |method: &str, url: &str| json!({"method": method, "urlPattern": url});
        let paired = |m: Value| pair_of(&m, &reqs).map(|r| r.name.clone());
        assert_eq!(paired(mock("get", "http://h/orders?a=1")), Ok("Get orders".into()));
        assert_eq!(paired(mock("POST", "http://h/orders")), Ok("Make order".into()));
        assert_eq!(paired(mock("GET", "http://h/orders/7")), Ok("Get order 7".into()));
        assert_eq!(paired(mock("GET", "http://h/orders/9")), Err("ambiguous: Get order, Get order x".into()));
        assert_eq!(paired(mock("DELETE", "http://h/orders")), Err("no saved request".into()));
        let mut regex = mock("GET", "/orders/\\d+");
        regex["useRegex"] = json!(true);
        assert_eq!(paired(regex), Err("regex pattern".into()));
    }

    #[test]
    fn globs_let_a_star_stand_for_any_run() {
        assert!(glob_matches(".meta.*", ".meta.page.total"));
        assert!(glob_matches(".items[].debug", ".items[].debug"));
        assert!(glob_matches("*.debug", ".items[].debug"));
        assert!(!glob_matches(".meta.*", ".meta"));
        assert!(!glob_matches(".items[].debug", ".items[].debugx"));
        assert!(glob_matches("*", ""));
    }

    #[test]
    fn drift_beats_failure_beats_ok() {
        let argv = ["contract", "check"];
        let args = crate::args::parse_args(&argv.map(String::from)).unwrap();
        let ctx = Context::new(&args, Box::new(|| unreachable!()));
        assert!(finish(&ctx, "x".into(), 0, 0).is_ok());
        assert_eq!(finish(&ctx, "x".into(), 0, 2).unwrap_err().code(), 4);
        assert_eq!(finish(&ctx, "x".into(), 1, 2).unwrap_err().code(), 1);
    }

    fn mock(id: &str, name: &str, method: &str, url: &str, status: u16, body: &str) -> Value {
        json!({"id": id, "name": name, "method": method, "urlPattern": url, "enabled": true,
               "responseStatus": status, "responseBody": body,
               "responseHeaders": {"Content-Type": "application/json"}})
    }

    fn engine(real_status: u16, real_body: &str) -> Vec<Answer> {
        let rows = json!([{"id": "m1", "name": "Get orders"}, {"id": "m2", "name": "Create order"}, {"id": "m3", "name": "Lonely"}]);
        let reply = json!({"status": real_status, "durationMs": 3, "headers": {"content-type": "application/json"},
                           "body": STANDARD.encode(real_body)});
        vec![
            ("config.get", Ok(config())),
            ("entity.list:mocks", Ok(json!({"entities": rows}))),
            (
                "entity.load:mocks:m1",
                Ok(
                    json!({"ok": true, "entity": mock("m1", "Get orders", "GET", "http://h/orders", 200, r#"{"items":[{"id":1}]}"#)}),
                ),
            ),
            (
                "entity.load:mocks:m2",
                Ok(json!({"ok": true, "entity": mock("m2", "Create order", "POST", "http://h/orders", 201, "{}")})),
            ),
            (
                "entity.load:mocks:m3",
                Ok(json!({"ok": true, "entity": mock("m3", "Lonely", "GET", "http://h/none", 200, "{}")})),
            ),
            (
                "entity.list:requests",
                Ok(json!({"entities": [
                    {"id": "r1", "name": "Get orders", "method": "GET", "url": "{{baseUrl}}/orders"},
                    {"id": "r2", "name": "Make order", "method": "POST", "url": "{{baseUrl}}/orders"}]})),
            ),
            ("entity.list", Ok(json!({"entities": []}))),
            ("request.send", Ok(reply.clone())),
            ("request.send", Ok(reply)),
        ]
    }

    #[test]
    fn a_check_prints_the_drift_and_counts_the_rest() {
        let argv = ["contract", "check", "--ignore", ".items[].noise"];
        let body = r#"{"items":[{"id":1,"discount":2.5,"noise":1}]}"#;
        let (out, calls) = run(&argv, engine(200, body), check);
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        assert_eq!(
            text,
            "GET   /orders  Get orders    ✗\n  + .items[].discount (number)\nPOST  /orders  Create order  ✗\n  ~ status 201 → 200\n  + .items (array)\n2 drifted · 1 unpaired"
        );
        // Each request is sent once, with its own id.
        let sends = payloads(&calls, "request.send");
        let ids: Vec<&str> = sends.iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["r1", "r2"]);
    }

    #[test]
    fn json_lists_checks_and_unpaired_and_a_clean_check_exits_0() {
        let argv = ["contract", "check", "--json", "--mock", "get orders"];
        let (out, _) = run(&argv, engine(200, r#"{"items":[{"id":2}]}"#), check);
        let value: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(
            (value["schemaVersion"].clone(), value["ok"].clone(), value["drifted"].clone()),
            (json!(1), json!(1), json!(0))
        );
        assert_eq!(value["checks"][0]["request"], "Get orders");
        assert_eq!(value["checks"][0]["ok"], true);
    }
}
