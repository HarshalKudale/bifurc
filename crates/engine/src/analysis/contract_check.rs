//! `contract.check`: does each mock still look like what the real endpoint answers? Every mock is paired with a
//! saved request of the same method and path, the request is sent (or the newest capture of that call is taken,
//! or the stored snapshot, or an OpenAPI operation), and status, content type and body shape are compared (never
//! values, never headers). Mocks answer proxied traffic and `mocks serve`, never `request.send`, so a send here
//! always reaches the real server.

use std::collections::HashMap;

use bifurc_protocol::analysis::{
    pairing::{Req, glob_matches, pair_of, path_of},
    shape::{self, Line, Side},
};
use serde_json::{Value, json};

use super::{Sent, call, capture_entry, capture_side, entities, id_of, is_call_to, recent_captures, send_saved, text};
use crate::{
    kernel::CommandContext,
    protocol::{EngineError, Result},
    utils::json::{array_of, bool_of, opt_str, str_of},
};

/// A mock's answer as one side of a comparison.
fn mock_side(mock: &Value) -> Side {
    let headers: Vec<(String, String)> = mock
        .get("responseHeaders")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), super::js(Some(v))))
        .collect();
    let content_type = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map_or("", |(_, v)| v);
    let raw = if text(mock, "responseBodyEncoding") == "base64" {
        super::decode(mock.get("responseBody"))
    } else {
        super::js(mock.get("responseBody")).into_bytes()
    };
    let status = mock.get("responseStatus").and_then(Value::as_u64).unwrap_or(200) as u16;
    Side::new(status, content_type, &headers, raw)
}

/// What a snapshot stores of a response, as one side of a comparison.
fn snapshot_side(snapshot: &Value) -> Side {
    let content_type = text(snapshot, "contentType");
    let body = snapshot.get("body").filter(|b| !b.is_null() && content_type == "json").cloned();
    let status = snapshot.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
    Side { status, content_type, headers: Vec::new(), body, raw: Vec::new() }
}

/// The folder path (`a/b/`) of every folder in the mock tree.
async fn folder_paths(ctx: &CommandContext) -> Result<HashMap<String, String>> {
    let config = call(ctx, "config.get", json!({})).await?;
    let folders = config.get("mockFolders").and_then(Value::as_array).cloned().unwrap_or_default();
    let by_id: HashMap<String, &Value> = folders.iter().map(|f| (text(f, "id"), f)).collect();
    Ok(by_id
        .keys()
        .map(|id| {
            let (mut names, mut at) = (Vec::new(), Some(id.as_str()));
            // The depth cap stops a parent loop in a damaged tree.
            while let Some(folder) = at.and_then(|id| by_id.get(id)).filter(|_| names.len() < 32) {
                names.push(text(folder, "name"));
                at = folder.get("parentId").and_then(Value::as_str);
            }
            names.reverse();
            (id.clone(), format!("{}/", names.join("/")))
        })
        .collect())
}

/// The mocks the check works on (`mock`, `folder`, `all`: enabled ones by default).
async fn select_mocks(ctx: &CommandContext, params: &Value, workspace: &str) -> Result<Vec<Value>> {
    let mut mocks = Vec::new();
    for row in entities(ctx, workspace, "mocks").await? {
        let got =
            call(ctx, "entity.load", json!({ "workspaceId": workspace, "kind": "mocks", "id": id_of(&row) })).await?;
        if let Some(mock) = got.get("entity").filter(|e| e.is_object()) {
            mocks.push(mock.clone());
        }
    }
    if let Some(wanted) = opt_str(params, "mock") {
        let id = super::find_named(&mocks, wanted, "mock").map(id_of)?;
        mocks.retain(|mock| id_of(mock) == id);
    } else if !bool_of(params, "all", false) {
        mocks.retain(|mock| mock.get("enabled").is_some_and(|v| v == &json!(true)));
    }
    if let Some(folder) = opt_str(params, "folder") {
        let folders = folder_paths(ctx).await?;
        let wanted = folder.trim_matches('/').to_lowercase();
        mocks.retain(|mock| {
            let path = folders.get(&text(mock, "folderId")).map(|p| p.to_lowercase()).unwrap_or_default();
            path.trim_end_matches('/') == wanted || path.starts_with(&format!("{wanted}/"))
        });
    }
    Ok(mocks)
}

async fn saved_requests(ctx: &CommandContext, workspace: &str) -> Result<Vec<Req>> {
    Ok(entities(ctx, workspace, "requests")
        .await?
        .iter()
        .map(|r| Req { id: id_of(r), name: text(r, "name"), method: text(r, "method"), path: path_of(&text(r, "url")) })
        .collect())
}

/// `/orders/{id}` as a mock path: a whole `{id}` segment becomes the variable segment `{{id}}`.
fn template_path(path: &str) -> String {
    let segment = |s: &str| match s.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
        Some(name) if !name.starts_with('{') => format!("{{{{{name}}}}}"),
        _ => s.to_string(),
    };
    path.split('/').map(segment).collect::<Vec<_>>().join("/")
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

/// Have `contract.fromOpenApi` turn the 2xx responses of `file` into skeletons. Each operation is reachable by
/// its path with the spec's base path (`servers[0].url`, `basePath`) and by the path alone.
async fn load_spec(ctx: &CommandContext, file: &str) -> Result<(Vec<Req>, Spec)> {
    let got = call(ctx, "contract.fromOpenApi", json!({ "text": file })).await?;
    let base = text(&got, "basePath");
    let (mut reqs, mut sides): (Vec<Req>, HashMap<String, Vec<Side>>) = (Vec::new(), HashMap::new());
    let mut optional: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for op in array_of(&got, "operations") {
        let (method, path) = (text(op, "method"), text(op, "path"));
        let raw = if op["body"].is_null() { Vec::new() } else { serde_json::to_vec(&op["body"]).unwrap_or_default() };
        let status = op["status"].as_u64().unwrap_or(200) as u16;
        let side = Side::new(status, &text(op, "contentType"), &[], raw);
        let paths: Vec<String> =
            array_of(op, "optional").iter().filter_map(Value::as_str).map(str::to_string).collect();
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
            optional.entry(key.clone()).or_default().push(paths.clone());
            sides.entry(key).or_default().push(side.clone());
        }
    }
    Ok((reqs, Spec { sides, optional }))
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

const NO_CAPTURE: &str = "no real capture";

/// The real side of every pair, fetched once per request: sent now, or read from the buffer.
struct Real {
    from_capture: bool,
    entries: Option<Vec<Value>>,
    cache: HashMap<String, Sent>,
}

impl Real {
    async fn side(&mut self, ctx: &CommandContext, params: &Value, request: &Req) -> Result<Sent> {
        if let Some(done) = self.cache.get(&request.id) {
            return Ok(done.clone());
        }
        let workspace = str_of(params, "workspaceId");
        let done = if self.from_capture {
            if self.entries.is_none() {
                self.entries = Some(recent_captures(ctx, workspace).await?);
            }
            let hit = self.entries.iter().flatten().find(|e| is_call_to(e, &request.method, &request.path));
            match hit {
                None => Err(NO_CAPTURE.to_string()),
                Some(hit) => match capture_entry(ctx, workspace, &text(hit, "id")).await? {
                    Some(entry) => Ok(capture_side(&entry)),
                    None => Err(NO_CAPTURE.to_string()),
                },
            }
        } else {
            send_saved(ctx, params, &request.id).await?
        };
        self.cache.insert(request.id.clone(), done.clone());
        Ok(done)
    }
}

/// The stored snapshots by request id.
async fn snapshots(ctx: &CommandContext, workspace: &str) -> Result<HashMap<String, Value>> {
    let listed = call(ctx, "contract.listSnapshots", json!({ "workspaceId": workspace })).await?;
    Ok(array_of(&listed, "snapshots").iter().map(|row| (text(row, "requestId"), row.clone())).collect())
}

pub(super) async fn check(params: Value, ctx: CommandContext) -> Result<Value> {
    let workspace = str_of(&params, "workspaceId").to_string();
    let (openapi, from_snapshot, from_capture) =
        (opt_str(&params, "openapi"), bool_of(&params, "fromSnapshot", false), bool_of(&params, "fromCapture", false));
    if openapi.is_some() && (from_snapshot || from_capture) {
        return Err(EngineError::bad_request("openapi cannot be combined with fromCapture or fromSnapshot"));
    }
    let mocks = select_mocks(&ctx, &params, &workspace).await?;
    let (reqs, spec) = match openapi {
        Some(text) => {
            let (reqs, spec) = load_spec(&ctx, text).await?;
            (reqs, Some(spec))
        }
        None => (saved_requests(&ctx, &workspace).await?, None),
    };
    let ignore: Vec<&str> = array_of(&params, "ignore").iter().filter_map(Value::as_str).collect();
    let mut stored: Option<HashMap<String, Value>> = None;
    let mut real = Real { from_capture, entries: None, cache: HashMap::new() };
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
        let right: Sent = if let Some(spec) = &spec {
            source = "openapi";
            let (side, optional) = spec.side(&request.id, mock_side(mock).status);
            allowed = optional;
            Ok(side)
        } else if from_snapshot {
            if stored.is_none() {
                stored = Some(snapshots(&ctx, &workspace).await?);
            }
            match stored.iter().flatten().find(|(id, _)| **id == request.id) {
                Some((_, row)) => Ok(snapshot_side(row)),
                None => {
                    unpaired.push((mock, "no snapshot".into()));
                    continue;
                }
            }
        } else {
            match real.side(&ctx, &params, request).await? {
                Err(reason) if reason == NO_CAPTURE => {
                    unpaired.push((mock, reason));
                    continue;
                }
                // The backend cannot be reached: the stored shape stands in for it.
                Err(reason) => {
                    if stored.is_none() {
                        stored = Some(snapshots(&ctx, &workspace).await?);
                    }
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
    let rows: Vec<Value> = checks
        .iter()
        .map(|c| {
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
        })
        .collect();
    // `method` is for people reading the list of unpaired mocks; the CLI drops it from its JSON.
    let unpaired_rows: Vec<Value> = unpaired
        .iter()
        .map(|(m, reason)| {
            json!({
                "mock": text(m, "name"), "mockId": id_of(m), "reason": reason, "method": text(m, "method").to_uppercase(),
            })
        })
        .collect();
    Ok(json!({
        "schemaVersion": 1, "checks": rows, "unpaired": unpaired_rows,
        "drifted": drifted, "ok": ok, "failed": failed, "unpairedCount": unpaired.len(),
    }))
}
