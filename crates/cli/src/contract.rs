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
    (segment.starts_with("{{") && segment.ends_with("}}")) || (segment.starts_with(':') && segment.len() > 1)
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
    /// What the mock was compared with: `real`, `snapshot` or `openapi`.
    source: &'static str,
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

/// A stored snapshot as one side of a comparison.
fn snapshot_side(snapshot: &Value) -> Side {
    let content_type = text(snapshot, "contentType");
    let body = snapshot.get("body").filter(|b| !b.is_null() && content_type == "json").cloned();
    let status = snapshot.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
    Side { status, content_type, headers: Vec::new(), body, raw: Vec::new() }
}

/// The stored snapshots by request id, read once.
fn snapshots(ctx: &mut Context, workspace: &str, cache: &mut Option<HashMap<String, Value>>) -> Result<()> {
    if cache.is_none() {
        let listed = ctx.request("contract.listSnapshots", json!({ "workspaceId": workspace }))?;
        let rows = listed["snapshots"].as_array().cloned().unwrap_or_default();
        *cache = Some(rows.into_iter().map(|row| (text(&row, "requestId"), row)).collect());
    }
    Ok(())
}

/// The operations of an OpenAPI file as pairing sees them.
struct Spec {
    /// status and the response skeleton, per `METHOD path` key of a [`Req`] built from the operation.
    sides: HashMap<String, Vec<Side>>,
    /// Shape paths of the properties the operation declares but does not require, by the same key and order.
    optional: HashMap<String, Vec<Vec<String>>>,
}

impl Spec {
    /// The operation's response for a mock answering `status`: the one with that status, else the first 2xx,
    /// and its optional paths.
    fn side(&self, key: &str, status: u16) -> (Side, &[String]) {
        let all = &self.sides[key];
        let at = all.iter().position(|s| s.status == status).unwrap_or(0);
        (all[at].clone(), &self.optional[key][at])
    }
}

/// `/orders/{id}` as a mock path: a whole `{id}` segment becomes the variable segment `{{id}}`.
fn template_path(path: &str) -> String {
    let segment = |s: &str| match s.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        Some(name) if !name.starts_with('{') => format!("{{{{{name}}}}}"),
        _ => s.to_string(),
    };
    path.split('/').map(segment).collect::<Vec<_>>().join("/")
}

/// Read `file` and have the engine turn its 2xx responses into skeletons. Each operation is reachable by its
/// path with the spec's base path (`servers[0].url`, `basePath`) and by the path alone.
fn load_spec(ctx: &mut Context, file: &str) -> Result<(Vec<Req>, Spec)> {
    let text = std::fs::read_to_string(file).map_err(|e| CliError::usage(format!("cannot read {file}: {e}")))?;
    let got = ctx.request("contract.fromOpenApi", json!({ "text": text }))?;
    let base = text_of(&got, "basePath");
    let (mut reqs, mut sides): (Vec<Req>, HashMap<String, Vec<Side>>) = (Vec::new(), HashMap::new());
    let mut optional: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    let paths_of = |op: &Value| -> Vec<String> {
        op["optional"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
    };
    for op in got["operations"].as_array().into_iter().flatten() {
        let (method, path) = (text_of(op, "method"), text_of(op, "path"));
        let raw = if op["body"].is_null() { Vec::new() } else { serde_json::to_vec(&op["body"]).unwrap_or_default() };
        let status = op["status"].as_u64().unwrap_or(200) as u16;
        let side = Side::new(status, &text_of(op, "contentType"), &[], raw);
        let mut prefixes = vec![base.as_str()];
        if !base.is_empty() {
            prefixes.push("");
        }
        for prefix in prefixes {
            let full = format!("{prefix}{path}");
            let key = format!("{method} {full}");
            if !sides.contains_key(&key) {
                reqs.push(Req {
                    id: key.clone(),
                    name: key.clone(),
                    method: method.clone(),
                    path: template_path(&full),
                });
            }
            optional.entry(key.clone()).or_default().push(paths_of(op));
            sides.entry(key).or_default().push(side.clone());
        }
    }
    Ok((reqs, Spec { sides, optional }))
}

fn text_of(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// `contract check [--mock <name>] [--folder <folder>] [--all] [--ignore <glob>]... [--from-capture]
/// [--from-snapshot]`.
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
    let (mocks, mut reqs) = selection(ctx, &workspace)?;
    let spec = match args.flag("openapi") {
        Some(file) => {
            let loaded = load_spec(ctx, file)?;
            reqs = loaded.0;
            Some(loaded.1)
        }
        None => None,
    };
    let from_snapshot = args.has("from-snapshot");
    let mut stored: Option<HashMap<String, Value>> = None;
    let ignore = args.flags("ignore");
    let mut real = real_of(ctx);
    let (mut checks, mut unpaired): (Vec<Check>, Vec<(&Value, String)>) = (Vec::new(), Vec::new());
    for mock in &mocks {
        let request = match pair_of(mock, &reqs) {
            Ok(request) => request,
            Err(reason) => {
                unpaired.push((mock, if spec.is_some() { "not in the spec".into() } else { reason }));
                continue;
            }
        };
        let mut allowed: &[String] = &[];
        let mut source = if from_snapshot { "snapshot" } else { "real" };
        let right = if let Some(spec) = &spec {
            source = "openapi";
            let (side, optional) = spec.side(&request.id, mock_side(mock).status);
            allowed = optional;
            Ok(side)
        } else if from_snapshot {
            snapshots(ctx, &workspace, &mut stored)?;
            match stored.iter().flatten().find(|(id, _)| **id == request.id) {
                Some((_, row)) => Ok(snapshot_side(row)),
                None => {
                    unpaired.push((mock, "no snapshot".into()));
                    continue;
                }
            }
        } else {
            match real.side(ctx, &workspace, request)? {
                Err(reason) if reason == "no real capture" => {
                    unpaired.push((mock, reason));
                    continue;
                }
                // The backend cannot be reached: the stored shape stands in for it.
                Err(reason) => {
                    snapshots(ctx, &workspace, &mut stored)?;
                    match stored.iter().flatten().find(|(id, _)| **id == request.id) {
                        Some((_, row)) => {
                            source = "snapshot";
                            Ok(snapshot_side(row))
                        }
                        None => Err(reason),
                    }
                }
                Ok(side) => Ok(side),
            }
        };
        let result = match right {
            Err(reason) => Outcome::Failed(reason),
            Ok(mut right) => {
                let mut left = mock_side(mock);
                (left.headers, right.headers) = (Vec::new(), Vec::new());
                let mut lines = shape::diff(&left, &right, false);
                lines.retain(|line| !ignore.iter().any(|glob| glob_matches(glob, &line.path)));
                // A mock may carry a property the spec declares without requiring it.
                lines.retain(|line| !(line.kind == '-' && allowed.contains(&line.path)));
                let (lines, more) = shape::capped(lines);
                Outcome::Drift(lines, more)
            }
        };
        checks.push(Check { mock, request, result, source });
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
                "source": c.source,
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
                let mark = if c.source != "real" && !mark.contains("failed") {
                    format!("{mark} ({})", c.source)
                } else {
                    mark
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

    #[test]
    fn a_failed_send_falls_back_to_the_snapshot_and_from_snapshot_sends_nothing() {
        let stored = json!({"snapshots": [{"requestId": "r1", "status": 200, "contentType": "json",
                                           "body": {"items": [{"id": 0, "extra": ""}]}}]});
        let mut down = engine(200, "{}");
        down.retain(|(name, _)| *name != "request.send");
        down.push(("request.send", Err(CliError::network("connection refused"))));
        down.push(("contract.listSnapshots", Ok(stored)));
        let (out, _) = run(&["contract", "check", "--mock", "Get orders"], down.clone(), check);
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        assert_eq!(
            text,
            "GET  /orders  Get orders  ✗ (snapshot)
  + .items[].extra (string)
1 drifted"
        );
        // Without a snapshot for the request the send failure stays a failure.
        let (out, _) = run(&["contract", "check", "--mock", "Create order"], down.clone(), check);
        assert_eq!(out.unwrap_err().code(), 4);
        let (out, calls) = run(&["contract", "check", "--from-snapshot"], down, check);
        assert_eq!(out.unwrap_err().code(), 1);
        assert!(payloads(&calls, "request.send").is_empty());
    }

    #[test]
    fn a_spec_pairs_by_method_and_path_with_or_without_the_base_path_and_sends_nothing() {
        let file = std::env::temp_dir().join(format!("bifurc-spec-{}.yaml", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let operations = json!({"basePath": "/v1", "operations": [
            {"method": "GET", "path": "/orders", "status": 200, "contentType": "application/json", "body": {"items": [{"id": 0, "discount": 0}]}},
            {"method": "GET", "path": "/orders/{id}", "status": 200, "contentType": "application/json", "body": {"id": 0}},
            {"method": "POST", "path": "/orders", "status": 201, "contentType": "", "body": null}]});
        let rows =
            json!([{"id": "m1", "name": "Get orders"}, {"id": "m3", "name": "Lonely"}, {"id": "m4", "name": "One"}]);
        let answers: Vec<Answer> = vec![
            ("config.get", Ok(config())),
            ("contract.fromOpenApi", Ok(operations)),
            ("entity.list:mocks", Ok(json!({"entities": rows}))),
            (
                "entity.load:mocks:m1",
                Ok(
                    json!({"ok": true, "entity": mock("m1", "Get orders", "GET", "http://h/v1/orders", 200, r#"{"items":[{"id":1}]}"#)}),
                ),
            ),
            (
                "entity.load:mocks:m3",
                Ok(json!({"ok": true, "entity": mock("m3", "Lonely", "GET", "http://h/none", 200, "{}")})),
            ),
            (
                "entity.load:mocks:m4",
                Ok(json!({"ok": true, "entity": mock("m4", "One", "GET", "http://h/orders/9", 200, r#"{"id":3}"#)})),
            ),
            ("entity.list", Ok(json!({"entities": []}))),
        ];
        let path = file.to_str().unwrap().to_string();
        let (out, calls) = run(&["contract", "check", "--openapi", &path], answers, check);
        std::fs::remove_file(&file).ok();
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        assert_eq!(
            text,
            "GET  /v1/orders  Get orders  ✗ (openapi)\n  + .items[].discount (number)\nGET  /orders/9   One         ✓ (openapi)\n1 drifted · 1 ok · 1 unpaired"
        );
        assert!(payloads(&calls, "request.send").is_empty());
        let (out, _) = run(&["contract", "check", "--openapi", "a.yaml", "--from-capture"], vec![], check);
        assert_eq!(out.unwrap_err().code(), 2);
    }

    #[test]
    fn a_mock_may_carry_an_optional_property_but_not_an_undeclared_one() {
        let file = std::env::temp_dir().join(format!("bifurc-spec-opt-{}.yaml", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let operations = json!({"basePath": "", "operations": [
            {"method": "GET", "path": "/a", "status": 200, "contentType": "application/json", "body": {"id": 0}, "optional": [".note"]}]});
        let rows = json!([{"id": "m1", "name": "Has note"}, {"id": "m2", "name": "Has extra"}]);
        let answers: Vec<Answer> = vec![
            ("config.get", Ok(config())),
            ("contract.fromOpenApi", Ok(operations)),
            ("entity.list:mocks", Ok(json!({"entities": rows}))),
            (
                "entity.load:mocks:m1",
                Ok(
                    json!({"ok": true, "entity": mock("m1", "Has note", "GET", "http://h/a", 200, r#"{"id":1,"note":"x"}"#)}),
                ),
            ),
            (
                "entity.load:mocks:m2",
                Ok(
                    json!({"ok": true, "entity": mock("m2", "Has extra", "GET", "http://h/a", 200, r#"{"id":1,"zzz":"x"}"#)}),
                ),
            ),
            ("entity.list", Ok(json!({"entities": []}))),
        ];
        let path = file.to_str().unwrap().to_string();
        let (out, _) = run(&["contract", "check", "--openapi", &path], answers, check);
        std::fs::remove_file(&file).ok();
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        assert_eq!(
            text,
            "GET  /a  Has note   ✓ (openapi)\nGET  /a  Has extra  ✗ (openapi)\n  - .zzz (string)\n1 drifted · 1 ok"
        );
    }
}
