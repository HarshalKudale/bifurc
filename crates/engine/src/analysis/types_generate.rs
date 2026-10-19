//! `types.generate`: the type of what a saved request answers, inferred from its recorded responses
//! (`response.history`), the captured calls to the same method and path, and optionally a send made now.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bifurc_protocol::analysis::{
    infer::{Lang, emit, infer, pascal},
    pairing::path_of,
    shape::Side,
};
use serde_json::{Value, json};

use super::{
    MAX_CAPTURES, call, capture_entry, capture_side, entities, find_named, id_of, is_call_to, recent_captures,
    send_saved, text,
};
use crate::{
    kernel::{CommandContext, Scope},
    protocol::{EngineError, ErrorCode, Result},
    utils::json::{bool_of, opt_str, str_of},
};

/// The JSON body of a 2xx answer.
fn json_of(side: &Side) -> Option<Value> {
    side.body.clone().filter(|_| (200..300).contains(&side.status))
}

/// A response-history entry: status, headers as an object, body as text or base64.
fn history_side(entry: &Value) -> Side {
    let status = entry["status"].as_u64().unwrap_or(0) as u16;
    let content_type = entry["headers"]
        .as_object()
        .and_then(|h| h.iter().find(|(name, _)| name.eq_ignore_ascii_case("content-type")))
        .and_then(|(_, value)| value.as_str())
        .unwrap_or_default();
    let body = entry["body"].as_str().unwrap_or_default();
    let raw = if entry["encoding"] == "base64" { STANDARD.decode(body).unwrap_or_default() } else { body.into() };
    Side::new(status, content_type, &[], raw)
}

fn valid_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(super) async fn generate(params: Value, ctx: CommandContext) -> Result<Value> {
    let send = bool_of(&params, "send", false);
    // The command is scoped Read; sending is a call out, so it takes Execute.
    let scopes = ctx.session.as_ref().and_then(|s| s.scopes.as_ref());
    if send && scopes.is_some_and(|granted| !granted.contains(&Scope::Execute)) {
        return Err(EngineError::new(ErrorCode::Forbidden, "types.generate with send needs the execute scope"));
    }
    let lang = match opt_str(&params, "lang") {
        None => Lang::Ts,
        Some(name) => Lang::parse(name)
            .ok_or_else(|| EngineError::bad_request(format!("lang must be ts, zod, python or go, not \"{name}\"")))?,
    };
    if let Some(given) = opt_str(&params, "name").filter(|given| !valid_name(given)) {
        return Err(EngineError::bad_request(format!("name must be an identifier, not \"{given}\"")));
    }
    let workspace = str_of(&params, "workspaceId");
    let rows = entities(&ctx, workspace, "requests").await?;
    let request = find_named(&rows, str_of(&params, "request"), "request")?;
    let (id, name) = (id_of(request), text(request, "name"));

    let mut samples: Vec<Value> = Vec::new();
    let history = call(&ctx, "response.history", json!({ "workspaceId": workspace, "id": id })).await?;
    samples.extend(history["entries"].as_array().into_iter().flatten().map(history_side).filter_map(|s| json_of(&s)));
    // shortcut: only the newest MAX_CAPTURES matches of the last captures are read; raise them if a busy proxy
    // buries older samples.
    let (method, path) = (text(request, "method"), path_of(&text(request, "url")));
    let hits: Vec<Value> = recent_captures(&ctx, workspace)
        .await?
        .into_iter()
        .filter(|entry| {
            entry["status"].as_u64().is_some_and(|s| (200..300).contains(&s)) && is_call_to(entry, &method, &path)
        })
        .take(MAX_CAPTURES)
        .collect();
    for hit in hits {
        if let Some(entry) = capture_entry(&ctx, workspace, &text(&hit, "id")).await? {
            samples.extend(json_of(&capture_side(&entry)));
        }
    }

    if send {
        match send_saved(&ctx, &params, &id).await? {
            Ok(side) => samples.extend(json_of(&side)),
            Err(failure) => return Err(EngineError::upstream(failure)),
        }
    }
    if samples.is_empty() {
        return Err(EngineError::not_found(format!("no JSON response for \"{name}\" yet"))
            .keyed("types.no_samples", json!({ "name": name })));
    }

    let type_name = opt_str(&params, "name").map_or_else(|| format!("{}Response", pascal(&name)), str::to_string);
    let count = samples.len();
    let header = format!("Inferred by bifurc from {count} response{} of \"{name}\"", if count == 1 { "" } else { "s" });
    let code = emit(lang, &type_name, &infer(&samples), &header);
    Ok(json!({ "name": type_name, "lang": lang.name(), "samples": count, "code": code }))
}
