//! Mock scenarios without a stored change: the mocks of one call (same method, URL pattern, regex flag and
//! captured request body, which is what the engine switches off when one of them is enabled) form a group, and a
//! mock named `<default name> · <scenario>` is a scenario of the group's default mock, the one without a suffix.

use serde_json::{Map, Value, json};

use super::{MOCKS, create, load, set_enabled_by_id};
use crate::{
    capture,
    commands::{Context, find_one, id_of, list_entities, merge_headers, need, refused, workspace_id},
    error::{CliError, Result},
    format::{aligned, js_template, truthy},
    requests::{add_overrides, type_named},
    response::{decode_base64, short_type, size},
};

#[cfg(test)]
mod tests;

const SEPARATOR: &str = " · ";

/// The scenarios any mock can switch to without defining them.
const PRESETS: [&str; 6] = ["error500", "error401", "notfound", "slow", "timeout", "empty"];

/// Headers that describe the connection, not the answer.
const NOT_REPLAYED: [&str; 10] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "set-cookie",
];

fn text<'a>(mock: &'a Value, key: &str) -> &'a str {
    mock.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The part of the name after the last ` · `, or `default`.
fn scenario_of(mock: &Value) -> &str {
    text(mock, "name").rsplit_once(SEPARATOR).map_or("default", |(_, scenario)| scenario)
}

fn is_on(mock: &Value) -> bool {
    mock.get("enabled").is_some_and(truthy)
}

fn call_key(mock: &Value) -> String {
    let regex = mock.get("useRegex").is_some_and(truthy);
    format!(
        "{}|{}|{regex}|{}",
        text(mock, "method").to_uppercase(),
        text(mock, "urlPattern"),
        text(mock, "capturedBody")
    )
}

/// The mocks split by call, in the order the calls first appear; in a group the default comes first (the oldest
/// when several have no suffix), then the scenarios by age.
fn groups(mocks: Vec<Value>) -> Vec<Vec<Value>> {
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();
    for mock in mocks {
        let key = call_key(&mock);
        match out.iter_mut().find(|(other, _)| *other == key) {
            Some((_, group)) => group.push(mock),
            None => out.push((key, vec![mock])),
        }
    }
    out.into_iter()
        .map(|(_, mut group)| {
            group.sort_by_key(|m| (scenario_of(m) != "default", m.get("createdAt").and_then(Value::as_i64)));
            group
        })
        .collect()
}

/// The name of the group's default mock; a group with only scenarios still has the name they share.
fn default_name(group: &[Value]) -> String {
    let name = text(&group[0], "name");
    name.rsplit_once(SEPARATOR).map_or(name, |(default, _)| default).to_string()
}

/// Every mock of the workspace with whether it is enabled.
fn all_mocks(ctx: &mut Context, workspace: &str) -> Result<Vec<Value>> {
    let mut mocks = Vec::new();
    for row in list_entities(ctx, workspace, MOCKS.kind)? {
        mocks.push(load(ctx, &MOCKS, workspace, &id_of(&row))?);
    }
    Ok(mocks)
}

/// The group of the mock `wanted` names.
fn group_of(ctx: &mut Context, workspace: &str, wanted: &str) -> Result<Vec<Value>> {
    let mocks = all_mocks(ctx, workspace)?;
    let key = call_key(find_one(&mocks, wanted, MOCKS.name_keys, MOCKS.what, MOCKS.group)?);
    Ok(groups(mocks).into_iter().find(|group| call_key(&group[0]) == key).expect("the mock is in a group"))
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A short description of the body: `3 items`, `[]`, `{}`, `2 keys`, or its size.
fn body_words(mock: &Value) -> String {
    if text(mock, "responseBodyEncoding") == "base64" {
        return size(decode_base64(mock.get("responseBody")).len());
    }
    let body = text(mock, "responseBody");
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(items)) if items.is_empty() => "[]".into(),
        Ok(Value::Array(items)) => count(items.len(), "item", "items"),
        Ok(Value::Object(keys)) if keys.is_empty() => "{}".into(),
        Ok(Value::Object(keys)) => count(keys.len(), "key", "keys"),
        _ if body.is_empty() => String::new(),
        _ => size(body.len()),
    }
}

fn delay_words(mock: &Value) -> Option<String> {
    mock.get("responseDelay").and_then(Value::as_u64).filter(|ms| *ms > 0).map(|ms| format!("delay {ms}ms"))
}

/// `200 []` and so on: the status, the body and the delay of a mock.
fn answer_words(mock: &Value) -> String {
    let status = mock.get("responseStatus").and_then(Value::as_u64).unwrap_or(200).to_string();
    [Some(status), Some(body_words(mock)), delay_words(mock)]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `--delay`, in milliseconds.
fn delay_flag(ctx: &Context) -> Result<Option<u64>> {
    ctx.args
        .flag("delay")
        .map(|raw| {
            raw.parse()
                .map_err(|_| CliError::usage(format!("--delay must be a whole number of milliseconds, not \"{raw}\"")))
        })
        .transpose()
}

fn set_json_type(mock: &mut Value) {
    let mut headers = mock.get("responseHeaders").and_then(Value::as_object).cloned().unwrap_or_default();
    merge_headers(&mut headers, &[("Content-Type".into(), "application/json".into())]);
    mock["responseHeaders"] = Value::Object(headers);
}

/// A copy of `default` that answers as the preset `name` says, or `None` when `name` is not a preset.
fn preset(default: &Value, name: &str, delay: Option<u64>) -> Option<Value> {
    if !PRESETS.contains(&name) {
        return None;
    }
    let mut mock = default.clone();
    if let Some(fields) = mock.as_object_mut() {
        for key in ["id", "createdAt", "updatedAt"] {
            fields.remove(key);
        }
    }
    for key in ["standalone", "responseStatusMocked", "responseBodyMocked", "responseDelayMocked"] {
        mock[key] = json!(true);
    }
    let failure = |mock: &mut Value, status: u64, message: &str| {
        mock["responseStatus"] = json!(status);
        mock["responseBody"] = json!(json!({ "error": message }).to_string());
        set_json_type(mock);
    };
    match name {
        "error500" => failure(&mut mock, 500, "Internal Server Error"),
        "error401" => failure(&mut mock, 401, "Unauthorized"),
        "notfound" => failure(&mut mock, 404, "Not Found"),
        "slow" => mock["responseDelay"] = json!(delay.unwrap_or(3000)),
        "timeout" => mock["responseDelay"] = json!(delay.unwrap_or(60_000)),
        _ => {
            // The shape of the default body: an empty array, an empty object, or nothing.
            let shape = match serde_json::from_str::<Value>(text(default, "responseBody")) {
                Ok(Value::Array(_)) if text(default, "responseBodyEncoding") != "base64" => "[]",
                Ok(Value::Object(_)) if text(default, "responseBodyEncoding") != "base64" => "{}",
                _ => "",
            };
            mock["responseStatus"] = json!(200);
            mock["responseBody"] = json!(shape);
            if !shape.is_empty() {
                set_json_type(&mut mock);
            }
        }
    }
    if !matches!(name, "slow" | "timeout")
        && let Some(fields) = mock.as_object_mut()
    {
        fields.remove("responseBodyEncoding");
    }
    Some(mock)
}

/// `mocks scenario <mock> [<scenario>] [--list] [--delay <ms>]`.
pub fn scenario(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["list", "delay"], 2)?;
    let args = ctx.args;
    let wanted = need(args.positional(2), "mock id or name")?;
    let delay = delay_flag(ctx)?;
    let workspace = workspace_id(ctx)?;
    let group = group_of(ctx, &workspace, &wanted)?;
    let name = default_name(&group);
    let Some(asked) = args.positional(3).filter(|_| !args.has("list")) else {
        let rows: Vec<Value> = group
            .iter()
            .map(|m| {
                json!({ "scenario": scenario_of(m), "active": is_on(m), "id": id_of(m),
                        "status": m.get("responseStatus"), "body": body_words(m), "delay": m.get("responseDelay") })
            })
            .collect();
        let off = !group.iter().any(is_on);
        return Ok(ctx.print(&Value::Array(rows), || {
            let cells: Vec<Vec<String>> = group
                .iter()
                .map(|m| {
                    let status = m.get("responseStatus").and_then(Value::as_u64).unwrap_or(200).to_string();
                    let rest = [Some(body_words(m)), delay_words(m), off.then(|| "(off)".to_string())];
                    let rest: Vec<String> = rest.into_iter().flatten().filter(|part| !part.is_empty()).collect();
                    vec![format!("{} {}", if is_on(m) { "*" } else { " " }, scenario_of(m)), status, rest.join("  ")]
                })
                .collect();
            aligned(&cells)
        }));
    };
    let asked = asked.to_lowercase();
    let existing = group.iter().find(|m| scenario_of(m).to_lowercase() == asked);
    if delay.is_some() && (existing.is_some() || !matches!(asked.as_str(), "slow" | "timeout")) {
        return Err(CliError::usage(
            "--delay sets the delay of a slow or timeout scenario that is created; change an existing one with: bifurc mocks update",
        ));
    }
    let mut target = match existing {
        Some(mock) => {
            set_enabled_by_id(ctx, &MOCKS, &workspace, &id_of(mock), true)?;
            mock.clone()
        }
        None => {
            let Some(mut mock) = preset(&group[0], &asked, delay) else {
                let known: Vec<&str> = group.iter().map(scenario_of).collect();
                return Err(CliError::not_found(format!(
                    "\"{name}\" has no scenario \"{asked}\": {} (presets: {})",
                    known.join(", "),
                    PRESETS.join(", ")
                )));
            };
            mock["name"] = json!(format!("{name}{SEPARATOR}{asked}"));
            mock["enabled"] = json!(true);
            let saved = create(ctx, &MOCKS, &workspace, mock.clone())?;
            mock["id"] = json!(id_of(&saved));
            mock
        }
    };
    target["enabled"] = json!(true);
    Ok(ctx.print(&target, || format!("{name} → {} ({})", scenario_of(&target), answer_words(&target))))
}

/// A pattern as the list shows it: a URL without scheme and host, anything else as it is.
fn short_pattern(pattern: &str) -> String {
    match pattern.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/".to_string(), |at| rest[at..].to_string()),
        None => pattern.to_string(),
    }
}

/// `mocks status`: a line for each call that has mocks.
pub fn status(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let workspace = workspace_id(ctx)?;
    let groups = groups(all_mocks(ctx, &workspace)?);
    let buffer = capture::summaries(ctx, &workspace, json!({}))?;
    let answered: Vec<&str> =
        buffer["entries"].as_array().into_iter().flatten().filter_map(|e| e["mockId"].as_str()).collect();
    let rows: Vec<Value> = groups
        .iter()
        .map(|group| {
            let on = group.iter().find(|m| is_on(m));
            let hits = answered.iter().filter(|id| group.iter().any(|m| m["id"].as_str() == Some(**id))).count();
            json!({
                "name": default_name(group), "method": text(&group[0], "method"),
                "pattern": text(&group[0], "urlPattern"), "scenario": on.map(scenario_of),
                "enabled": on.is_some(), "hits": hits,
                "mocks": group.iter().map(|m| json!({
                    "id": m["id"], "name": m["name"], "scenario": scenario_of(m), "enabled": is_on(m),
                    "status": m["responseStatus"]
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(ctx.print(&Value::Array(rows.clone()), || {
        if rows.is_empty() {
            return "(none)".to_string();
        }
        let cells: Vec<Vec<String>> = rows
            .iter()
            .map(|row| {
                vec![
                    js_template(row.get("name")),
                    js_template(row.get("method")),
                    short_pattern(&js_template(row.get("pattern"))),
                    row["scenario"].as_str().unwrap_or("-").to_string(),
                    if row["enabled"] == true { "on" } else { "off" }.to_string(),
                    count(row["hits"].as_u64().unwrap_or(0) as usize, "hit", "hits"),
                ]
            })
            .collect();
        aligned(&cells)
    }))
}

/// `mocks hits <mock> [--last N]`: the captured exchanges any mock of its call answered.
pub fn hits(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["last"], 1)?;
    let wanted = need(ctx.args.positional(2), "mock id or name")?;
    let last = capture::number(ctx, "last")?.unwrap_or(5);
    let workspace = workspace_id(ctx)?;
    let ids: Vec<String> = group_of(ctx, &workspace, &wanted)?.iter().map(id_of).collect();
    let answer = capture::summaries(ctx, &workspace, json!({ "mockId": ids.join(","), "last": last }))?;
    Ok(ctx.print(&answer, || capture::lines(&answer)))
}

/// The mock a sent request answered with: its status, headers and body, for its resolved URL.
fn mock_of_request(ctx: &mut Context, workspace: &str, wanted: &str) -> Result<Value> {
    let rows = list_entities(ctx, workspace, "requests")?;
    let request = find_one(&rows, wanted, &["name"], "request", "requests")?;
    let mut payload = json!({ "workspaceId": workspace, "id": id_of(request) });
    add_overrides(ctx, type_named("rest")?, workspace, None, &mut payload)?;
    let sent = refused(ctx.request("request.send", payload)?)?;
    let headers: Map<String, Value> = sent
        .get("headers")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(name, _)| !NOT_REPLAYED.contains(&name.to_lowercase().as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let method = sent["request"].get("method").filter(|m| m.is_string()).cloned().unwrap_or(json!("GET"));
    let entry = json!({
        "method": method,
        "url": sent["request"]["url"], "reqHeaders": {}, "reqBody": "",
        "resStatus": sent["status"], "resHeaders": headers, "resBody": sent["body"],
    });
    let mut mock = ctx.request("capture.buildMock", json!({ "entry": entry }))?["mock"].clone();
    mock["name"] = json!(text(request, "name"));
    Ok(mock)
}

/// `mocks create --from-request <request>` and `--from-capture <entry>`, each with `--name`.
pub fn create_from(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["from-request", "from-capture", "name", "disabled", "var", "env", "fresh-session"], 0)?;
    let args = ctx.args;
    let workspace = workspace_id(ctx)?;
    let mut mock = match (args.flag("from-request"), args.flag("from-capture")) {
        (Some(_), Some(_)) => return Err(CliError::usage("--from-request and --from-capture cannot both be given")),
        (Some(request), None) => mock_of_request(ctx, &workspace, request)?,
        (None, Some(entry)) => {
            let id = capture::entry_ids(ctx, &workspace, &[entry.to_string()])?.remove(0);
            ctx.request("capture.buildMock", json!({ "workspaceId": workspace, "id": id }))?["mock"].clone()
        }
        (None, None) => unreachable!("the caller checked"),
    };
    if let Some(name) = args.flag("name") {
        mock["name"] = json!(name);
    }
    if text(&mock, "name").is_empty() {
        mock["name"] = json!(format!("{} {}", text(&mock, "method"), text(&mock, "urlPattern")));
    }
    mock["enabled"] = json!(!args.has("disabled"));
    let saved = create(ctx, &MOCKS, &workspace, mock.clone())?;
    Ok(ctx.print(&saved, || {
        let body = if text(&mock, "responseBodyEncoding") == "base64" {
            decode_base64(mock.get("responseBody"))
        } else {
            text(&mock, "responseBody").as_bytes().to_vec()
        };
        let content_type = mock["responseHeaders"]
            .as_object()
            .and_then(|h| h.iter().find(|(name, _)| name.eq_ignore_ascii_case("content-type")))
            .map(|(_, value)| js_template(Some(value)))
            .unwrap_or_default();
        format!(
            "created mock \"{}\" ({} {}, {}, {} {})",
            text(&mock, "name"),
            text(&mock, "method"),
            text(&mock, "urlPattern"),
            js_template(mock.get("responseStatus")),
            size(body.len()),
            short_type(&content_type, &body)
        )
    }))
}
