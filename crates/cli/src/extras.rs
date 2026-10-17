//! The verbs of the groups in `direct.rs` that take more than one engine command: reading a tree's folders,
//! changing one variable of an environment, saving what was captured, running a folder to its end, and the
//! engine's settings.

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::{
    capture,
    commands::{Context, created, find_one, id_of, list_entities, need, refused, workspace_id},
    error::{CliError, Result},
    format::{aligned, js_template, json, table},
    response::{is_sensitive_name, mask, revealed},
    steps::{Step, lines, max_body, one_line, path_of},
};

/// The tree of each folder kind, as the engine's snapshot names its folders.
const TREES: [(&str, &str); 4] =
    [("request", "requestFolders"), ("mock", "mockFolders"), ("flow", "flowFolders"), ("rule", "ruleFolders")];

/// The settings of `settings show`: the engine's, without the workspace's items the snapshot also holds.
const SETTINGS: [&str; 6] =
    ["port", "forwardProxyPort", "companionPort", "tlsEnabled", "telemetryEnabled", "updateCheckEnabled"];

/// How often a run is asked whether it has finished.
const RUN_POLL: Duration = Duration::from_millis(300);

/// Run `group verb` when it is one of this module's; `None` when it is not.
pub fn run(ctx: &mut Context, group: &str, verb: &str) -> Option<Result<String>> {
    Some(match (group, verb) {
        ("folders", "list") => folders_list(ctx),
        ("cookies", "list") => cookies_list(ctx),
        ("cookies", "set") => crate::named::cookies_set(ctx),
        ("env", "show") => env_show(ctx),
        ("env", "set") => env_variables(ctx, true),
        ("env", "unset") => env_variables(ctx, false),
        ("capture", "list") => capture::list(ctx),
        ("capture", "show") => capture::show(ctx),
        ("capture", "diff") => capture::diff(ctx),
        ("capture", "pin") => capture::pin(ctx, true),
        ("capture", "unpin") => capture::pin(ctx, false),
        ("capture", "mock") => capture_save(ctx, "mocks"),
        ("capture", "request") => capture_save(ctx, "requests"),
        ("runner", "run") => runner_run(ctx),
        ("contract", "check") => crate::contract::check(ctx),
        ("contract", "update-mock") => crate::contract::update_mock(ctx),
        ("contract", "snapshot") => crate::contract::snapshot(ctx),
        ("settings", "show") => settings_show(ctx),
        _ => return None,
    })
}

/// `folders list --tree <tree>`: the folders of one tree of the active workspace.
fn folders_list(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["tree"], 0)?;
    let wanted = need(ctx.args.flag("tree"), "--tree request|mock|flow|rule")?;
    let Some((_, key)) = TREES.iter().find(|(tree, _)| *tree == wanted) else {
        return Err(CliError::usage(format!("--tree must be one of request, mock, flow, rule, not \"{wanted}\"")));
    };
    if ctx.args.flag("workspace").is_some() {
        return Err(CliError::msg(
            "\"folders list\" reads the active workspace: run bifurc workspaces use <workspace>",
        ));
    }
    let config = ctx.request("config.get", json!({}))?;
    let rows = config.get(*key).and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(ctx.print(&Value::Array(rows.clone()), || table(&rows, &["id", "name", "parentId", "deleted"])))
}

/// `cookies list [--domain <d>]`: the jar with every value masked (`--reveal` shows them on a terminal).
fn cookies_list(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["domain", "reveal"], 0)?;
    let workspace = workspace_id(ctx)?;
    let mut jar = ctx.request("cookies.list", json!({ "workspaceId": workspace }))?;
    let reveal = revealed(ctx.args);
    let wanted = ctx.args.flag("domain").map(|d| d.trim_start_matches('.').to_lowercase());
    let domains = jar.get_mut("domains").and_then(Value::as_array_mut);
    let mut cells: Vec<Vec<String>> = Vec::new();
    if let Some(domains) = domains {
        domains.retain(|d| {
            wanted.as_deref().is_none_or(|w| js_template(d.get("domain")).trim_start_matches('.').to_lowercase() == w)
        });
        for domain in domains.iter_mut() {
            let name = js_template(domain.get("domain"));
            for cookie in domain.get_mut("cookies").and_then(Value::as_array_mut).into_iter().flatten() {
                if !reveal {
                    cookie["value"] = json!(mask(&js_template(cookie.get("value"))));
                }
                let expires = match cookie.get("expires").and_then(Value::as_i64) {
                    Some(ms) => format!("expires {}", utc_minute(ms / 1000)),
                    None => "session".to_string(),
                };
                let flags = [("httpOnly", "HttpOnly"), ("secure", "Secure")]
                    .iter()
                    .filter(|(key, _)| cookie.get(*key).is_some_and(|v| v == true))
                    .map(|(_, label)| *label)
                    .collect::<Vec<_>>()
                    .join(" ");
                cells.push(vec![
                    name.clone(),
                    js_template(cookie.get("name")),
                    js_template(cookie.get("value")),
                    expires,
                    flags,
                ]);
            }
        }
    }
    Ok(ctx.print(&jar, || if cells.is_empty() { "(none)".to_string() } else { aligned(&cells) }))
}

/// `1970-01-01 00:00` for Unix seconds, in UTC (days to a civil date by Howard Hinnant's algorithm).
fn utc_minute(seconds: i64) -> String {
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let year_of_era = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let day_of_year = doe - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", rest / 3600, rest % 3600 / 60)
}

/// `env show`: the merged variables, those with a secret-looking name masked (`--reveal` shows them on a
/// terminal). `{{vault.x}}` references are not secrets and print as written.
fn env_show(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["reveal"], 0)?;
    let workspace = workspace_id(ctx)?;
    let mut merged = ctx.request("env.getMerged", json!({ "workspaceId": workspace }))?;
    if !revealed(ctx.args) {
        for variable in merged.get_mut("variables").and_then(Value::as_array_mut).into_iter().flatten() {
            let value = js_template(variable.get("value"));
            if is_sensitive_name(&js_template(variable.get("key"))) && !value.contains("{{vault.") {
                variable["value"] = json!(mask(&value));
            }
        }
    }
    // One `KEY  value` line per variable: the merged environment's own fields say nothing an agent needs.
    let rows: Vec<Vec<String>> = merged
        .get("variables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|variable| vec![js_template(variable.get("key")), js_template(variable.get("value"))])
        .collect();
    Ok(ctx.print(&merged, || aligned(&rows)))
}

/// `env set <env> KEY=VALUE...` and `env unset <env> KEY...`: change some variables and leave the others.
fn env_variables(ctx: &mut Context, set: bool) -> Result<String> {
    ctx.accept(&["force"], 1000)?;
    let args = ctx.args;
    let wanted = need(args.positional(2), "environment id or name")?;
    let given = args.positionals.get(3..).unwrap_or_default();
    if given.is_empty() {
        return Err(CliError::usage(if set { "missing KEY=VALUE" } else { "missing the name of a variable" }));
    }
    let mut changes: Vec<(&str, Option<&str>)> = Vec::new();
    for raw in given {
        changes.push(match (set, raw.split_once('=')) {
            (true, Some((key, value))) if !key.is_empty() => (key, Some(value)),
            (true, _) => return Err(CliError::usage(format!("a variable is set as KEY=VALUE, not \"{raw}\""))),
            (false, _) => (raw.as_str(), None),
        });
    }
    let workspace = workspace_id(ctx)?;
    let all = list_entities(ctx, &workspace, "environments")?;
    let mut env = find_one(&all, &wanted, &["name"], "environment", "env")?.clone();
    let mut variables = env.get("variables").and_then(Value::as_array).cloned().unwrap_or_default();
    for (key, value) in changes {
        let at = variables.iter().position(|variable| variable.get("key").and_then(Value::as_str) == Some(key));
        match (value, at) {
            (Some(value), Some(at)) => variables[at]["value"] = json!(value),
            // The id only has to be unlike the others of this environment.
            (Some(value), None) => {
                variables.push(json!({ "id": format!("v{}{key}", variables.len()), "key": key, "value": value }))
            }
            (None, Some(at)) => {
                variables.remove(at);
            }
            (None, None) => {
                return Err(CliError::not_found(format!("{} has no variable \"{key}\"", js_template(env.get("name")))));
            }
        }
    }
    env["variables"] = Value::Array(variables);
    if env.get("workspaceId").is_none_or(Value::is_null) {
        env["workspaceId"] = json!(workspace);
    }
    refused(ctx.request("entity.update", json!({ "kind": "environments", "entity": env }))?)?;
    Ok(ctx.print(&env, || format!("Updated environment {} ({})", js_template(env.get("name")), id_of(&env))))
}

/// `capture mock <entry>` and `capture request <entry>`: save a mock of, or a request like, a captured exchange.
fn capture_save(ctx: &mut Context, kind: &str) -> Result<String> {
    ctx.accept(&[], 1)?;
    let entry = need(ctx.args.positional(2), "the entry's id (see: bifurc capture list)")?;
    let workspace = workspace_id(ctx)?;
    let entry = capture::entry_ids(ctx, &workspace, &[entry])?.remove(0);
    let (command, field) =
        if kind == "mocks" { ("capture.buildMock", "mock") } else { ("capture.buildRequest", "request") };
    let built = ctx.request(command, json!({ "workspaceId": workspace, "id": entry }))?;
    let entity = built.get(field).cloned().unwrap_or(Value::Null);
    let saved =
        created(&ctx.request("entity.create", json!({ "kind": kind, "workspaceId": workspace, "entity": entity }))?);
    Ok(ctx.print(&saved, || format!("Saved {field} {}", id_of(&saved))))
}

/// The id of the request folder `wanted` names: its id, a path (`auth/admin`) or a unique name. Nothing is created.
fn request_folder(ctx: &mut Context, workspace: &str, wanted: &str) -> Result<String> {
    let config = ctx.request("config.get", json!({}))?;
    if config.get("activeWorkspaceId").and_then(Value::as_str).is_some_and(|active| active != workspace) {
        return Err(CliError::msg(
            "a folder is looked up in the active workspace: run bifurc workspaces use <workspace>",
        ));
    }
    let folders: Vec<Value> = config
        .get("requestFolders")
        .and_then(Value::as_array)
        .map(|all| all.iter().filter(|f| f.get("deleted") != Some(&json!(true))).cloned().collect())
        .unwrap_or_default();
    if !wanted.contains('/') {
        return Ok(id_of(find_one(&folders, wanted, &["name"], "folder", "folders")?));
    }
    let mut parent: Option<String> = None;
    for part in wanted.split('/').filter(|part| !part.is_empty()) {
        let hit = folders.iter().find(|f| {
            js_template(f.get("name")).eq_ignore_ascii_case(part)
                && f.get("parentId").and_then(Value::as_str) == parent.as_deref()
        });
        parent = Some(id_of(hit.ok_or_else(|| {
            CliError::not_found(format!("no folder \"{wanted}\" (see: bifurc folders list --tree request)"))
        })?));
    }
    parent.ok_or_else(|| CliError::usage("a folder path needs a name"))
}

/// The failure of one result of a run, `None` when it passed: a request that was not sent, a script that threw or
/// a test that did not pass.
fn failure_of(result: &Value) -> Option<String> {
    let text = |key: &str| result.get(key).and_then(Value::as_str).filter(|text| !text.is_empty());
    if let Some(error) = text("error") {
        return Some(one_line(error));
    }
    if let Some(error) = text("preScriptError").or(text("postScriptError")) {
        return Some(format!("script: {}", one_line(error)));
    }
    let tests = result.get("tests").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let failed = tests.iter().find(|test| test.get("passed") != Some(&json!(true)))?;
    let error = failed.get("error").and_then(Value::as_str).map(|e| format!(": {}", one_line(e))).unwrap_or_default();
    Some(format!("test {}{error}", js_template(failed.get("name"))))
}

/// `runner run <folder> [--bail] [--var k=v] [--env e] [--fresh-session] [--verbose]`: send every request of a
/// folder in order and print a line for each, then the totals. Exit 1 when a request or a test failed.
fn runner_run(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["bail", "var", "env", "fresh-session", "verbose", "max-body"], 1)?;
    let wanted =
        need(ctx.args.positional(2), "the folder's id, path or name (see: bifurc folders list --tree request)")?;
    let max_body = max_body(ctx.args)?;
    let workspace = workspace_id(ctx)?;
    let mut run = json!({ "workspaceId": workspace, "folderId": request_folder(ctx, &workspace, &wanted)? });
    crate::requests::add_scope(ctx, &workspace, &mut run)?;
    for (flag, key) in [("bail", "bail"), ("fresh-session", "freshJar")] {
        if ctx.args.has(flag) {
            run[key] = json!(true);
        }
    }
    let status_request = json!({ "workspaceId": workspace, "folderId": run["folderId"] });
    refused(ctx.request("runner.start", run)?)?;
    let finished = loop {
        std::thread::sleep(RUN_POLL);
        let status = ctx.request("runner.status", status_request.clone())?;
        if status.get("running") != Some(&json!(true)) {
            break status;
        }
    };
    if let Some(error) = finished.get("error").and_then(Value::as_str) {
        return Err(CliError::msg(format!("the run stopped: {error}")));
    }
    let report = finished.get("report").cloned().unwrap_or_else(|| finished.clone());
    let results = report.get("results").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let steps: Vec<Step> = results
        .iter()
        .map(|result| Step {
            label: format!("{} {}", js_template(result.get("method")), path_of(&js_template(result.get("url")))),
            status: result.get("status").filter(|status| !status.is_null()).map(|status| js_template(Some(status))),
            ms: result.get("responseTime").and_then(Value::as_u64),
            failure: failure_of(result),
            body: result.get("responseBody").and_then(Value::as_str).map(str::to_string),
        })
        .collect();
    let text = ctx.print(&report, || {
        let failed = steps.iter().filter(|step| step.failure.is_some()).count();
        let all = finished.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
        let ran =
            if steps.len() < all { format!("{} of {all} run", steps.len()) } else { format!("{} run", steps.len()) };
        let took = match (
            report.get("startedAt").and_then(Value::as_i64),
            report.get("completedAt").and_then(Value::as_i64),
        ) {
            (Some(start), Some(end)) => format!(" · {}ms", (end - start).max(0)),
            _ => String::new(),
        };
        let mut lines = lines(&steps, ctx.args.has("verbose"), max_body);
        lines.push(format!("{ran} · {} passed · {failed} failed{took}", steps.len() - failed));
        lines.join(
            "
",
        )
    });
    if results.iter().all(|result| failure_of(result).is_none()) { Ok(text) } else { Err(CliError::Failed(text)) }
}

/// `settings show`: the engine's ports and switches.
fn settings_show(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let config = ctx.request("config.get", json!({}))?;
    let settings: Map<String, Value> =
        SETTINGS.iter().filter_map(|key| config.get(*key).map(|value| (key.to_string(), value.clone()))).collect();
    let mut settings = settings;
    // The capture limit is not in the snapshot: the buffer reports it.
    if let Ok(workspace) = workspace_id(ctx)
        && let Ok(buffer) = ctx.request("capture.list", json!({ "workspaceId": workspace, "summary": true, "last": 0 }))
    {
        settings.insert("captureLimit".into(), buffer["limit"].clone());
    }
    let settings = Value::Object(settings);
    Ok(ctx.print(&settings, || json(&settings)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run as with_engine};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        with_engine(argv, answers, |ctx| run(ctx, argv[0], argv[1]).expect("one of this module's verbs"))
    }

    fn environments() -> Vec<Answer> {
        let dev = json!({"id": "e1", "name": "Dev", "workspaceId": "w1", "variables": [
            {"id": "a", "key": "HOST", "value": "http://old"}, {"id": "b", "key": "GONE", "value": "x"},
        ]});
        vec![
            ("config.get", Ok(config())),
            ("entity.list", Ok(json!({"entities": [dev]}))),
            ("entity.update", Ok(json!({"ok": true}))),
        ]
    }

    #[test]
    fn env_set_changes_or_adds_a_variable_and_unset_removes_one() {
        let (out, calls) = go(&["env", "set", "dev", "HOST=http://new", "TOKEN=a=b"], environments());
        assert_eq!(out.unwrap(), "Updated environment Dev (e1)");
        assert_eq!(
            payloads(&calls, "entity.update")[0]["entity"]["variables"],
            json!([
                {"id": "a", "key": "HOST", "value": "http://new"},
                {"id": "b", "key": "GONE", "value": "x"},
                {"id": "v2TOKEN", "key": "TOKEN", "value": "a=b"},
            ])
        );
        let (_, calls) = go(&["env", "unset", "e1", "GONE"], environments());
        assert_eq!(
            payloads(&calls, "entity.update")[0]["entity"]["variables"],
            json!([{"id": "a", "key": "HOST", "value": "http://old"}])
        );
        for (argv, expected) in [
            (vec!["env", "set", "Dev"], "missing KEY=VALUE"),
            (vec!["env", "set", "Dev", "HOST"], "a variable is set as KEY=VALUE, not \"HOST\""),
            (vec!["env", "unset", "Dev", "NOPE"], "Dev has no variable \"NOPE\""),
        ] {
            let (out, calls) = go(&argv, environments());
            assert_eq!(out.unwrap_err().line(), format!("bifurc: {expected}"));
            assert!(payloads(&calls, "entity.update").is_empty());
        }
    }

    #[test]
    fn a_captured_exchange_is_saved_as_a_mock_or_a_request() {
        let answers = vec![
            ("config.get", Ok(config())),
            ("capture.list", Ok(json!({"entries": [{"id": "xxxxc1"}, {"id": "xxxxc2"}]}))),
            ("capture.buildMock", Ok(json!({"mock": {"urlPattern": "http://a/"}}))),
            ("entity.create", Ok(json!({"id": "m9", "entity": {"id": "m9"}}))),
        ];
        let (out, calls) = go(&["capture", "mock", "#c1"], answers);
        assert_eq!(out.unwrap(), "Saved mock m9");
        assert_eq!(calls[2].1, json!({"workspaceId": "w1", "id": "xxxxc1"}));
        assert_eq!(calls[3].1, json!({"kind": "mocks", "workspaceId": "w1", "entity": {"urlPattern": "http://a/"}}));
    }

    fn row(name: &str, method: &str, url: &str, status: u64, ms: u64, tests: Value, body: &str) -> Value {
        json!({"requestName": name, "method": method, "url": url, "status": status, "responseTime": ms, "error": null,
               "tests": tests, "responseBody": body})
    }

    /// The runner's status of a finished run of `results`, out of `total` requests.
    fn finished(results: Vec<Value>, total: u64) -> Value {
        json!({"running": false, "total": total, "report": {"startedAt": 1000, "completedAt": 1812,
            "totalRequests": results.len(), "results": results}})
    }

    fn runner(status: Value) -> Vec<Answer> {
        let mut snapshot = config();
        snapshot["requestFolders"] = json!([
            {"id": "f1", "name": "auth", "parentId": null},
            {"id": "f2", "name": "admin", "parentId": "f1"},
            {"id": "f3", "name": "admin", "parentId": null},
            {"id": "f4", "name": "gone", "parentId": null, "deleted": true},
        ]);
        vec![
            ("config.get", Ok(snapshot)),
            ("entity.list:environments", Ok(json!({"entities": [{"id": "e1", "name": "Dev"}]}))),
            ("runner.start", Ok(json!({"ok": true, "total": 3}))),
            ("runner.status", Ok(status)),
        ]
    }

    #[test]
    fn a_folder_run_prints_a_line_per_request_and_the_totals() {
        let pass = json!([{"name": "ok", "passed": true}]);
        let status = finished(
            vec![
                row("Login", "POST", "http://127.0.0.1:80/login", 200, 88, pass.clone(), "{\"token\":1}"),
                row("Carts", "POST", "{{baseUrl}}/carts", 201, 41, pass, "{}"),
            ],
            2,
        );
        let (out, calls) = go(&["runner", "run", "f1"], runner(status));
        assert_eq!(
            out.unwrap(),
            "1 POST /login  200 · 88ms  ✓\n2 POST /carts  201 · 41ms  ✓\n2 run · 2 passed · 0 failed · 812ms"
        );
        assert_eq!(payloads(&calls, "runner.start"), [json!({"workspaceId": "w1", "folderId": "f1"})]);
    }

    #[test]
    fn a_failed_test_shows_its_reason_and_body_and_fails_the_command() {
        let fail = json!([{"name": "created", "passed": false, "error": "expected 422 to equal 201"}]);
        let mut sent = row("Cart", "POST", "http://x/carts/c_91", 422, 37, fail, "{\"error\":\"qty must be ≤ 1\"}");
        let mut unsent = row("Down", "GET", "http://x/down", 0, 0, json!([]), "");
        unsent["status"] = Value::Null;
        unsent["error"] = json!("connection refused");
        sent["preScriptError"] = Value::Null;
        let (out, _) = go(&["runner", "run", "f1"], runner(finished(vec![sent, unsent], 2)));
        let CliError::Failed(text) = out.unwrap_err() else { panic!("a failed test is a failed command") };
        assert_eq!(
            text,
            "1 POST /carts/c_91  422 · 37ms  ✗ test created: expected 422 to equal 201\n\
             \x20 → {\"error\":\"qty must be ≤ 1\"}\n\
             2 GET /down         - · 0ms     ✗ connection refused\n\
             2 run · 0 passed · 2 failed · 812ms"
        );
        // A script that threw fails the step too.
        let mut threw = row("S", "GET", "http://x/s", 200, 5, json!([]), "");
        threw["postScriptError"] = json!("x is not defined");
        let (out, _) = go(&["runner", "run", "f1", "--verbose"], runner(finished(vec![threw], 1)));
        assert!(
            matches!(out, Err(CliError::Failed(text)) if text.starts_with("1 GET /s  200 · 5ms  ✗ script: x is not"))
        );
    }

    #[test]
    fn flags_ride_along_and_a_bailed_run_says_how_far_it_got() {
        let fail = json!([{"name": "t", "passed": false}]);
        let status = finished(vec![row("A", "GET", "http://x/a", 500, 9, fail, "")], 3);
        let argv = ["runner", "run", "auth/admin", "--bail", "--var", "a=b", "--env", "dev", "--fresh-session"];
        let (out, calls) = go(&argv, runner(status));
        let CliError::Failed(text) = out.unwrap_err() else { panic!("a failed test is a failed command") };
        assert!(text.ends_with("1 of 3 run · 0 passed · 1 failed · 812ms"), "{text}");
        assert_eq!(
            payloads(&calls, "runner.start"),
            [json!({"workspaceId": "w1", "folderId": "f2", "bail": true, "variables": {"a": "b"},
                    "environmentId": "e1", "freshJar": true})]
        );
        assert_eq!(payloads(&calls, "runner.status"), [json!({"workspaceId": "w1", "folderId": "f2"})]);
    }

    #[test]
    fn a_folder_is_found_by_id_path_or_unique_name_and_never_made() {
        let status = || finished(vec![row("A", "GET", "http://x/a", 200, 1, json!([]), "")], 1);
        let folder = |wanted: &str| {
            let (out, calls) = go(&["runner", "run", wanted], runner(status()));
            (out, payloads(&calls, "runner.start").first().map(|p| p["folderId"].clone()))
        };
        assert_eq!(folder("f3").1, Some(json!("f3")));
        assert_eq!(folder("AUTH/admin").1, Some(json!("f2")));
        assert_eq!(folder("/admin/").1, Some(json!("f3")));
        assert_eq!(folder("auth").1, Some(json!("f1")));
        let (out, started) = folder("admin");
        assert!(out.unwrap_err().line().contains("2 folders are named \"admin\""));
        assert_eq!(started, None);
        let (out, started) = folder("auth/nope");
        assert_eq!(out.unwrap_err().code(), 3);
        assert_eq!(started, None);
        assert_eq!(folder("gone").0.unwrap_err().code(), 3);
    }

    #[test]
    fn json_is_the_engines_report_and_a_busy_folder_is_refused() {
        let status = finished(vec![row("A", "GET", "http://x/a", 200, 1, json!([]), "")], 1);
        let (out, _) = go(&["runner", "run", "f1", "--json"], runner(status.clone()));
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap(), status["report"]);
        let mut busy = runner(status);
        busy.insert(0, ("runner.start", Ok(json!({"ok": false, "error": "already running"}))));
        assert_eq!(go(&["runner", "run", "f1"], busy).0.unwrap_err().line(), "bifurc: already running");
    }

    #[test]
    fn folders_and_settings_are_read_from_the_snapshot() {
        let snapshot = || {
            let mut snapshot = config();
            snapshot["mockFolders"] = json!([{"id": "f1", "name": "Default", "parentId": null}]);
            snapshot["port"] = json!(80);
            snapshot["tlsEnabled"] = json!(false);
            vec![("config.get", Ok(snapshot))]
        };
        let (out, _) = go(&["folders", "list", "--tree", "mock"], snapshot());
        assert_eq!(out.unwrap(), "ID  NAME     PARENTID  DELETED\nf1  Default");
        let (out, calls) = go(&["folders", "list", "--tree", "shrub"], snapshot());
        assert!(out.unwrap_err().line().starts_with("bifurc: --tree must be one of"));
        assert!(calls.is_empty());
        let (out, _) = go(&["settings", "show", "--json"], snapshot());
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap(), json!({"port": 80, "tlsEnabled": false}));
    }

    fn jar() -> Value {
        json!({"clearOnRestart": true, "domains": [
            {"domain": "a.com", "cookies": [
                {"name": "sid", "value": "0123456789abcdef0123456789abcdef", "path": "/", "expires": 1792332120000i64,
                 "secure": true, "httpOnly": true}]},
            {"domain": "b.com", "cookies": [
                {"name": "t", "value": "x", "path": "/", "expires": null, "secure": false, "httpOnly": false}]}]})
    }

    #[test]
    fn cookies_list_masks_values_and_filters_by_domain() {
        let go = |extra: &[&str]| {
            let argv = [&["cookies", "list"], extra].concat();
            go(&argv, vec![("config.get", Ok(config())), ("cookies.list", Ok(jar()))]).0.unwrap()
        };
        assert_eq!(
            go(&[]),
            "a.com  sid  ••••(32)  expires 2026-10-18 14:02  HttpOnly Secure\nb.com  t    ••••(1)   session"
        );
        assert_eq!(go(&["--domain", "B.com"]), "b.com  t  ••••(1)  session");
        let masked: Value = serde_json::from_str(&go(&["--json", "--domain", "a.com"])).unwrap();
        assert_eq!(masked["domains"].as_array().unwrap().len(), 1);
        assert_eq!(masked["domains"][0]["cookies"][0]["value"], "••••(32)");
        // Not a terminal here, so --reveal is ignored.
        assert!(!go(&["--reveal"]).contains("0123456789"));
    }

    #[test]
    fn env_show_masks_secret_looking_names_but_not_vault_references() {
        let merged = json!({"id": "merged", "variables": [
            {"key": "baseUrl", "value": "http://x", "enabled": true},
            {"key": "API_KEY", "value": "abcdef", "enabled": true},
            {"key": "dbPassword", "value": "{{vault.db}}", "enabled": true}]});
        let answers = || vec![("config.get", Ok(config())), ("env.getMerged", Ok(merged.clone()))];
        let (out, _) = go(&["env", "show"], answers());
        assert_eq!(
            out.unwrap(),
            "baseUrl     http://x
API_KEY     ••••(6)
dbPassword  {{vault.db}}"
        );
        let (out, calls) = go(&["env", "show", "--json"], answers());
        let shown: Value = serde_json::from_str(&out.unwrap()).unwrap();
        let values: Vec<&str> =
            shown["variables"].as_array().unwrap().iter().map(|v| v["value"].as_str().unwrap()).collect();
        assert_eq!(values, ["http://x", "••••(6)", "{{vault.db}}"]);
        assert_eq!(calls[1].1, json!({"workspaceId": "w1"}));
    }
}
