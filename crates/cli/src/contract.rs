//! `contract check`: does each mock still look like what the real endpoint answers? Every mock is paired with a
//! saved request of the same method and path, the request is sent (or the newest capture of that call is
//! taken), and status, content type and body shape are compared (never values, never headers).
//!
//! `contract update-mock` rewrites a mock's body to the real shape; `contract snapshot` stores the shape of what
//! each paired request answers (never a value) in the workspace, and `check` falls back to it when the backend
//! cannot be reached.
//!
//! Mocks answer proxied traffic and `mocks serve`, never `request.send`, so a send here always reaches the real
//! server.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::{
    capture::{self, mock_side, response_side},
    commands::{Context, find_one, id_of, list_entities, refused, workspace_id},
    error::{CliError, Result},
    format::aligned,
    requests::{self, TYPES, folder_paths, saved, text},
    response::Response,
    shape::{self, Line, Side},
};

pub(crate) use bifurc_protocol::analysis::pairing::{Req, method_matches, pair_of, path_of, paths_match};

/// How many captured entries `--from-capture` looks through, newest first.
const CAPTURE_WINDOW: u64 = 500;

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

fn one_line(error: &str) -> String {
    error.lines().next().unwrap_or_default().trim().trim_start_matches("ENGINE_ERROR: ").to_string()
}

/// The mocks a command works on (`--mock`, `--folder`, `--all`: enabled ones by default) and the saved requests
/// they can be paired with.
fn selection(ctx: &mut Context, workspace: &str) -> Result<(Vec<Value>, Vec<Req>)> {
    let args = ctx.args;
    let mut mocks = crate::entities::all_mocks(ctx, workspace)?;
    if let Some(wanted) = args.flag("mock") {
        let rows = list_entities(ctx, workspace, "mocks")?;
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
    // --openapi pairs with the spec's operations, not with saved requests.
    Ok((mocks, if args.has("openapi") { Vec::new() } else { saved_requests(ctx, workspace)? }))
}

fn saved_requests(ctx: &mut Context, workspace: &str) -> Result<Vec<Req>> {
    Ok(saved(ctx, workspace, &[&TYPES[0]])?
        .into_iter()
        .map(|(_, r)| Req {
            id: id_of(&r),
            name: text(&r, "name"),
            method: text(&r, "method"),
            path: path_of(&text(&r, "url")),
        })
        .collect())
}

/// What a snapshot stores of a response: status, short content type and the skeleton of a JSON body.
fn snapshot_of(request: &Req, side: &Side) -> Value {
    json!({
        "requestId": request.id, "method": request.method.to_uppercase(), "path": request.path,
        "status": side.status, "contentType": side.content_type,
        "body": side.body.as_ref().map_or(Value::Null, shape::skeleton),
    })
}

/// `contract check [--mock <name>] [--folder <folder>] [--all] [--ignore <glob>]... [--from-capture]
/// [--from-snapshot] [--openapi <file>]`: the engine's `contract.check` does the work; this prints its report.
pub fn check(ctx: &mut Context) -> Result<String> {
    ctx.accept(
        &[
            "mock",
            "folder",
            "all",
            "ignore",
            "from-capture",
            "from-snapshot",
            "openapi",
            "var",
            "env",
            "fresh-session",
            "quiet",
        ],
        0,
    )?;
    let args = ctx.args;
    if args.has("openapi") && (args.has("from-capture") || args.has("from-snapshot")) {
        return Err(CliError::usage("--openapi cannot be combined with --from-capture or --from-snapshot"));
    }
    let workspace = workspace_id(ctx)?;
    let mut params = json!({ "workspaceId": workspace });
    if let Some(wanted) = args.flag("mock") {
        let rows = list_entities(ctx, &workspace, "mocks")?;
        params["mock"] = json!(id_of(find_one(&rows, wanted, &["name"], "mock", "mocks")?));
    }
    if let Some(folder) = args.flag("folder") {
        params["folder"] = json!(folder);
    }
    if let Some(file) = args.flag("openapi") {
        let spec = std::fs::read_to_string(file).map_err(|e| CliError::usage(format!("cannot read {file}: {e}")))?;
        params["openapi"] = json!(spec);
    }
    for (flag, key) in [("all", "all"), ("from-capture", "fromCapture"), ("from-snapshot", "fromSnapshot")] {
        if args.has(flag) {
            params[key] = json!(true);
        }
    }
    if args.has("fresh-session") {
        params["freshJar"] = json!(true);
    }
    let ignore = args.flags("ignore");
    if !ignore.is_empty() {
        params["ignore"] = json!(ignore);
    }
    requests::add_scope(ctx, &workspace, &mut params)?;
    let full = ctx.request("contract.check", params)?;
    let count = |key: &str| full[key].as_u64().unwrap_or(0) as usize;
    let (drifted, ok, failed) = (count("drifted"), count("ok"), count("failed"));
    let human = || {
        let checks = full["checks"].as_array().map(Vec::as_slice).unwrap_or_default();
        let rows: Vec<Vec<String>> = checks
            .iter()
            .map(|c| {
                let mark = match c["error"].as_str() {
                    Some(error) => format!("✗ real request failed: {}", one_line(error)),
                    None if c["ok"] == true => "✓".to_string(),
                    None => "✗".to_string(),
                };
                let source = text(c, "source");
                let mark =
                    if source != "real" && !mark.contains("failed") { format!("{mark} ({source})") } else { mark };
                vec![text(c, "method"), text(c, "path"), text(c, "mock"), mark]
            })
            .collect();
        let mut lines = Vec::new();
        for (row, c) in aligned(&rows).lines().zip(checks) {
            lines.push(row.to_string());
            let mut diff: Vec<String> =
                c["differences"].as_array().into_iter().flatten().map(|d| text(d, "text")).collect();
            if let Some(more) = c["truncated"].as_u64().filter(|m| *m > 0) {
                diff.push(format!("… {more} more differences"));
            }
            lines.extend(diff.into_iter().map(|l| format!("  {l}")));
        }
        let unpaired = full["unpaired"].as_array().map(Vec::as_slice).unwrap_or_default();
        if args.has("mock") {
            for row in unpaired {
                lines.push(format!("{} {}  unpaired: {}", text(row, "method"), text(row, "mock"), text(row, "reason")));
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
    // The `method` of an unpaired row is for the lines above, not part of the JSON.
    let mut out = full.clone();
    for row in out["unpaired"].as_array_mut().into_iter().flatten() {
        row.as_object_mut().map(|fields| fields.shift_remove("method"));
    }
    let printed = ctx.print(&out, human);
    finish(ctx, printed, drifted, failed)
}

/// A name and why (or what went wrong).
type Pair = (String, String);

fn real_of(ctx: &Context) -> Real {
    Real { from_capture: ctx.args.has("from-capture"), entries: None, cache: HashMap::new() }
}

/// `contract update-mock <mock> [--from-capture]`: rewrite the mock's body to the shape the real answer has,
/// keeping the mock's values where a key stays, and its status when it differs. Headers and the rest stay.
pub fn update_mock(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["from-capture", "var", "env", "fresh-session", "quiet", "force"], 1)?;
    let args = ctx.args;
    let wanted = crate::commands::need(args.positionals.get(2).map(String::as_str), "a mock name")?;
    let workspace = workspace_id(ctx)?;
    let mocks = crate::entities::all_mocks(ctx, &workspace)?;
    let mock = find_one(&mocks, &wanted, &["name"], "mock", "mocks")?;
    let name = text(mock, "name");
    let reqs = saved_requests(ctx, &workspace)?;
    let request = pair_of(mock, &reqs)
        .map_err(|why| CliError::not_found(format!("\"{name}\" has no saved request to compare with ({why})")))?;
    let mut right = match real_of(ctx).side(ctx, &workspace, request)? {
        Ok(side) => side,
        Err(reason) if reason == "no real capture" => {
            return Err(CliError::not_found(format!("\"{name}\" has no captured answer to compare with ({reason})")));
        }
        Err(reason) => return Err(CliError::network(format!("the real request failed: {}", one_line(&reason)))),
    };
    let mut left = mock_side(mock);
    (left.headers, right.headers) = (Vec::new(), Vec::new());
    let (lines, more) = shape::capped(shape::diff(&left, &right, false));
    let updated = !lines.is_empty() || more > 0;
    if updated {
        let mut entity = mock.clone();
        let before: Option<Value> = serde_json::from_slice(&left.raw).ok();
        let body = match (&right.body, &before) {
            (Some(real), Some(have)) => Some(shape::merge(have, real)),
            (Some(real), None) => Some(real.clone()),
            _ => None,
        };
        if let Some(body) = body.filter(|body| before.as_ref() != Some(body)) {
            entity["responseBody"] = json!(serde_json::to_string_pretty(&body).unwrap_or_default());
        } else if right.body.is_none() && before.is_some() {
            entity["responseBody"] = json!(String::from_utf8_lossy(&right.raw));
        }
        if entity["responseBody"] != mock["responseBody"] {
            entity.as_object_mut().map(|fields| fields.remove("responseBodyEncoding"));
        }
        if left.status != right.status {
            entity["responseStatus"] = json!(right.status);
        }
        if entity.get("workspaceId").is_none_or(Value::is_null) {
            entity["workspaceId"] = json!(workspace);
        }
        refused(ctx.request("entity.update", json!({ "kind": "mocks", "entity": entity }))?)?;
    }
    let out = json!({
        "schemaVersion": 1, "mock": name, "updated": updated,
        "differences": lines.iter().map(Line::to_json).collect::<Vec<_>>(),
    });
    let printed = ctx.print(&out, || {
        let verdict =
            if updated { format!("updated mock \"{name}\"") } else { format!("mock \"{name}\" already matches") };
        if updated {
            format!(
                "{}
{verdict}",
                shape::render(&lines, more)
            )
        } else {
            verdict
        }
    });
    Ok(if args.has("quiet") { String::new() } else { printed })
}

/// `contract snapshot [--mock <name>] [--folder <folder>] [--all] [--from-capture]`: store the shape (never a
/// value) of what each paired request answers, one send per request, in the workspace's `contracts/` folder.
pub fn snapshot(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["mock", "folder", "all", "from-capture", "var", "env", "fresh-session", "quiet"], 0)?;
    let workspace = workspace_id(ctx)?;
    let (mocks, reqs) = selection(ctx, &workspace)?;
    let mut real = real_of(ctx);
    let (mut saved, mut failed, mut unpaired): (Vec<Value>, Vec<Pair>, Vec<Pair>) =
        (Vec::new(), Vec::new(), Vec::new());
    let mut seen: Vec<&str> = Vec::new();
    for mock in &mocks {
        let request = match pair_of(mock, &reqs) {
            Ok(request) => request,
            Err(reason) => {
                unpaired.push((text(mock, "name"), reason));
                continue;
            }
        };
        if seen.contains(&request.id.as_str()) {
            continue;
        }
        seen.push(&request.id);
        match real.side(ctx, &workspace, request)? {
            Ok(side) => saved.push(snapshot_of(request, &side)),
            Err(reason) if reason == "no real capture" => unpaired.push((text(mock, "name"), reason)),
            Err(reason) => failed.push((request.name.clone(), one_line(&reason))),
        }
    }
    if !saved.is_empty() {
        ctx.request("contract.saveSnapshots", json!({ "workspaceId": workspace, "snapshots": saved }))?;
    }
    let pair = |rows: &[Pair], key: &str| -> Vec<Value> {
        rows.iter()
            .map(|(name, why)| json!({ key: name, if key == "mock" { "reason" } else { "error" }: why }))
            .collect()
    };
    let out = json!({
        "schemaVersion": 1, "saved": saved, "failed": pair(&failed, "request"), "unpaired": pair(&unpaired, "mock"),
    });
    let printed = ctx.print(&out, || {
        let mut head = format!("saved {} snapshot{}", saved.len(), if saved.len() == 1 { "" } else { "s" });
        for (n, word) in [(failed.len(), "failed"), (unpaired.len(), "unpaired")] {
            if n > 0 {
                head.push_str(&format!(" · {n} {word}"));
            }
        }
        let mut lines = vec![head];
        lines.extend(failed.iter().map(|(name, why)| format!("✗ {name}: {why}")));
        lines.join(
            "
",
        )
    });
    finish(ctx, printed, 0, failed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run};
    use base64::{Engine as _, engine::general_purpose::STANDARD};

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

    /// What `contract.check` reports for two drifted mocks and one lonely mock.
    fn report() -> Value {
        json!({"schemaVersion": 1, "checks": [
            {"mock": "Get orders", "mockId": "m1", "method": "GET", "path": "/orders", "request": "Get orders",
             "source": "real", "ok": false, "truncated": 0,
             "differences": [{"kind": "+", "path": ".items[].discount", "text": "+ .items[].discount (number)"}]},
            {"mock": "Create order", "mockId": "m2", "method": "POST", "path": "/orders", "request": "Make order",
             "source": "real", "ok": false, "truncated": 3,
             "differences": [{"kind": "~", "path": "status", "text": "~ status 201 → 200"},
                             {"kind": "+", "path": ".items", "text": "+ .items (array)"}]}],
            "unpaired": [{"mock": "Lonely", "mockId": "m3", "reason": "no saved request", "method": "GET"}],
            "drifted": 2, "ok": 0, "failed": 0, "unpairedCount": 1})
    }

    #[test]
    fn a_check_prints_the_engines_report_and_passes_the_flags_on() {
        let argv = ["contract", "check", "--ignore", ".items[].noise", "--from-capture", "--var", "a=b", "--all"];
        let (out, calls) = run(&argv, vec![("config.get", Ok(config())), ("contract.check", Ok(report()))], check);
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        assert_eq!(
            text,
            "GET   /orders  Get orders    ✗\n  + .items[].discount (number)\nPOST  /orders  Create order  ✗\n  ~ status 201 → 200\n  + .items (array)\n  … 3 more differences\n2 drifted · 1 unpaired"
        );
        let sent = &payloads(&calls, "contract.check")[0];
        assert_eq!(
            (sent["ignore"].clone(), sent["fromCapture"].clone(), sent["all"].clone(), sent["variables"]["a"].clone()),
            (json!([".items[].noise"]), json!(true), json!(true), json!("b"))
        );
        assert!(sent.get("mock").is_none() && sent.get("openapi").is_none());
    }

    #[test]
    fn json_is_the_report_without_the_method_of_unpaired_rows_and_a_named_mock_lists_its_unpaired() {
        let rows = json!({"entities": [{"id": "m1", "name": "Get orders"}]});
        let answers =
            vec![("config.get", Ok(config())), ("entity.list:mocks", Ok(rows)), ("contract.check", Ok(report()))];
        let (out, calls) = run(&["contract", "check", "--json", "--mock", "get orders"], answers.clone(), check);
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!((value["schemaVersion"].clone(), value["drifted"].clone()), (json!(1), json!(2)));
        assert_eq!(value["unpaired"][0], json!({"mock": "Lonely", "mockId": "m3", "reason": "no saved request"}));
        assert_eq!(payloads(&calls, "contract.check")[0]["mock"], "m1");
        let (out, _) = run(&["contract", "check", "--mock", "get orders"], answers, check);
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        assert!(text.contains("\nGET Lonely  unpaired: no saved request\n"), "{text}");
    }

    #[test]
    fn failures_and_other_sources_are_marked_and_only_failures_exit_4() {
        let failed = json!({"schemaVersion": 1, "drifted": 0, "ok": 1, "failed": 1, "unpairedCount": 0, "unpaired": [], "checks": [
            {"mock": "A", "method": "GET", "path": "/a", "source": "real", "ok": false, "differences": [],
             "error": "UPSTREAM_FAILED: connection refused\nmore"},
            {"mock": "B", "method": "GET", "path": "/b", "source": "snapshot", "ok": true, "differences": [], "truncated": 0}]});
        let answers = vec![("config.get", Ok(config())), ("contract.check", Ok(failed))];
        let (out, _) = run(&["contract", "check"], answers, check);
        let Err(CliError::Reported(code, text)) = out else { panic!("{out:?}") };
        assert_eq!(code, 4);
        assert_eq!(
            text,
            "GET  /a  A  ✗ real request failed: UPSTREAM_FAILED: connection refused\nGET  /b  B  ✓ (snapshot)\n1 ok · 1 failed"
        );
    }

    #[test]
    fn the_spec_file_is_read_and_sent_as_text_and_cannot_be_combined_with_a_capture() {
        let file = std::env::temp_dir().join(format!("bifurc-spec-{}.yaml", std::process::id()));
        std::fs::write(&file, "openapi: 3.0.0").unwrap();
        let clean = json!({"schemaVersion": 1, "checks": [], "unpaired": [], "drifted": 0, "ok": 0, "failed": 0, "unpairedCount": 0});
        let answers = vec![("config.get", Ok(config())), ("contract.check", Ok(clean))];
        let path = file.to_str().unwrap().to_string();
        let (out, calls) = run(&["contract", "check", "--openapi", &path], answers, check);
        std::fs::remove_file(&file).ok();
        assert_eq!(out.unwrap(), "0 checked");
        assert_eq!(payloads(&calls, "contract.check")[0]["openapi"], "openapi: 3.0.0");
        let (out, _) = run(&["contract", "check", "--openapi", "a.yaml", "--from-capture"], vec![], check);
        assert_eq!(out.unwrap_err().code(), 2);
    }

    fn answers_for(real_body: &str, extra: Vec<Answer>) -> Vec<Answer> {
        let mut answers = engine(200, real_body);
        answers.extend(extra);
        answers
    }

    #[test]
    fn update_mock_rewrites_the_shape_keeps_values_and_saves_once() {
        let argv = ["contract", "update-mock", "get ORD"];
        let body = r#"{"items":[{"id":7,"discount":2.5}],"total":"9"}"#;
        let answers = answers_for(body, vec![("entity.update", Ok(json!({"ok": true})))]);
        let (out, calls) = run(&argv, answers, update_mock);
        assert_eq!(
            out.unwrap(),
            "+ .items[].discount (number)
+ .total (string)
updated mock \"Get orders\""
        );
        let sent = payloads(&calls, "entity.update");
        assert_eq!(sent.len(), 1);
        let entity = &sent[0]["entity"];
        let stored: Value = serde_json::from_str(entity["responseBody"].as_str().unwrap()).unwrap();
        assert_eq!(stored, json!({"items": [{"id": 1, "discount": 2.5}], "total": "9"}));
        assert!(entity["responseBody"].as_str().unwrap().contains("\n"));
        assert_eq!((entity["responseStatus"].clone(), entity["name"].clone()), (json!(200), json!("Get orders")));
    }

    #[test]
    fn update_mock_sets_the_status_and_leaves_a_matching_mock_alone() {
        let answers = answers_for(r#"{"items":[{"id":3}]}"#, vec![("entity.update", Ok(json!({"ok": true})))]);
        let (out, calls) = run(&["contract", "update-mock", "Create order", "--json"], answers.clone(), update_mock);
        let value: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!((value["updated"].clone(), value["mock"].clone()), (json!(true), json!("Create order")));
        assert_eq!(payloads(&calls, "entity.update")[0]["entity"]["responseStatus"], 200);
        let (out, calls) = run(&["contract", "update-mock", "Get orders"], answers, update_mock);
        assert_eq!(out.unwrap(), "mock \"Get orders\" already matches");
        assert!(payloads(&calls, "entity.update").is_empty());
    }

    #[test]
    fn update_mock_refuses_an_unpaired_mock_and_a_failed_send() {
        let (out, _) = run(&["contract", "update-mock", "Lonely"], engine(200, "{}"), update_mock);
        let err = out.unwrap_err();
        assert_eq!(err.code(), 3);
        assert_eq!(err.to_string(), "\"Lonely\" has no saved request to compare with (no saved request)");
        let mut down = engine(200, "{}");
        down.retain(|(name, _)| *name != "request.send");
        down.push(("request.send", Err(CliError::network("connection refused"))));
        let (out, calls) = run(&["contract", "update-mock", "Get orders"], down, update_mock);
        assert_eq!(out.unwrap_err().code(), 4);
        assert!(payloads(&calls, "entity.update").is_empty());
    }

    #[test]
    fn a_snapshot_stores_the_shape_without_values_once_per_request() {
        let body = r#"{"items":[{"id":1,"token":"abc"}],"name":"Ann"}"#;
        let answers = answers_for(body, vec![("contract.saveSnapshots", Ok(json!({"saved": 2})))]);
        let (out, calls) = run(&["contract", "snapshot", "--all"], answers, snapshot);
        assert_eq!(out.unwrap(), "saved 2 snapshots · 1 unpaired");
        assert_eq!(payloads(&calls, "request.send").len(), 2);
        let sent = &payloads(&calls, "contract.saveSnapshots")[0];
        assert!(!sent.to_string().contains("abc") && !sent.to_string().contains("Ann"));
        let first = &sent["snapshots"][0];
        assert_eq!(first["requestId"], "r1");
        assert_eq!(first["body"], json!({"items": [{"id": 0, "token": ""}], "name": ""}));
    }
}
