//! `types <request>`: the type of what a saved request answers, inferred from its recorded responses and the
//! captured calls to the same method and path (see `infer.rs`), as TypeScript, zod, Python or Go.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    capture::{self, response_side},
    commands::{Context, find_one, id_of, need, workspace_id},
    contract::{path_of, paths_match},
    error::{CliError, Result},
    infer::Lang,
    requests::{self, TYPES, saved, text},
    response::Response,
    shape::Side,
};

/// How many captured entries are looked through, newest first, and how many matching ones are read.
const CAPTURE_WINDOW: u64 = 500;
const MAX_CAPTURES: usize = 20;

/// A response-history entry (`response.history`): status, headers as an object, body as text or base64.
pub(crate) fn history_side(entry: &Value) -> Side {
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

/// The recorded responses of a saved request (`response.history`), whatever their status.
pub(crate) fn history_sides(ctx: &mut Context, workspace: &str, id: &str) -> Result<Vec<Side>> {
    let history = ctx.request("response.history", json!({ "workspaceId": workspace, "id": id }))?;
    Ok(history["entries"].as_array().into_iter().flatten().map(history_side).collect())
}

/// The newest captured calls (summaries), to match against several requests with [`capture_sides`].
pub(crate) fn recent_captures(ctx: &mut Context, workspace: &str) -> Result<Value> {
    capture::summaries(ctx, workspace, json!({ "last": CAPTURE_WINDOW }))
}

/// The answers captured for the calls with the same method and path as `request` (mock-served ones skipped)
/// whose status `keep` accepts, newest first.
// shortcut: only the newest MAX_CAPTURES matches of the last CAPTURE_WINDOW captures are read; raise them if a
// busy proxy buries older samples.
pub(crate) fn capture_sides(
    ctx: &mut Context,
    workspace: &str,
    all: &Value,
    request: &Value,
    keep: fn(u16) -> bool,
) -> Result<Vec<Side>> {
    let (method, path) = (text(request, "method"), path_of(&text(request, "url")));
    let hits = all["entries"].as_array().into_iter().flatten().filter(|entry| {
        entry["via"] != "mock"
            && entry["status"].as_u64().is_some_and(|s| keep(s as u16))
            && text(entry, "method").eq_ignore_ascii_case(&method)
            && paths_match(&path, &path_of(&text(entry, "url")))
    });
    let mut sides = Vec::new();
    for entry in hits.take(MAX_CAPTURES) {
        let got = ctx.request("capture.get", json!({ "workspaceId": workspace, "id": text(entry, "id") }))?;
        if let Some(entry) = got.get("entry").filter(|e| e.is_object()) {
            sides.push(response_side(&Response::from_capture(entry)));
        }
    }
    Ok(sides)
}

/// `types <request> [--lang ..] [--name ..] [--send] [--out <file>]`: the engine's `types.generate` infers the
/// type; this names the request, writes the file and maps the engine's refusals to exit codes.
pub fn types(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    ctx.accept_top(&["lang", "out", "name", "send", "ts", "zod", "python", "go", "var", "env", "fresh-session"], 1)?;
    let args = ctx.args;
    let wanted = need(sub, "a request name")?;
    let shorthands: Vec<&str> = ["ts", "zod", "python", "go"].into_iter().filter(|l| args.has(l)).collect();
    let lang_name = match (args.flag("lang"), shorthands.as_slice()) {
        (Some(_), [_, ..]) | (None, [_, _, ..]) => {
            return Err(CliError::usage("give one language: --lang <ts|zod|python|go>"));
        }
        (Some(lang), []) => lang,
        (None, [one]) => one,
        (None, []) => "ts",
    };
    if Lang::parse(lang_name).is_none() {
        return Err(CliError::usage(format!("--lang must be ts, zod, python or go, not \"{lang_name}\"")));
    }
    if let Some(given) = args.flag("name").filter(|given| {
        !(given.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && given.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    }) {
        return Err(CliError::usage(format!("--name must be an identifier, not \"{given}\"")));
    }

    let workspace = workspace_id(ctx)?;
    let entities: Vec<Value> = saved(ctx, &workspace, &[&TYPES[0]])?.into_iter().map(|(_, entity)| entity).collect();
    let request = find_one(&entities, &wanted, &["name"], "request", "requests")?.clone();
    let (id, name) = (id_of(&request), text(&request, "name"));

    let mut params = json!({ "workspaceId": workspace, "request": id, "lang": lang_name });
    if let Some(given) = args.flag("name") {
        params["name"] = json!(given);
    }
    if args.has("send") {
        params["send"] = json!(true);
        if args.has("fresh-session") {
            params["freshJar"] = json!(true);
        }
        requests::add_scope(ctx, &workspace, &mut params)?;
    }
    let got = match ctx.request("types.generate", params) {
        Ok(got) => got,
        Err(CliError::Engine { message, .. }) if message.starts_with("no JSON response for") => {
            let hint = if args.has("send") { "" } else { " (or add --send)" };
            return Err(CliError::not_found(format!(
                "no JSON response for \"{name}\" yet; send it first: bifurc requests send \"{name}\"{hint}"
            )));
        }
        // A send that got no response at all.
        Err(CliError::Engine { code, message }) if code == "UPSTREAM_FAILED" && args.has("send") => {
            return Err(CliError::network(message));
        }
        Err(other) => return Err(other),
    };
    let (code, count) = (text(&got, "code"), got["samples"].as_u64().unwrap_or(0));

    let saved_to = match args.flag("out") {
        Some(path) => {
            if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| CliError::msg(format!("cannot write {path}: {e}")))?;
            }
            std::fs::write(path, &code).map_err(|e| CliError::msg(format!("cannot write {path}: {e}")))?;
            Some(path)
        }
        None => None,
    };
    let result = json!({
        "schemaVersion": 1, "name": got["name"], "lang": got["lang"], "samples": count, "code": code,
    });
    Ok(ctx.print(&result, || match saved_to {
        Some(path) => format!("saved {path} ({count} samples)"),
        None => code.trim_end().to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        run(argv, answers, |ctx| {
            let sub = ctx.args.positional(1).map(str::to_string);
            types(ctx, sub.as_deref())
        })
    }

    fn engine(extra: Vec<Answer>) -> Vec<Answer> {
        let mut answers = vec![
            ("config.get", Ok(config())),
            (
                "entity.list:requests",
                Ok(json!({"entities": [
                    {"id": "r1", "name": "Get orders", "method": "GET", "url": "{{baseUrl}}/orders/{{id}}"},
                    {"id": "r2", "name": "Make order", "method": "POST", "url": "{{baseUrl}}/orders"}]})),
            ),
        ];
        answers.extend(extra);
        answers
    }

    fn generated(lang: &str, name: &str) -> Answer {
        (
            "types.generate",
            Ok(json!({"name": name, "lang": lang, "samples": 3,
                      "code": "// Inferred by bifurc from 3 responses of \"Get orders\"
export type X = {};
"})),
        )
    }

    #[test]
    fn the_engine_infers_for_the_request_the_name_picks_and_the_code_prints_trimmed() {
        let (out, calls) = go(&["types", "get ord"], engine(vec![generated("ts", "GetOrdersResponse")]));
        assert_eq!(
            out.unwrap(),
            "// Inferred by bifurc from 3 responses of \"Get orders\"
export type X = {};"
        );
        let sent = &payloads(&calls, "types.generate")[0];
        assert_eq!(
            (sent["request"].clone(), sent["lang"].clone(), sent["workspaceId"].clone()),
            (json!("r1"), json!("ts"), json!("w1"))
        );
        assert!(sent.get("send").is_none() && sent.get("name").is_none());
    }

    #[test]
    fn no_json_sample_is_exit_3_and_json_flag_lang_and_name_shape_the_output() {
        let none = Err(CliError::Engine {
            code: "NOT_FOUND".into(),
            message: "no JSON response for \"Get orders\" yet".into(),
        });
        let err = go(&["types", "Get orders"], engine(vec![("types.generate", none.clone())])).0.unwrap_err();
        assert_eq!(err.code(), 3);
        assert_eq!(
            err.to_string(),
            "no JSON response for \"Get orders\" yet; send it first: bifurc requests send \"Get orders\" (or add --send)"
        );
        let err = go(&["types", "Get orders", "--send"], engine(vec![("types.generate", none)])).0.unwrap_err();
        assert!(err.to_string().ends_with("bifurc requests send \"Get orders\""), "{err}");

        let (out, calls) =
            go(&["types", "Get orders", "--zod", "--name", "Order", "--json"], engine(vec![generated("zod", "Order")]));
        let value: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(
            (value["schemaVersion"].clone(), value["name"].clone(), value["lang"].clone(), value["samples"].clone()),
            (json!(1), json!("Order"), json!("zod"), json!(3))
        );
        let sent = &payloads(&calls, "types.generate")[0];
        assert_eq!((sent["lang"].clone(), sent["name"].clone()), (json!("zod"), json!("Order")));
    }

    #[test]
    fn send_is_passed_on_out_writes_the_file_and_a_send_with_no_answer_is_exit_4() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("src/types/orders.ts");
        let answers = engine(vec![generated("ts", "GetOrdersResponse"), ("entity.list", Ok(json!({"entities": []})))]);
        let (out, calls) =
            go(&["types", "Get orders", "--send", "--var", "a=b", "--out", file.to_str().unwrap()], answers);
        assert_eq!(out.unwrap(), format!("saved {} (3 samples)", file.display()));
        let sent = &payloads(&calls, "types.generate")[0];
        assert_eq!((sent["send"].clone(), sent["variables"]["a"].clone()), (json!(true), json!("b")));
        let code = std::fs::read_to_string(&file).unwrap();
        assert!(
            code.starts_with(
                "// Inferred by bifurc from 3 responses of \"Get orders\"
"
            ),
            "{code}"
        );

        let down = Err(CliError::Engine { code: "UPSTREAM_FAILED".into(), message: "no response".into() });
        let err = go(&["types", "Get orders", "--send"], engine(vec![("types.generate", down)])).0.unwrap_err();
        assert_eq!((err.code(), err.to_string().as_str()), (4, "no response"));
    }

    #[test]
    fn a_bad_language_or_name_is_a_usage_error_before_the_engine_is_asked() {
        assert_eq!(go(&["types", "Get orders", "--lang", "rust"], engine(vec![])).0.unwrap_err().code(), 2);
        assert_eq!(go(&["types", "Get orders", "--go", "--ts"], engine(vec![])).0.unwrap_err().code(), 2);
        let (out, calls) = go(&["types", "Get orders", "--name", "1 bad"], engine(vec![generated("ts", "X")]));
        assert_eq!(out.unwrap_err().code(), 2);
        assert!(payloads(&calls, "types.generate").is_empty());
    }
}
