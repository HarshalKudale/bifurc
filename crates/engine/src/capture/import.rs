//! Saving captured exchanges as requests without what must not be stored with them: secrets become variables of
//! the selected environment, cookies go to the workspace's jar. Several can also become a flow that sends them in
//! the order they were captured, each wired to the earlier answer the values it used came from.
//!
//! `capture.inspectImport` says what an import would take out, so the user can choose; `capture.import` does it.

use std::collections::{BTreeMap, BTreeSet};

use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};

use super::{
    captured_to_request,
    entities::{b64_to_text, header_of},
    get_capture,
};
use crate::{
    cookies,
    env::{apply_variable_changes, load_merged_environment},
    kernel::{CommandContext, CommandRegistry},
    protocol::{EngineError, Result},
    utils::json::{array_of, bool_of, opt_str, str_of},
    vault::MIN_SECRET_LEN,
};

/// How far apart the nodes of a built flow are, left to right.
const NODE_STEP: f64 = 360.0;

/// Whether a header, query parameter or body field by this name holds a credential.
fn is_sensitive(name: &str) -> bool {
    let name: String = name.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    matches!(name.as_str(), "key" | "sig" | "pwd")
        || ["token", "secret", "password", "passwd", "apikey", "session", "signature", "credential", "authorization"]
            .iter()
            .any(|word| name.contains(word))
        || (name.contains("auth") && !name.contains("author"))
}

/// A value a request may have taken from an earlier answer: an id or a token, never a word or a small number.
fn is_id_like(value: &str) -> bool {
    value.len() >= 4
        && !value.contains("{{")
        && !value.contains(char::is_whitespace)
        && (value.len() >= 16 || value.contains(|c: char| c.is_ascii_digit()))
}

/// `X-Api-Key` as `x_api_key`.
fn variable_base(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_');
    match out.chars().next() {
        None => "value".to_string(),
        Some(first) if first.is_ascii_digit() => format!("v_{out}"),
        Some(_) => out.to_string(),
    }
}

fn reference(name: &str) -> String {
    format!("{{{{{name}}}}}")
}

fn decoded(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

/// `Bearer abc` as `("Bearer ", "abc")`; a value with no scheme is all secret.
fn split_scheme(value: &str) -> (&str, &str) {
    match value.split_once(' ') {
        Some((scheme, rest))
            if !rest.is_empty()
                && !rest.contains(' ')
                && scheme.len() <= 16
                && scheme.chars().all(|c| c.is_ascii_alphabetic()) =>
        {
            value.split_at(scheme.len() + 1)
        }
        _ => ("", value),
    }
}

/// What a captured request is called: the last part of its path, or its host.
fn name_of(url: &str) -> String {
    let address = url.split(['?', '#']).next().unwrap_or(url);
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    path.rsplit('/').find(|segment| !segment.is_empty()).unwrap_or(host).to_string()
}

struct Named {
    name: String,
    value: String,
    secret: bool,
    /// Where it was found, for the user to recognise it.
    places: Vec<String>,
    /// Not in the environment yet.
    new: bool,
}

/// The variables an import names, over those the environment already has. With a vault, secrets are named
/// against the vault's secrets instead.
struct Variables {
    existing: BTreeMap<String, String>,
    vault: Option<BTreeMap<String, String>>,
    named: Vec<Named>,
}

impl Variables {
    fn of_environment(ws_id: &str) -> Self {
        let existing = load_merged_environment(ws_id)
            .map(|env| {
                array_of(&env, "variables")
                    .iter()
                    .filter(|v| !str_of(v, "key").is_empty())
                    .map(|v| (str_of(v, "key").to_string(), str_of(v, "value").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Self { existing, vault: None, named: Vec::new() }
    }

    /// The variable that holds `value`: one already named for it, one of the environment (or, for a secret, the
    /// vault) that holds it, or a new one called after `base`.
    fn name(&mut self, base: &str, value: &str, secret: bool, place: &str) -> String {
        if let Some(found) = self.named.iter_mut().find(|named| named.value == value) {
            found.places.push(place.to_string());
            found.secret |= secret;
            return found.name.clone();
        }
        let pool = match &self.vault {
            Some(vault) if secret => vault,
            _ => &self.existing,
        };
        let held = pool.iter().find(|(_, held)| *held == value).map(|(name, _)| name.clone());
        let name = held.clone().unwrap_or_else(|| {
            // A vault's secret names are at most 64 characters and match whatever their case.
            let base: String = variable_base(base).chars().take(56).collect();
            (1..)
                .map(|n| if n == 1 { base.clone() } else { format!("{base}_{n}") })
                .find(|name| {
                    !pool.keys().any(|taken| taken.eq_ignore_ascii_case(name))
                        && !self.named.iter().any(|named| named.name.eq_ignore_ascii_case(name))
                })
                .unwrap_or(base)
        });
        self.named.push(Named {
            name: name.clone(),
            value: value.to_string(),
            secret,
            places: vec![place.to_string()],
            new: held.is_none(),
        });
        name
    }
}

fn headers_mut(request: &mut Value) -> Option<&mut Map<String, Value>> {
    request.get_mut("headers")?.as_object_mut()
}

/// `a=1&b=2` with each value `name_of(key, raw value)` names replaced by a reference to that variable.
fn replace_pairs(text: &str, mut name_of: impl FnMut(&str, &str) -> Option<String>) -> String {
    text.split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, raw)) if !raw.is_empty() && !raw.contains("{{") => match name_of(&decoded(key), raw) {
                Some(name) => format!("{key}={}", reference(&name)),
                None => pair.to_string(),
            },
            _ => pair.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Each text of `value`, with the key it is under, replaced by a reference to the variable `visit` names for it.
fn replace_strings(value: &mut Value, key: &str, visit: &mut dyn FnMut(&str, &str) -> Option<String>) -> bool {
    match value {
        Value::String(text) if !text.is_empty() && !text.contains("{{") => match visit(key, text) {
            Some(name) => {
                *text = reference(&name);
                true
            }
            None => false,
        },
        Value::Object(map) => {
            map.iter_mut().fold(false, |changed, (key, value)| replace_strings(value, key, visit) | changed)
        }
        Value::Array(items) => {
            items.iter_mut().fold(false, |changed, item| replace_strings(item, key, visit) | changed)
        }
        _ => false,
    }
}

/// [`replace_strings`] over a JSON body, or [`replace_pairs`] over a form. Other bodies are left as they are.
fn edit_body(request: &mut Value, visit: &mut dyn FnMut(&str, &str) -> Option<String>) {
    if request.get("bodyEncoding").is_some() {
        return;
    }
    let body = str_of(request, "body").to_string();
    if body.trim().is_empty() {
        return;
    }
    if header_of(request.get("headers"), "content-type").to_lowercase().contains("x-www-form-urlencoded") {
        request["body"] = json!(replace_pairs(&body, |key, raw| visit(key, raw)));
        return;
    }
    let Ok(mut parsed) = serde_json::from_str::<Value>(&body) else { return };
    if replace_strings(&mut parsed, "", visit) {
        let text =
            if body.contains('\n') { serde_json::to_string_pretty(&parsed) } else { serde_json::to_string(&parsed) };
        request["body"] = json!(text.unwrap_or(body));
    }
}

/// Credentials of the request's headers, query and body moved into variables. An `Authorization` header keeps
/// its scheme: `Bearer {{authorization}}`.
fn strip_secrets(request: &mut Value, place: &str, variables: &mut Variables) {
    if let Some(headers) = headers_mut(request) {
        for (name, value) in headers.iter_mut() {
            let Some(text) = value.as_str().filter(|text| !text.trim().is_empty() && !text.contains("{{")) else {
                continue;
            };
            if !is_sensitive(name) {
                continue;
            }
            let (scheme, secret) = split_scheme(text);
            let variable = variables.name(name, secret, true, &format!("{place} · header {name}"));
            *value = json!(format!("{scheme}{}", reference(&variable)));
        }
    }
    let url = str_of(request, "url").to_string();
    if let Some((address, query)) = url.split_once('?') {
        let query = replace_pairs(query, |key, raw| {
            is_sensitive(key).then(|| variables.name(key, raw, true, &format!("{place} · query {key}")))
        });
        request["url"] = json!(format!("{address}?{query}"));
    }
    edit_body(request, &mut |key, value| {
        is_sensitive(key).then(|| variables.name(key, value, true, &format!("{place} · body {key}")))
    });
}

/// The request's `Cookie` header taken out, as its name and value pairs.
fn take_cookies(request: &mut Value) -> Vec<(String, String)> {
    let Some(headers) = headers_mut(request) else { return Vec::new() };
    let text = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .filter_map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(";");
    headers.retain(|name, _| !name.eq_ignore_ascii_case("cookie"));
    text.split(';')
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// A captured answer as a flow's request node gives it on: its headers, and its body read as JSON.
fn answer_of(entry: &Value) -> Value {
    let body = b64_to_text(str_of(entry, "resBody"));
    json!({
        "headers": entry.get("resHeaders").cloned().unwrap_or(Value::Null),
        "body": serde_json::from_str::<Value>(&body).unwrap_or(Value::Null),
    })
}

/// The keys and indexes that lead to a text or number of `value` equal to `wanted`.
fn find_leaf(value: &Value, wanted: &str) -> Option<Vec<Value>> {
    let under = |key: Value, found: Option<Vec<Value>>| {
        found.map(|mut path| {
            path.insert(0, key);
            path
        })
    };
    match value {
        Value::String(text) if text == wanted => Some(Vec::new()),
        Value::Number(number) if number.to_string() == wanted => Some(Vec::new()),
        Value::Object(map) => map.iter().find_map(|(key, value)| under(json!(key), find_leaf(value, wanted))),
        Value::Array(items) => {
            items.iter().enumerate().find_map(|(at, item)| under(json!(at), find_leaf(item, wanted)))
        }
        _ => None,
    }
}

/// The latest of `answers` that holds `wanted`, and where in it.
fn source_of(answers: &[Value], wanted: &str) -> Option<(usize, Vec<Value>)> {
    answers.iter().enumerate().rev().find_map(|(at, answer)| find_leaf(answer, wanted).map(|path| (at, path)))
}

/// A value a node of the flow takes from the answer of an earlier one.
#[derive(Debug, PartialEq)]
struct Wire {
    from: usize,
    to: usize,
    variable: String,
    path: Vec<Value>,
}

/// `value` as a variable when an earlier answer holds it, named after where it is in that answer (`user_id`).
fn linked(
    value: &str,
    key: &str,
    place: &str,
    earlier: &[Value],
    variables: &mut Variables,
    found: &mut Vec<(String, usize, Vec<Value>)>,
) -> Option<String> {
    if !is_id_like(value) {
        return None;
    }
    let (from, path) = source_of(earlier, value)?;
    let keys: Vec<&str> = path.iter().skip(1).filter_map(Value::as_str).collect();
    let base = match keys.as_slice() {
        [.., parent, last] if last.len() <= 3 => format!("{parent}_{last}"),
        [.., last] => last.to_string(),
        [] => key.to_string(),
    };
    let name = variables.name(&base, value, false, place);
    found.push((name.clone(), from, path));
    Some(name)
}

/// The values of request `to` that an earlier answer gave (in its path, query and body, and the secrets it
/// uses) turned into variables, and the wires that bring them.
fn link(
    request: &mut Value,
    to: usize,
    place: &str,
    answers: &[Value],
    variables: &mut Variables,
    wires: &mut Vec<Wire>,
) {
    let earlier = &answers[..to];
    let mut found = Vec::new();

    let url = str_of(request, "url").to_string();
    let (address, query) = match url.split_once('?') {
        Some((address, query)) => (address, Some(query)),
        None => (url.as_str(), None),
    };
    let host_at = address.find("://").map_or(0, |at| at + 3);
    let (origin, path) = address.split_at(address[host_at..].find('/').map_or(address.len(), |at| host_at + at));
    let path = path
        .split('/')
        .map(|segment| {
            match linked(&decoded(segment), "id", &format!("{place} · path"), earlier, variables, &mut found) {
                Some(name) => reference(&name),
                None => segment.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    let query = query.map(|query| {
        replace_pairs(query, |key, raw| {
            linked(&decoded(raw), key, &format!("{place} · query {key}"), earlier, variables, &mut found)
        })
    });
    request["url"] = json!(match query {
        Some(query) => format!("{origin}{path}?{query}"),
        None => format!("{origin}{path}"),
    });
    edit_body(request, &mut |key, value| {
        linked(value, key, &format!("{place} · body {key}"), earlier, variables, &mut found)
    });

    let text = request.to_string();
    for named in variables.named.iter().filter(|named| named.secret && text.contains(&reference(&named.name))) {
        if let Some((from, path)) = source_of(earlier, &named.value) {
            found.push((named.name.clone(), from, path));
        }
    }
    for (variable, from, path) in found {
        if !wires.iter().any(|wire| wire.to == to && wire.variable == variable) {
            wires.push(Wire { from, to, variable, path });
        }
    }
}

/// Captured exchanges as the requests to save, with what was taken out of them.
struct Prepared {
    requests: Vec<Value>,
    variables: Variables,
    /// Each request's cookies by its address.
    cookies: Vec<(String, Vec<(String, String)>)>,
    wires: Vec<Wire>,
}

fn prepare(entries: &[Value], variables: Variables, strip: bool, jar: bool, chain: bool) -> Prepared {
    let answers: Vec<Value> = if chain { entries.iter().map(answer_of).collect() } else { Vec::new() };
    let mut prepared = Prepared { requests: Vec::new(), variables, cookies: Vec::new(), wires: Vec::new() };
    for (at, entry) in entries.iter().enumerate() {
        let mut request = captured_to_request(entry);
        let place = format!("{} {}", str_of(entry, "method"), name_of(str_of(entry, "url")));
        if jar {
            let pairs = take_cookies(&mut request);
            if !pairs.is_empty() {
                prepared.cookies.push((str_of(entry, "url").to_string(), pairs));
            }
        }
        if strip {
            strip_secrets(&mut request, &place, &mut prepared.variables);
        }
        if chain {
            link(&mut request, at, &place, &answers, &mut prepared.variables, &mut prepared.wires);
        }
        prepared.requests.push(request);
    }
    prepared
}

/// Each text of `value` with `from` replaced by `to`.
fn replace_text(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(text) if text.contains(from) => *text = text.replace(from, to),
        Value::Object(map) => map.values_mut().for_each(|value| replace_text(value, from, to)),
        Value::Array(items) => items.iter_mut().for_each(|item| replace_text(item, from, to)),
        _ => {}
    }
}

/// The secrets of an import into `vault`: each `{{name}}` becomes `{{vault.name}}`, and the values the vault does
/// not hold yet are returned to be sealed into it. A secret the flow hands on from an earlier answer stays a plain
/// variable that only the flow fills; one too short for a vault (it could not be kept out of logs) stays written
/// in the requests, and is returned second.
fn into_vault(prepared: &mut Prepared, vault: &str) -> (Vec<(String, String)>, Vec<String>) {
    let (mut sealed, mut kept) = (Vec::new(), Vec::new());
    for named in prepared.variables.named.iter().filter(|named| named.secret) {
        if prepared.wires.iter().any(|wire| wire.variable == named.name) {
            continue;
        }
        let to = if named.value.chars().count() < MIN_SECRET_LEN {
            kept.push(named.name.clone());
            named.value.clone()
        } else {
            if named.new {
                sealed.push((named.name.clone(), named.value.clone()));
            }
            reference(&format!("{vault}.{}", named.name))
        };
        for request in &mut prepared.requests {
            replace_text(request, &reference(&named.name), &to);
        }
    }
    (sealed, kept)
}

/// A flow that sends the saved requests one after the other, in the order they were captured. A node expects
/// the answer the capture saw: one that was outside 2xx and that the app went on from does not stop the run.
/// Every node's error port goes to one display node, where a run that stopped shows why.
fn build_flow(name: &str, entries: &[Value], request_ids: &[String], wires: &[Wire]) -> Value {
    let node_id = |at: usize| format!("request_{}", at + 1);
    let mut nodes = vec![json!({ "id": "start", "type": "start", "name": "", "x": 60.0, "y": 120.0 })];
    let mut edges: Vec<Value> = Vec::new();
    let edge = |from: &str, port: &str, to: &str, edges: &mut Vec<Value>| {
        edges.push(json!({ "id": format!("e{}", edges.len() + 1), "from": from, "port": port, "to": to }));
    };
    let mut previous = ("start".to_string(), "out");
    for (at, (entry, request_id)) in entries.iter().zip(request_ids).enumerate() {
        let id = node_id(at);
        let status = entry.get("resStatus").and_then(Value::as_u64);
        nodes.push(json!({
            "id": id, "type": "request", "name": name_of(str_of(entry, "url")),
            "x": 60.0 + NODE_STEP * (at + 1) as f64, "y": 120.0,
            "requestKind": "requests", "requestId": request_id,
            "failOnStatus": status.is_none_or(|status| (200..300).contains(&status)),
        }));
        edge(&previous.0, previous.1, &id, &mut edges);
        edge(&id, "error", "failed", &mut edges);
        previous = (id, "success");
    }
    nodes.push(json!({ "id": "failed", "type": "display", "name": "Failed", "x": 60.0 + NODE_STEP, "y": 420.0 }));
    for wire in wires {
        edges.push(json!({
            "id": format!("e{}", edges.len() + 1), "from": node_id(wire.from), "port": "success",
            "to": node_id(wire.to), "input": format!("var:{}", wire.variable), "path": wire.path,
        }));
    }
    json!({ "name": name, "inputs": [], "nodes": nodes, "edges": edges, "settings": {} })
}

fn secrets_report(variables: &Variables) -> Value {
    let preview = |value: &str| match value.chars().count() {
        0..=8 => "••••".to_string(),
        _ => format!("{}…", value.chars().take(4).collect::<String>()),
    };
    let secrets: Vec<Value> = variables
        .named
        .iter()
        .filter(|named| named.secret)
        .map(|named| json!({ "name": named.name, "preview": preview(&named.value), "places": named.places }))
        .collect();
    json!(secrets)
}

fn cookies_report(cookies: &[(String, Vec<(String, String)>)]) -> Value {
    let mut domains: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (url, pairs) in cookies {
        let host = url::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_default();
        domains.entry(host).or_default().extend(pairs.iter().map(|(name, _)| name.clone()));
    }
    let domains: Vec<Value> =
        domains.into_iter().map(|(domain, names)| json!({ "domain": domain, "names": names })).collect();
    json!(domains)
}

/// Hosts (with an optional path prefix) whose traffic is analytics, not the app's own.
const ANALYTICS: [&str; 14] = [
    "google-analytics.com",
    "googletagmanager.com",
    "analytics.google.com",
    "segment.io",
    "api.segment.io",
    "cdn.segment.com",
    "mixpanel.com",
    "api-js.mixpanel.com",
    "plausible.io",
    "sentry.io",
    "ingest.sentry.io",
    "hotjar.com",
    "doubleclick.net",
    "facebook.com/tr",
];

const ASSET_EXTENSIONS: [&str; 13] =
    ["png", "jpg", "jpeg", "gif", "svg", "ico", "webp", "woff", "woff2", "ttf", "css", "js", "map"];

/// Why an import leaves the exchange out by default: `preflights`, `analytics` or `assets`.
fn noise(entry: &Value) -> Option<&'static str> {
    if str_of(entry, "method").eq_ignore_ascii_case("OPTIONS") {
        return Some("preflights");
    }
    let url = url::Url::parse(str_of(entry, "url")).ok();
    let host = url.as_ref().and_then(|url| url.host_str()).unwrap_or_default().to_ascii_lowercase();
    let path = url.as_ref().map_or("", |url| url.path());
    let analytics = ANALYTICS.iter().any(|item| {
        let (name, prefix) = item.split_once('/').map_or((*item, ""), |(name, rest)| (name, rest));
        (host == name || host.ends_with(&format!(".{name}"))) && path.trim_start_matches('/').starts_with(prefix)
    });
    if analytics {
        return Some("analytics");
    }
    let content_type = header_of(entry.get("resHeaders"), "content-type").to_ascii_lowercase();
    let content_type = content_type.split(';').next().unwrap_or_default().trim();
    let by_type = content_type.starts_with("image/")
        || content_type.starts_with("font/")
        || matches!(content_type, "text/css" | "application/javascript" | "text/javascript");
    let extension = path.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase());
    let by_path = extension.is_some_and(|extension| ASSET_EXTENSIONS.contains(&extension.as_str()));
    (by_type || by_path).then_some("assets")
}

/// The entries an import keeps, and how many it left out for each reason.
fn without_noise(entries: Vec<Value>) -> (Vec<Value>, BTreeMap<&'static str, usize>) {
    let mut dropped = BTreeMap::new();
    let kept = entries
        .into_iter()
        .filter(|entry| match noise(entry) {
            Some(why) => {
                *dropped.entry(why).or_default() += 1;
                false
            }
            None => true,
        })
        .collect();
    (kept, dropped)
}

/// The captured entries `ids` names, oldest first.
fn entries_of(params: &Value) -> Result<Vec<Value>> {
    let ws_id = str_of(params, "workspaceId");
    let mut entries = Vec::new();
    for id in array_of(params, "ids").iter().filter_map(Value::as_str) {
        let entry = get_capture(ws_id, id).ok_or_else(|| {
            EngineError::not_found(format!("No captured entry \"{id}\" in workspace \"{ws_id}\"."))
                .keyed("capture.entry_not_found", serde_json::json!({ "id": id }))
        })?;
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(EngineError::bad_request("Name at least one captured entry.".to_string())
            .keyed("capture.name_entry", serde_json::json!({})));
    }
    entries.sort_by_key(|entry| entry.get("ts").and_then(Value::as_i64).unwrap_or(0));
    Ok(entries)
}

async fn invoke(ctx: &CommandContext, action: &str, payload: Value) -> Result<Value> {
    ctx.registry.invoke(action, payload, ctx.clone()).await
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("capture.inspectImport", |params, _| {
        let entries = entries_of(&params)?;
        let variables = Variables::of_environment(str_of(&params, "workspaceId"));
        let prepared = prepare(&entries, variables, true, true, false);
        Ok(json!({ "secrets": secrets_report(&prepared.variables), "cookies": cookies_report(&prepared.cookies) }))
    });

    // Secrets go nowhere (`keep`), to the selected environment, or into an unsealed vault (`vaultId`). Variables,
    // vault secrets and cookies are in place before the requests are saved, so the first send already has them.
    registry.register("capture.import", |params, ctx| async move {
        let ws_id = str_of(&params, "workspaceId").to_string();
        let (entries, dropped) = match bool_of(&params, "keepAll", false) {
            true => (entries_of(&params)?, BTreeMap::new()),
            false => without_noise(entries_of(&params)?),
        };
        if entries.is_empty() {
            return Err(EngineError::bad_request(
                "Every entry is a preflight, a static asset or an analytics call. keepAll saves them anyway.",
            ));
        }
        let flow = bool_of(&params, "flow", false) && entries.len() > 1;
        let destination = opt_str(&params, "secrets").unwrap_or("keep");
        let mut variables = Variables::of_environment(&ws_id);
        let vault = match (destination, opt_str(&params, "vaultId")) {
            ("vault", Some(vault_id)) => {
                let (vault_name, secrets) = crate::vault::read_unsealed(&ws_id, vault_id).await?;
                variables.vault = Some(secrets.into_iter().collect());
                Some((vault_id.to_string(), vault_name))
            }
            ("vault", None) => {
                return Err(EngineError::bad_request("Name the vault the secrets go into.".to_string())
                    .keyed("capture.name_vault", serde_json::json!({})));
            }
            _ => None,
        };
        let mut prepared =
            prepare(&entries, variables, destination != "keep", bool_of(&params, "cookiesToJar", false), flow);
        let name = opt_str(&params, "folderName").unwrap_or("Captured").to_string();

        let (sealed, kept) = match &vault {
            Some((vault_id, vault_name)) => {
                let (sealed, kept) = into_vault(&mut prepared, vault_name);
                let names: Vec<String> = sealed.iter().map(|(name, _)| name.clone()).collect();
                crate::vault::add_secrets(&ws_id, vault_id, sealed).await?;
                (names, kept)
            }
            None => (Vec::new(), Vec::new()),
        };
        let set: Vec<(String, String)> = prepared
            .variables
            .named
            .iter()
            .filter(|named| named.new && !(named.secret && vault.is_some()))
            .map(|named| (named.name.clone(), named.value.clone()))
            .collect();
        apply_variable_changes(&ws_id, &set, &[]);
        for (url, pairs) in &prepared.cookies {
            let ops: Vec<Value> =
                pairs.iter().map(|(name, value)| json!({ "op": "set", "name": name, "value": value })).collect();
            cookies::apply(&ws_id, url, &json!(ops));
        }

        let folder_id = if entries.len() > 1 {
            let folder = json!({ "kind": "request", "name": name, "parentId": null, "workspaceId": ws_id });
            Some(str_of(&invoke(&ctx, "folder.add", folder).await?, "id").to_string())
        } else {
            None
        };
        let mut request_ids = Vec::new();
        for (entry, mut request) in entries.iter().zip(prepared.requests) {
            request["name"] = json!(name_of(str_of(entry, "url")));
            if let Some(folder_id) = &folder_id {
                request["folderId"] = json!(folder_id);
            }
            let payload = json!({ "kind": "requests", "workspaceId": ws_id, "entity": request });
            request_ids.push(str_of(&invoke(&ctx, "entity.create", payload).await?, "id").to_string());
        }
        let flow_id = if flow {
            let entity = build_flow(&name, &entries, &request_ids, &prepared.wires);
            let payload = json!({ "kind": "flows", "workspaceId": ws_id, "entity": entity });
            Some(str_of(&invoke(&ctx, "entity.create", payload).await?, "id").to_string())
        } else {
            None
        };
        Ok(json!({
            "folderId": folder_id,
            "requestIds": request_ids,
            "flowId": flow_id,
            "variables": set.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            "vaultSecrets": sealed,
            "keptInRequests": kept,
            "cookies": prepared.cookies.iter().map(|(_, pairs)| pairs.len()).sum::<usize>(),
            "dropped": dropped,
        }))
    });
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    use super::*;
    use crate::flows::model::Flow;

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    fn none() -> Variables {
        Variables { existing: BTreeMap::new(), vault: None, named: Vec::new() }
    }

    fn login_then_orders() -> Vec<Value> {
        vec![
            json!({
                "id": "a", "ts": 1, "method": "POST", "url": "https://shop.test/api/login", "resStatus": 200,
                "reqHeaders": { "Content-Type": "application/json" },
                "reqBody": b64(r#"{"username":"ann","password":"hunter22"}"#),
                "resHeaders": { "content-type": "application/json" },
                "resBody": b64(r#"{"token":"tok-123456789","user":{"id":"user-42","name":"Ann"}}"#),
            }),
            json!({
                "id": "b", "ts": 2, "method": "GET", "url": "https://shop.test/api/users/user-42/orders?api_key=k-77&page=2",
                "resStatus": 404,
                "reqHeaders": { "Authorization": "Bearer tok-123456789", "Cookie": "sid=abc; theme=dark", "Accept": "*/*" },
                "reqBody": "",
                "resHeaders": {}, "resBody": "",
            }),
        ]
    }

    #[test]
    fn preflights_assets_and_analytics_are_noise() {
        let entry = |method: &str, url: &str, content_type: &str| json!({ "method": method, "url": url, "resHeaders": { "Content-Type": content_type } });
        let why = |method, url, content_type| noise(&entry(method, url, content_type));
        assert_eq!(why("OPTIONS", "https://shop.test/api/login", ""), Some("preflights"));
        assert_eq!(why("GET", "https://shop.test/logo", "image/png"), Some("assets"));
        assert_eq!(why("GET", "https://shop.test/app?v=2", "text/javascript; charset=utf-8"), Some("assets"));
        assert_eq!(why("GET", "https://shop.test/static/app.JS.map", ""), Some("assets"));
        assert_eq!(why("POST", "https://api.segment.io/v1/t", "application/json"), Some("analytics"));
        assert_eq!(why("GET", "https://o1.ingest.sentry.io/api/1/store", "application/json"), Some("analytics"));
        assert_eq!(why("POST", "https://www.facebook.com/tr?id=1", "image/gif"), Some("analytics"));
        assert_eq!(why("GET", "https://www.facebook.com/profile", "text/html"), None);
        assert_eq!(why("POST", "https://shop.test/api/login", "application/json"), None);
        assert_eq!(why("GET", "https://shop.test/api/data.json", "application/json"), None);
        let (kept, dropped) = without_noise(vec![
            entry("GET", "https://shop.test/a.css", "text/css"),
            entry("GET", "https://shop.test/api", "application/json"),
            entry("OPTIONS", "https://shop.test/api", ""),
        ]);
        assert_eq!((kept.len(), dropped), (1, BTreeMap::from([("assets", 1), ("preflights", 1)])));
    }

    #[test]
    fn names_that_hold_credentials() {
        for name in
            ["Authorization", "X-Api-Key", "access_token", "password", "client_secret", "X-Auth", "key", "sessionId"]
        {
            assert!(is_sensitive(name), "{name}");
        }
        for name in ["author", "Accept", "Content-Type", "page", "username", "monkey"] {
            assert!(!is_sensitive(name), "{name}");
        }
    }

    #[test]
    fn secrets_become_variables_and_cookies_leave_the_request() {
        let mut existing = BTreeMap::new();
        existing.insert("api_key".to_string(), "other".to_string());
        let variables = Variables { existing, vault: None, named: Vec::new() };
        let prepared = prepare(&login_then_orders(), variables, true, true, false);

        assert_eq!(prepared.requests[0]["body"], r#"{"username":"ann","password":"{{password}}"}"#);
        let orders = &prepared.requests[1];
        assert_eq!(orders["headers"], json!({ "Authorization": "Bearer {{authorization}}", "Accept": "*/*" }));
        // `api_key` is the environment's already, for another value.
        assert_eq!(orders["url"], "https://shop.test/api/users/user-42/orders?api_key={{api_key_2}}&page=2");
        let set: Vec<(&str, &str)> =
            prepared.variables.named.iter().map(|named| (named.name.as_str(), named.value.as_str())).collect();
        assert_eq!(set, [("password", "hunter22"), ("authorization", "tok-123456789"), ("api_key_2", "k-77")]);
        assert_eq!(
            prepared.cookies,
            [(
                "https://shop.test/api/users/user-42/orders?api_key=k-77&page=2".to_string(),
                vec![("sid".to_string(), "abc".to_string()), ("theme".to_string(), "dark".to_string())]
            )]
        );
        assert_eq!(cookies_report(&prepared.cookies), json!([{ "domain": "shop.test", "names": ["sid", "theme"] }]));
        assert_eq!(secrets_report(&prepared.variables)[1]["preview"], "tok-…");

        let kept = prepare(&login_then_orders(), none(), false, false, false);
        assert_eq!(kept.requests[1]["headers"]["Cookie"], "sid=abc; theme=dark");
        assert!(kept.variables.named.is_empty());
    }

    #[test]
    fn a_flow_wires_what_a_request_took_from_an_earlier_answer() {
        let entries = login_then_orders();
        let prepared = prepare(&entries, none(), true, true, true);
        assert_eq!(
            prepared.requests[1]["url"],
            "https://shop.test/api/users/{{user_id}}/orders?api_key={{api_key}}&page=2"
        );
        assert_eq!(
            prepared.wires,
            [
                Wire {
                    from: 0,
                    to: 1,
                    variable: "user_id".into(),
                    path: vec![json!("body"), json!("user"), json!("id")]
                },
                Wire { from: 0, to: 1, variable: "authorization".into(), path: vec![json!("body"), json!("token")] },
            ]
        );

        let flow = build_flow("Shop", &entries, &["r1".to_string(), "r2".to_string()], &prepared.wires);
        let parsed = Flow::parse(&flow).unwrap();
        let order: Vec<&str> = parsed.edges_from("start", "out").map(|edge| edge.to.as_str()).collect();
        assert_eq!(order, ["request_1"]);
        assert_eq!(parsed.edges_from("request_1", "success").next().unwrap().to, "request_2");
        assert_eq!(parsed.edges_from("request_2", "error").next().unwrap().to, "failed");
        // The capture saw a 404 the app went on from.
        assert!(parsed.node("request_1").unwrap().fail_on_status);
        assert!(!parsed.node("request_2").unwrap().fail_on_status);
        let wired: Vec<&str> = parsed.wires_into("request_2").map(|wire| wire.input.as_str()).collect();
        assert_eq!(wired, ["var:user_id", "var:authorization"]);
    }

    #[test]
    fn secrets_go_into_a_vault_unless_the_flow_hands_them_on_or_they_are_too_short() {
        let vault = BTreeMap::from([
            ("password".to_string(), "hunter22".to_string()),
            ("Authorization".to_string(), "x".repeat(9)),
        ]);
        let variables = Variables { existing: BTreeMap::new(), vault: Some(vault), named: Vec::new() };
        let mut prepared = prepare(&login_then_orders(), variables, true, true, false);
        let (sealed, kept) = into_vault(&mut prepared, "shop");
        // `password` is the vault's already with this value; `Authorization` is taken there for another.
        assert_eq!(sealed, [("authorization_2".to_string(), "tok-123456789".to_string())]);
        assert_eq!(kept, ["api_key"]);
        assert_eq!(prepared.requests[0]["body"], r#"{"username":"ann","password":"{{shop.password}}"}"#);
        assert_eq!(prepared.requests[1]["headers"]["Authorization"], "Bearer {{shop.authorization_2}}");
        assert_eq!(prepared.requests[1]["url"], "https://shop.test/api/users/user-42/orders?api_key=k-77&page=2");

        // In a flow the token comes from the login's answer, so the flow fills it and the vault is not asked.
        let variables = Variables { existing: BTreeMap::new(), vault: Some(BTreeMap::new()), named: Vec::new() };
        let mut prepared = prepare(&login_then_orders(), variables, true, true, true);
        let (sealed, _) = into_vault(&mut prepared, "shop");
        assert_eq!(sealed, [("password".to_string(), "hunter22".to_string())]);
        assert_eq!(prepared.requests[1]["headers"]["Authorization"], "Bearer {{authorization}}");
    }
}
