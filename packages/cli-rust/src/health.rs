//! `health`: the workspace's health checks. A check is a name and a URL; running one asks the engine to
//! resolve the URL's variables and call it.
//!
//! The engine keeps the checks as one list (`healthbar.getServices` / `healthbar.saveServices`), so a create or
//! an update here reads the list, changes one check and saves it, exactly as the desktop app does.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::{
    commands::{Context, find_one, id_of, need, switch, unknown_verb, with_field, workspace_id},
    error::{CliError, Result},
    format::{js_template, json, table, truthy},
};

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn checks(ctx: &mut Context, workspace: &str) -> Result<Vec<Value>> {
    let saved = ctx.request("healthbar.getServices", json!({ "workspaceId": workspace }))?;
    Ok(saved.as_array().cloned().unwrap_or_default())
}

fn save(ctx: &mut Context, workspace: &str, checks: &[Value]) -> Result<()> {
    ctx.request("healthbar.saveServices", json!({ "workspaceId": workspace, "services": checks }))?;
    Ok(())
}

/// The check with the columns a table shows: whether the desktop app re-checks it on its own.
fn row(check: &Value) -> Value {
    with_field(check, "auto", json!(if check.get("autoRefreshEnabled").is_some_and(truthy) { "auto" } else { "" }))
}

/// Run one check and return its row: the check, plus what the engine found.
fn run_check(ctx: &mut Context, workspace: &str, check: &Value) -> Result<Value> {
    let url = js_template(check.get("url"));
    // Variables are the engine's to resolve; the check then calls what they resolved to.
    let resolved = ctx.request("env.resolveText", json!({ "workspaceId": workspace, "text": url }))?;
    let resolved = resolved.get("text").and_then(Value::as_str).unwrap_or(&url).to_string();
    let result = ctx.request("healthbar.checkUrl", json!({ "url": resolved, "manual": true }))?;
    let healthy = result.get("ok").is_some_and(truthy);
    let mut out = json!({
        "id": check.get("id"),
        "name": check.get("name"),
        "url": resolved,
        "healthy": healthy,
        "result": if healthy { "healthy" } else { "failing" },
        "status": result.get("statusCode"),
        "ms": result.get("durationMs"),
        "error": result.get("error"),
    });
    if ctx.json {
        out["body"] = result.get("body").cloned().unwrap_or(Value::Null);
    }
    Ok(out)
}

/// `health list|show|create|update|run`.
pub fn health(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let args = ctx.args;
    match sub.unwrap_or("list") {
        "list" => {
            ctx.accept(&[], 0)?;
            let workspace = workspace_id(ctx)?;
            let all = checks(ctx, &workspace)?;
            let rows: Vec<Value> = all.iter().map(row).collect();
            Ok(ctx.print(&Value::Array(all), || table(&rows, &["id", "name", "url", "auto"])))
        }
        "show" => {
            ctx.accept(&[], 1)?;
            let wanted = need(args.positional(2), "health check id or name")?;
            let workspace = workspace_id(ctx)?;
            let all = checks(ctx, &workspace)?;
            let check = find_one(&all, &wanted, &["name"], "health check", "health")?;
            Ok(ctx.print(check, || json(check)))
        }
        "create" | "add" => {
            ctx.accept(&["auto"], 2)?;
            let name = need(args.positional(2), "name, e.g. \"Users API\"")?;
            let url = need(args.positional(3), "URL, e.g. {{HOST}}/health")?;
            let workspace = workspace_id(ctx)?;
            let mut all = checks(ctx, &workspace)?;
            let now = now_ms();
            // The id is the time it was made, as the desktop app's are; a later one when that is taken.
            let id = (now..)
                .map(|stamp| format!("hb{stamp:x}"))
                .find(|id| !all.iter().any(|check| check.get("id").and_then(Value::as_str) == Some(id)))
                .expect("some id is free");
            let check = json!({
                "id": id, "name": name, "url": url, "autoRefreshEnabled": args.has("auto"), "createdAt": now,
            });
            all.push(check.clone());
            save(ctx, &workspace, &all)?;
            Ok(ctx.print(&check, || format!("Saved health check {id}")))
        }
        "update" => {
            ctx.accept(&["name", "url", "auto", "no-auto"], 1)?;
            let auto = switch(args, "auto", "no-auto")?;
            if args.flag("name").is_none() && args.flag("url").is_none() && auto.is_none() {
                return Err(CliError::msg("nothing to change: give at least one of --name, --url, --auto, --no-auto"));
            }
            let wanted = need(args.positional(2), "health check id or name")?;
            let workspace = workspace_id(ctx)?;
            let mut all = checks(ctx, &workspace)?;
            let id = id_of(find_one(&all, &wanted, &["name"], "health check", "health")?);
            let check = all.iter_mut().find(|check| id_of(check) == id).expect("found above");
            for (key, flag) in [("name", "name"), ("url", "url")] {
                if let Some(value) = args.flag(flag) {
                    check[key] = json!(value);
                }
            }
            if let Some(auto) = auto {
                check["autoRefreshEnabled"] = json!(auto);
            }
            let check = check.clone();
            save(ctx, &workspace, &all)?;
            Ok(ctx.print(&check, || format!("Updated health check {id}")))
        }
        "run" => {
            ctx.accept(&[], 1)?;
            let workspace = workspace_id(ctx)?;
            let all = checks(ctx, &workspace)?;
            let wanted: Vec<Value> = match args.positional(2) {
                Some(wanted) => vec![find_one(&all, wanted, &["name"], "health check", "health")?.clone()],
                None => all,
            };
            let mut rows = Vec::new();
            for check in &wanted {
                rows.push(run_check(ctx, &workspace, check)?);
            }
            let report =
                ctx.print(&Value::Array(rows.clone()), || table(&rows, &["name", "result", "status", "ms", "url", "error"]));
            if rows.iter().all(|row| row.get("healthy").is_some_and(truthy)) {
                Ok(report)
            } else {
                Err(CliError::Failed(report))
            }
        }
        other => Err(unknown_verb("health", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run, sub};

    fn saved() -> Value {
        json!([
            {"id": "hb1", "name": "Users API", "url": "{{HOST}}/health", "autoRefreshEnabled": true, "createdAt": 1},
            {"id": "hb2", "name": "Billing", "url": "http://localhost:9/health", "autoRefreshEnabled": false, "createdAt": 2},
        ])
    }

    fn engine(probe: Value) -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("healthbar.getServices", Ok(saved())),
            ("healthbar.saveServices", Ok(json!({"ok": true, "added": 1}))),
            ("env.resolveText", Ok(json!({"text": "http://localhost:3000/health"}))),
            ("healthbar.checkUrl", Ok(probe)),
        ]
    }

    fn up() -> Value {
        json!({"ok": true, "statusCode": 200, "body": "ok", "headers": {}, "error": null, "durationMs": 4})
    }

    #[test]
    fn list_is_a_table_and_the_saved_list_as_json() {
        let argv = ["health"];
        let (out, calls) = run(&argv, engine(up()), |c| health(c, None));
        assert_eq!(
            out.unwrap(),
            "ID   NAME       URL                        AUTO\n\
             hb1  Users API  {{HOST}}/health            auto\n\
             hb2  Billing    http://localhost:9/health"
        );
        assert_eq!(calls[1], ("healthbar.getServices".to_string(), json!({"workspaceId": "w1"})));

        let argv = ["health", "list", "--json"];
        let (out, _) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
        assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap(), saved());
    }

    #[test]
    fn show_takes_an_id_or_a_name() {
        for wanted in ["hb2", "billing"] {
            let argv = ["health", "show", wanted];
            let (out, _) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
            assert_eq!(serde_json::from_str::<Value>(&out.unwrap()).unwrap()["id"], "hb2");
        }
        let argv = ["health", "show", "nope"];
        let (out, _) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("no health check \"nope\" (see: bifurc health list)"));
    }

    #[test]
    fn create_adds_one_check_to_the_saved_list() {
        let argv = ["health", "create", "Search", "{{HOST}}/search/health", "--auto"];
        let (out, calls) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
        let out = out.unwrap();
        let id = out.strip_prefix("Saved health check ").unwrap();
        assert!(id.starts_with("hb"));
        let sent = &payloads(&calls, "healthbar.saveServices")[0];
        assert_eq!(sent["workspaceId"], "w1");
        let services = sent["services"].as_array().unwrap();
        assert_eq!(services[..2], saved().as_array().unwrap()[..]);
        assert_eq!(services[2]["id"], id);
        assert_eq!(services[2]["name"], "Search");
        assert_eq!(services[2]["url"], "{{HOST}}/search/health");
        assert_eq!(services[2]["autoRefreshEnabled"], true);
        assert!(services[2]["createdAt"].as_u64().unwrap() > 0);

        let argv = ["health", "create", "Search"];
        let (out, calls) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("missing URL, e.g. {{HOST}}/health"));
        assert!(calls.is_empty());
    }

    #[test]
    fn update_changes_one_check_and_leaves_the_others() {
        let argv = ["health", "update", "Billing", "--url", "http://localhost:10/health", "--auto"];
        let (out, calls) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated health check hb2");
        let services = payloads(&calls, "healthbar.saveServices")[0]["services"].clone();
        assert_eq!(services[0], saved()[0]);
        assert_eq!(
            services[1],
            json!({"id": "hb2", "name": "Billing", "url": "http://localhost:10/health", "autoRefreshEnabled": true,
                   "createdAt": 2})
        );

        let argv = ["health", "update", "hb2"];
        let (out, calls) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::msg("nothing to change: give at least one of --name, --url, --auto, --no-auto")
        );
        assert!(calls.is_empty());
    }

    #[test]
    fn run_resolves_the_url_and_checks_it_as_asked_for_by_hand() {
        let argv = ["health", "run", "users api"];
        let (out, calls) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
        assert_eq!(
            out.unwrap(),
            "NAME       RESULT   STATUS  MS  URL                           ERROR\n\
             Users API  healthy  200     4   http://localhost:3000/health"
        );
        assert_eq!(payloads(&calls, "env.resolveText"), [json!({"workspaceId": "w1", "text": "{{HOST}}/health"})]);
        assert_eq!(
            payloads(&calls, "healthbar.checkUrl"),
            [json!({"url": "http://localhost:3000/health", "manual": true})]
        );
    }

    #[test]
    fn run_without_a_name_checks_all_and_a_check_that_does_not_answer_fails_the_command() {
        let down = json!({"ok": false, "statusCode": null, "body": null, "headers": null,
                          "error": "connection refused", "durationMs": 2});
        let argv = ["health", "run"];
        let (out, calls) = run(&argv, engine(down), |c| health(c, sub(&argv)));
        let CliError::Failed(report) = out.unwrap_err() else { panic!("a failing check is a failed command") };
        assert_eq!(report.lines().count(), 3);
        assert!(report.lines().nth(1).unwrap().contains("failing"));
        assert!(report.contains("connection refused"));
        assert_eq!(payloads(&calls, "healthbar.checkUrl").len(), 2);

        let argv = ["health", "run", "--json"];
        let (out, _) = run(&argv, engine(up()), |c| health(c, sub(&argv)));
        let rows: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!(rows[0]["healthy"], true);
        assert_eq!(rows[0]["body"], "ok");
    }

    #[test]
    fn nothing_is_removed_from_the_command_line() {
        let argv = ["health", "delete", "hb1"];
        let (out, calls) = run(&argv, vec![], |c| health(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::msg("unknown: health delete; deleting is done in the desktop app (see: bifurc health --help)")
        );
        assert!(calls.is_empty());
    }
}
