//! `contract.check` and `types.generate`: what the CLI's `contract check` and `types` do, so the desktop app can
//! too. Both read the workspace and send saved requests through the registry (`request.send`), so the policy
//! guard and the scopes apply as for any other caller. The pure parts (shapes, inference, pairing) live in
//! `bifurc_protocol::analysis`.

mod contract_check;
#[cfg(test)]
mod tests;
mod types_generate;

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use bifurc_protocol::analysis::{pairing::path_of, shape::Side};
use serde_json::{Value, json};

use crate::{
    kernel::{CommandContext, CommandRegistry},
    protocol::{EngineError, ErrorCode, Result},
};

/// How many captured entries are looked through, newest first, and how many matching ones are read.
const CAPTURE_WINDOW: u64 = 500;
const MAX_CAPTURES: usize = 20;

pub(crate) async fn call(ctx: &CommandContext, command: &str, params: Value) -> Result<Value> {
    ctx.registry.invoke(command, params, ctx.clone()).await
}

pub(crate) fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn js(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => String::new(),
        Some(other) => other.to_string(),
    }
}

/// The bytes of a base64 string, with or without padding; none when it is not one.
fn decode(value: Option<&Value>) -> Vec<u8> {
    STANDARD_NO_PAD.decode(js(value).trim_end_matches('=')).unwrap_or_default()
}

fn pairs(object: Option<&Value>) -> Vec<(String, String)> {
    object.and_then(Value::as_object).into_iter().flatten().map(|(k, v)| (k.clone(), js(Some(v)))).collect()
}

/// A response of the engine (`request.send` or a captured exchange) as one side of a comparison.
fn side_of(status: Option<&Value>, headers: Option<&Value>, body: Vec<u8>) -> Side {
    let status = status.and_then(Value::as_u64).unwrap_or(0) as u16;
    let headers = pairs(headers);
    let content_type =
        headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("content-type")).map_or("", |(_, v)| v.as_str());
    Side::new(status, content_type, &headers, body)
}

/// A captured exchange (`capture.get`): the response the proxy saw.
pub(crate) fn capture_side(entry: &Value) -> Side {
    let status = entry.get("resStatus").filter(|v| !v.is_null()).or(entry.get("status"));
    side_of(status, entry.get("resHeaders"), decode(entry.get("resBody")))
}

/// What a saved request answered when sent now, or why it did not (the wire failed, a script failed).
pub(crate) type Sent = std::result::Result<Side, String>;

/// Send the saved rest request `id`. A send that failed on the wire is that request's result; a refusal by
/// policy, or a request that does not exist, stops the command.
pub(crate) async fn send_saved(ctx: &CommandContext, params: &Value, id: &str) -> Result<Sent> {
    let mut payload = json!({ "workspaceId": text(params, "workspaceId"), "id": id });
    for key in ["variables", "environmentId", "freshJar"] {
        if let Some(value) = params.get(key).filter(|v| !v.is_null()) {
            payload[key] = value.clone();
        }
    }
    match call(ctx, "request.send", payload).await {
        Ok(reply) => {
            let side = side_of(reply.get("status"), reply.get("headers"), decode(reply.get("body")));
            if side.status != 0 {
                return Ok(Ok(side));
            }
            let failure = [("preScriptError", "pre-script"), ("postScriptError", "post-script")].iter().find_map(
                |(key, label)| {
                    let message = reply.get(*key).and_then(Value::as_str).filter(|m| !m.is_empty())?;
                    Some(format!("{label} error: {message}"))
                },
            );
            Ok(Err(failure.unwrap_or_else(|| "no response".into())))
        }
        Err(err) if matches!(err.code, ErrorCode::NotFound | ErrorCode::PolicyBlocked | ErrorCode::PolicyApproval) => {
            Err(err)
        }
        Err(err) => Ok(Err(format!("{}: {}", err.code.as_str(), err.message))),
    }
}

/// The newest captured calls (summaries).
pub(crate) async fn recent_captures(ctx: &CommandContext, workspace: &str) -> Result<Vec<Value>> {
    let all =
        call(ctx, "capture.list", json!({ "workspaceId": workspace, "summary": true, "last": CAPTURE_WINDOW })).await?;
    Ok(all["entries"].as_array().cloned().unwrap_or_default())
}

pub(crate) async fn capture_entry(ctx: &CommandContext, workspace: &str, id: &str) -> Result<Option<Value>> {
    let got = call(ctx, "capture.get", json!({ "workspaceId": workspace, "id": id })).await?;
    Ok(got.get("entry").filter(|entry| entry.is_object()).cloned())
}

/// Whether a summary is a real (not mock-served) answered call to `method` and `path`.
pub(crate) fn is_call_to(entry: &Value, method: &str, path: &str) -> bool {
    entry["via"] != "mock"
        && entry["status"].is_u64()
        && text(entry, "method").eq_ignore_ascii_case(method)
        && bifurc_protocol::analysis::pairing::paths_match(path, &path_of(&text(entry, "url")))
}

pub(crate) async fn entities(ctx: &CommandContext, workspace: &str, kind: &str) -> Result<Vec<Value>> {
    let got = call(ctx, "entity.list", json!({ "workspaceId": workspace, "kind": kind })).await?;
    Ok(got.get("entities").and_then(Value::as_array).cloned().unwrap_or_default())
}

pub(crate) fn id_of(entity: &Value) -> String {
    js(entity.get("id"))
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("contract.check", contract_check::check);
    registry.register("types.generate", types_generate::generate);
}

/// The row `wanted` names among `rows`: by id, else by exact name, else in any case, else by a unique name prefix.
pub(crate) fn find_named<'a>(rows: &'a [Value], wanted: &str, what: &str) -> Result<&'a Value> {
    let by = |matches: &dyn Fn(&Value) -> bool| -> Vec<&'a Value> { rows.iter().filter(|r| matches(r)).collect() };
    let lower = wanted.to_lowercase();
    for hits in [
        by(&|r| id_of(r) == wanted),
        by(&|r| text(r, "name") == wanted),
        by(&|r| text(r, "name").to_lowercase() == lower),
        by(&|r| text(r, "name").to_lowercase().starts_with(&lower)),
    ] {
        match hits.as_slice() {
            [] => continue,
            [one] => return Ok(one),
            _ => return Err(EngineError::bad_request(format!("several {what}s are named \"{wanted}\": use the id"))),
        }
    }
    Err(EngineError::not_found(format!("no {what} \"{wanted}\"")))
}
