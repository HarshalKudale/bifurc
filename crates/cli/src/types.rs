//! `types <request>`: the type of what a saved request answers, inferred from its recorded responses and the
//! captured calls to the same method and path (see `infer.rs`), as TypeScript, zod, Python or Go.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    capture::{self, response_side},
    commands::{Context, find_one, id_of, need, workspace_id},
    contract::{path_of, paths_match},
    error::{CliError, Result},
    infer::{Lang, emit, infer, pascal},
    requests::{self, TYPES, saved, text},
    response::Response,
    shape::Side,
};

/// How many captured entries are looked through, newest first, and how many matching ones are read.
const CAPTURE_WINDOW: u64 = 500;
const MAX_CAPTURES: usize = 20;

fn is_2xx(status: u16) -> bool {
    (200..300).contains(&status)
}

/// The JSON body of a 2xx answer.
fn json_of(side: &Side) -> Option<Value> {
    side.body.clone().filter(|_| is_2xx(side.status))
}

/// A response-history entry (`response.history`): status, headers as an object, body as text or base64.
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
    let lang = Lang::parse(lang_name)
        .ok_or_else(|| CliError::usage(format!("--lang must be ts, zod, python or go, not \"{lang_name}\"")))?;

    let workspace = workspace_id(ctx)?;
    let entities: Vec<Value> = saved(ctx, &workspace, &[&TYPES[0]])?.into_iter().map(|(_, entity)| entity).collect();
    let request = find_one(&entities, &wanted, &["name"], "request", "requests")?.clone();
    let (id, name) = (id_of(&request), text(&request, "name"));

    let mut samples: Vec<Value> = Vec::new();
    let history = ctx.request("response.history", json!({ "workspaceId": workspace, "id": id }))?;
    samples.extend(history["entries"].as_array().into_iter().flatten().filter_map(|e| json_of(&history_side(e))));

    // shortcut: only the newest MAX_CAPTURES of the last CAPTURE_WINDOW captures are read; raise them if a busy
    // proxy buries older samples.
    let (method, path) = (text(&request, "method"), path_of(&text(&request, "url")));
    let all = capture::summaries(ctx, &workspace, json!({ "last": CAPTURE_WINDOW }))?;
    let hits = all["entries"].as_array().into_iter().flatten().filter(|entry| {
        entry["via"] != "mock"
            && entry["status"].as_u64().is_some_and(|s| is_2xx(s as u16))
            && text(entry, "method").eq_ignore_ascii_case(&method)
            && paths_match(&path, &path_of(&text(entry, "url")))
    });
    for entry in hits.take(MAX_CAPTURES) {
        let got = ctx.request("capture.get", json!({ "workspaceId": workspace, "id": text(entry, "id") }))?;
        if let Some(entry) = got.get("entry").filter(|e| e.is_object()) {
            samples.extend(json_of(&response_side(&Response::from_capture(entry))));
        }
    }

    if args.has("send") {
        let response = requests::send_saved(ctx, &workspace, &id)?;
        if response.status == 0 {
            return Err(CliError::network(response.failure.unwrap_or_else(|| "no response".into())));
        }
        samples.extend(json_of(&response_side(&response)));
    }
    if samples.is_empty() {
        let hint = if args.has("send") { "" } else { " (or add --send)" };
        return Err(CliError::not_found(format!(
            "no JSON response for \"{name}\" yet; send it first: bifurc requests send \"{name}\"{hint}"
        )));
    }

    let type_name = match args.flag("name") {
        Some(given)
            if given.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && given.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
        {
            given.to_string()
        }
        Some(given) => return Err(CliError::usage(format!("--name must be an identifier, not \"{given}\""))),
        None => format!("{}Response", pascal(&name)),
    };
    let count = samples.len();
    let header = format!("Inferred by bifurc from {count} response{} of \"{name}\"", if count == 1 { "" } else { "s" });
    let code = emit(lang, &type_name, &infer(&samples), &header);

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
    let result = json!({ "schemaVersion": 1, "name": type_name, "lang": lang.name(), "samples": count, "code": code });
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

    fn b64(text: &str) -> String {
        STANDARD.encode(text)
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
            (
                "response.history",
                Ok(json!({"entries": [
                    {"status": 200, "headers": {"Content-Type": "application/json"}, "body": r#"{"id":1,"note":null}"#},
                    {"status": 500, "headers": {"content-type": "application/json"}, "body": r#"{"error":"x"}"#},
                    {"status": 200, "headers": {"content-type": "application/json"}, "body": b64(r#"{"id":2,"total":1.5}"#), "encoding": "base64"},
                    {"status": 200, "headers": {"content-type": "text/html"}, "body": "<p>hi</p>"}]})),
            ),
            (
                "capture.list",
                Ok(json!({"entries": [
                    {"id": "c1", "method": "GET", "url": "http://h/orders/7?x=1", "status": 200},
                    {"id": "c2", "method": "GET", "url": "http://h/users/7", "status": 200},
                    {"id": "c3", "method": "POST", "url": "http://h/orders/7", "status": 200},
                    {"id": "c4", "method": "GET", "url": "http://h/orders/8", "status": 200, "via": "mock"}]})),
            ),
        ];
        answers.extend(extra);
        answers
    }

    fn capture_entry(body: &str) -> Answer {
        (
            "capture.get",
            Ok(
                json!({"entry": {"resStatus": 200, "resHeaders": {"content-type": "application/json"}, "resBody": b64(body)}}),
            ),
        )
    }

    #[test]
    fn samples_come_from_history_and_matching_captures_only() {
        let (out, calls) = go(&["types", "get ord"], engine(vec![capture_entry(r#"{"id":3,"extra":true}"#)]));
        assert_eq!(
            out.unwrap(),
            "// Inferred by bifurc from 3 responses of \"Get orders\"\nexport type GetOrdersResponse = {\n  id: number;\n  note?: null;\n  total?: number;\n  extra?: boolean;\n};"
        );
        assert_eq!(payloads(&calls, "capture.get").len(), 1);
        assert_eq!(payloads(&calls, "capture.get")[0]["id"], "c1");
        assert!(payloads(&calls, "request.send").is_empty());
    }

    #[test]
    fn no_json_sample_is_exit_3_and_json_flag_lang_and_name_shape_the_output() {
        let mut answers = engine(vec![]);
        answers[2] = ("response.history", Ok(json!({"entries": []})));
        answers[3] = ("capture.list", Ok(json!({"entries": []})));
        let err = go(&["types", "Get orders"], answers).0.unwrap_err();
        assert_eq!(err.code(), 3);
        assert_eq!(
            err.to_string(),
            "no JSON response for \"Get orders\" yet; send it first: bifurc requests send \"Get orders\" (or add --send)"
        );

        let (out, _) =
            go(&["types", "Get orders", "--zod", "--name", "Order", "--json"], engine(vec![capture_entry("{}")]));
        let value: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(
            (value["schemaVersion"].clone(), value["name"].clone(), value["lang"].clone()),
            (json!(1), json!("Order"), json!("zod"))
        );
        assert_eq!(value["samples"], 3);
        assert!(value["code"].as_str().unwrap().contains("export const Order = z.object({"));
    }

    #[test]
    fn send_adds_the_sent_response_and_out_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("src/types/orders.ts");
        let reply = json!({"status": 200, "durationMs": 3, "headers": {"content-type": "application/json"}, "body": b64(r#"{"id":9}"#)});
        let answers = engine(vec![
            capture_entry("{}"),
            ("entity.list", Ok(json!({"entities": []}))),
            ("request.send", Ok(reply)),
        ]);
        let (out, calls) = go(&["types", "Get orders", "--send", "--out", file.to_str().unwrap()], answers);
        assert_eq!(out.unwrap(), format!("saved {} (4 samples)", file.display()));
        assert_eq!(payloads(&calls, "request.send")[0]["id"], "r1");
        let code = std::fs::read_to_string(&file).unwrap();
        assert!(code.starts_with("// Inferred by bifurc from 4 responses of \"Get orders\"\n"), "{code}");
    }

    #[test]
    fn a_bad_language_or_name_is_a_usage_error() {
        assert_eq!(go(&["types", "Get orders", "--lang", "rust"], engine(vec![])).0.unwrap_err().code(), 2);
        assert_eq!(go(&["types", "Get orders", "--go", "--ts"], engine(vec![])).0.unwrap_err().code(), 2);
        assert_eq!(
            go(&["types", "Get orders", "--name", "1 bad"], engine(vec![capture_entry("{}")])).0.unwrap_err().code(),
            2
        );
    }
}
