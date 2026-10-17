//! The verbs of the groups in `direct.rs` that take more than one engine command: reading a tree's folders,
//! changing one variable of an environment, saving what was captured, running a folder to its end, and the
//! engine's settings.

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::{
    commands::{Context, created, find_one, id_of, list_entities, need, refused, workspace_id},
    error::{CliError, Result},
    format::{js_template, json, table},
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
        ("env", "set") => env_variables(ctx, true),
        ("env", "unset") => env_variables(ctx, false),
        ("capture", "mock") => capture_save(ctx, "mocks"),
        ("capture", "request") => capture_save(ctx, "requests"),
        ("runner", "run") => runner_run(ctx),
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

/// `env set <env> KEY=VALUE...` and `env unset <env> KEY...`: change some variables and leave the others.
fn env_variables(ctx: &mut Context, set: bool) -> Result<String> {
    ctx.accept(&[], 1000)?;
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
    let (command, field) =
        if kind == "mocks" { ("capture.buildMock", "mock") } else { ("capture.buildRequest", "request") };
    let built = ctx.request(command, json!({ "workspaceId": workspace, "id": entry }))?;
    let entity = built.get(field).cloned().unwrap_or(Value::Null);
    let saved =
        created(&ctx.request("entity.create", json!({ "kind": kind, "workspaceId": workspace, "entity": entity }))?);
    Ok(ctx.print(&saved, || format!("Saved {field} {}", id_of(&saved))))
}

/// `runner run <folder>`: send every request of a folder in order and wait for the report.
fn runner_run(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 1)?;
    let folder = need(ctx.args.positional(2), "the folder's id (see: bifurc folders list --tree request)")?;
    let workspace = workspace_id(ctx)?;
    let run = json!({ "workspaceId": workspace, "folderId": folder });
    refused(ctx.request("runner.start", run.clone())?)?;
    let finished = loop {
        std::thread::sleep(RUN_POLL);
        let status = ctx.request("runner.status", run.clone())?;
        if status.get("running") != Some(&json!(true)) {
            break status;
        }
    };
    if let Some(error) = finished.get("error").and_then(Value::as_str) {
        return Err(CliError::msg(format!("the run stopped: {error}")));
    }
    let report = finished.get("report").cloned().unwrap_or(finished);
    let count = |key: &str| report.get(key).and_then(Value::as_u64).unwrap_or(0);
    let results = report.get("results").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let errors = results.iter().filter(|result| result.get("error").is_some_and(|error| !error.is_null())).count();
    let text = ctx.print(&report, || {
        let rows: Vec<Value> = results
            .iter()
            .map(|result| {
                let tests = result.get("tests").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
                let failed = tests.iter().filter(|test| test.get("passed") != Some(&json!(true))).count();
                json!({
                    "request": result.get("requestName"), "status": result.get("status"),
                    "ms": result.get("responseTime"), "tests": tests.len(), "failed": failed,
                    "error": result.get("error").filter(|error| !error.is_null()).map_or(String::new(), |e| js_template(Some(e))),
                })
            })
            .collect();
        format!(
            "{}\n{} requests, {} tests passed, {} failed",
            table(&rows, &["request", "status", "ms", "tests", "failed", "error"]),
            count("totalRequests"),
            count("passedTests"),
            count("failedTests"),
        )
    });
    if count("failedTests") == 0 && errors == 0 { Ok(text) } else { Err(CliError::Failed(text)) }
}

/// `settings show`: the engine's ports and switches.
fn settings_show(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let config = ctx.request("config.get", json!({}))?;
    let settings: Map<String, Value> =
        SETTINGS.iter().filter_map(|key| config.get(*key).map(|value| (key.to_string(), value.clone()))).collect();
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
            ("capture.buildMock", Ok(json!({"mock": {"urlPattern": "http://a/"}}))),
            ("entity.create", Ok(json!({"id": "m9", "entity": {"id": "m9"}}))),
        ];
        let (out, calls) = go(&["capture", "mock", "c1"], answers);
        assert_eq!(out.unwrap(), "Saved mock m9");
        assert_eq!(calls[1].1, json!({"workspaceId": "w1", "id": "c1"}));
        assert_eq!(calls[2].1, json!({"kind": "mocks", "workspaceId": "w1", "entity": {"urlPattern": "http://a/"}}));
    }

    #[test]
    fn a_folder_run_waits_for_its_report_and_fails_when_a_test_does() {
        let report = |failed: u64| {
            json!({"running": false, "report": {
                "totalRequests": 1, "passedTests": 1 - failed, "failedTests": failed,
                "results": [{"requestName": "Ping", "status": 200, "responseTime": 7, "error": null,
                             "tests": [{"name": "ok", "passed": failed == 0}]}],
            }})
        };
        let answers = |failed| {
            vec![
                ("config.get", Ok(config())),
                ("runner.start", Ok(json!({"ok": true, "total": 1}))),
                ("runner.status", Ok(report(failed))),
            ]
        };
        let (out, _) = go(&["runner", "run", "f1"], answers(0));
        assert_eq!(
            out.unwrap(),
            "REQUEST  STATUS  MS  TESTS  FAILED  ERROR\nPing     200     7   1      0\n1 requests, 1 tests passed, 0 failed"
        );
        let (out, _) = go(&["runner", "run", "f1"], answers(1));
        assert!(matches!(out, Err(CliError::Failed(text)) if text.ends_with("0 tests passed, 1 failed")));
        let busy =
            vec![("config.get", Ok(config())), ("runner.start", Ok(json!({"ok": false, "error": "already running"})))];
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
}
