//! The collection runner: run a folder's requests and keep its reports and run configuration under
//! `<workspace>/requests/.runs/<folderId>/`.

pub mod report_html;
pub mod runs;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use self::{
    report_html::{iso_string, render_runner_html},
    runs::{cancel_run, get_run, start_run},
};
use crate::{
    kernel::CommandRegistry,
    utils::json::{read_json, str_of, write_json_pretty},
    workspace::{blob::publish_artifact, fs::ws_dir},
};

fn runs_dir(ws_id: &str) -> PathBuf {
    ws_dir(ws_id).join("requests").join(".runs")
}

/// `String(n)` for a JSON number: integers print without a fraction.
fn number_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => match number.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            _ => number.to_string(),
        },
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "undefined".to_string(),
    }
}

pub(crate) fn save_report(params: &Value) -> std::io::Result<()> {
    let report = &params["report"];
    let run_dir = runs_dir(str_of(params, "workspaceId"))
        .join(str_of(report, "folderId"))
        .join(number_text(report.get("startedAt")));
    std::fs::create_dir_all(&run_dir)?;
    std::fs::write(run_dir.join("report.json"), serde_json::to_string_pretty(report)?)?;
    std::fs::write(run_dir.join("report.html"), render_runner_html(report))?;
    Ok(())
}

fn history_entry(runs_dir: &Path, name: &str) -> Option<Value> {
    let data = read_json(&runs_dir.join(name).join("report.json"))?;
    let count = |key: &str| data.get(key).filter(|v| !v.is_null()).cloned().unwrap_or(json!(0));
    let timestamp = name.trim().parse::<f64>().ok().filter(|t| t.is_finite()).map(|t| json!(t as i64));
    Some(json!({
        "timestamp": timestamp.unwrap_or(Value::Null),
        "summary": { "total": count("totalTests"), "passed": count("passedTests"), "failed": count("failedTests") },
    }))
}

fn history(ws_id: &str, folder_id: &str) -> Vec<Value> {
    let dir = runs_dir(ws_id).join(folder_id);
    let Ok(listing) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<String> = listing
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    let mut entries: Vec<Value> = names.iter().filter_map(|name| history_entry(&dir, name)).collect();
    let stamp = |entry: &Value| entry["timestamp"].as_i64().unwrap_or(i64::MIN);
    entries.sort_by_key(|entry| std::cmp::Reverse(stamp(entry)));
    entries
}

fn folder_ids(ws_id: &str) -> Vec<Value> {
    let dir = runs_dir(ws_id);
    let Ok(listing) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<String> = listing
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir() && entry.path().join("runner.json").exists())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names.into_iter().map(Value::from).collect()
}

fn export_report(params: &Value) -> crate::protocol::Result<Value> {
    let report = &params["report"];
    let format = str_of(params, "format");
    let folder_name = match report.get("folderName") {
        None | Some(Value::Null) => "collection".to_string(),
        Some(Value::String(name)) => name.clone(),
        Some(other) => other.to_string(),
    };
    let started = report.get("startedAt").and_then(Value::as_f64).unwrap_or(0.0) as i64;
    let stamp = iso_string(started).replace([':', '.'], "-");
    let json_format = format == "json";
    let content = if json_format { serde_json::to_string_pretty(report)? } else { render_runner_html(report) };
    let mime = if json_format { "application/json" } else { "text/html" };
    publish_artifact(
        &content,
        &format!("{folder_name}-report-{stamp}.{format}"),
        mime,
        params.get("workspaceId").and_then(Value::as_str),
    )
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("runner.saveReport", |params, _| {
        Ok(match save_report(&params) {
            Ok(()) => json!({ "ok": true }),
            Err(error) => json!({ "ok": false, "error": error.to_string() }),
        })
    });

    registry.register_sync("runner.exportReport", |params, _| export_report(&params));

    registry.register_sync("runner.saveConfig", |params, _| {
        let file = runs_dir(str_of(&params, "workspaceId")).join(str_of(&params, "folderId")).join("runner.json");
        Ok(match write_json_pretty(&file, &params["config"]) {
            Ok(()) => json!({ "ok": true }),
            Err(_) => json!({ "ok": false }),
        })
    });

    registry.register_sync("runner.loadConfig", |params, _| {
        let file = runs_dir(str_of(&params, "workspaceId")).join(str_of(&params, "folderId")).join("runner.json");
        Ok(read_json(&file).unwrap_or(Value::Null))
    });

    registry.register_sync("runner.getHistory", |params, _| {
        Ok(Value::Array(history(str_of(&params, "workspaceId"), str_of(&params, "folderId"))))
    });

    registry.register_sync("runner.listFolderIds", |params, _| {
        Ok(Value::Array(folder_ids(str_of(&params, "workspaceId"))))
    });

    registry.register("runner.start", |params: Value, ctx| async move {
        start_run(&ctx, str_of(&params, "workspaceId"), str_of(&params, "folderId")).await
    });

    registry.register_sync("runner.cancel", |params, _| {
        Ok(cancel_run(str_of(&params, "workspaceId"), str_of(&params, "folderId")))
    });

    registry.register_sync("runner.status", |params, _| {
        Ok(get_run(str_of(&params, "workspaceId"), str_of(&params, "folderId")).unwrap_or(Value::Null))
    });
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use super::*;
    use crate::{
        kernel::{CommandContext, bus},
        rest::test_server,
        testing::scratch_data_root,
        workspace::fs::write_entity,
    };

    const WS: &str = "ws1";

    fn context() -> (Arc<CommandRegistry>, CommandContext) {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        crate::rest::register(&mut registry);
        crate::workspace::entities::register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        (registry, ctx)
    }

    async fn call(action: &str, payload: Value) -> crate::protocol::Result<Value> {
        let (registry, ctx) = context();
        registry.invoke(action, payload, ctx).await
    }

    fn report(started: i64) -> Value {
        json!({
            "folderId": "f1", "folderName": "F", "startedAt": started, "completedAt": started + 10,
            "totalRequests": 1, "totalTests": 2, "passedTests": 1, "failedTests": 1, "results": [],
        })
    }

    #[tokio::test]
    async fn config_round_trips_and_folders_with_a_config_are_listed() {
        let _root = scratch_data_root();
        assert_eq!(
            call("runner.loadConfig", json!({ "workspaceId": WS, "folderId": "f1" })).await.unwrap(),
            Value::Null
        );
        assert!(
            call("runner.listFolderIds", json!({ "workspaceId": WS })).await.unwrap().as_array().unwrap().is_empty()
        );
        let saved = call(
            "runner.saveConfig",
            json!({ "workspaceId": WS, "folderId": "f1", "config": { "delayMs": 5, "requestOrder": ["a"] } }),
        )
        .await
        .unwrap();
        assert_eq!(saved, json!({ "ok": true }));
        let loaded = call("runner.loadConfig", json!({ "workspaceId": WS, "folderId": "f1" })).await.unwrap();
        assert_eq!(loaded, json!({ "delayMs": 5, "requestOrder": ["a"] }));
        let ids = call("runner.listFolderIds", json!({ "workspaceId": WS })).await.unwrap();
        assert_eq!(ids, json!(["f1"]));
    }

    #[tokio::test]
    async fn reports_are_saved_and_listed_newest_first() {
        let root = scratch_data_root();
        for started in [1000, 3000, 2000] {
            let saved =
                call("runner.saveReport", json!({ "workspaceId": WS, "report": report(started) })).await.unwrap();
            assert_eq!(saved, json!({ "ok": true }));
        }
        let dir = ws_dir(root.ws_id).join("requests").join(".runs").join("f1").join("3000");
        assert!(dir.join("report.json").exists() && dir.join("report.html").exists());
        let history = call("runner.getHistory", json!({ "workspaceId": WS, "folderId": "f1" })).await.unwrap();
        let stamps: Vec<i64> = history.as_array().unwrap().iter().map(|e| e["timestamp"].as_i64().unwrap()).collect();
        assert_eq!(stamps, vec![3000, 2000, 1000]);
        assert_eq!(history[0]["summary"], json!({ "total": 2, "passed": 1, "failed": 1 }));
        let none = call("runner.getHistory", json!({ "workspaceId": WS, "folderId": "other" })).await.unwrap();
        assert_eq!(none, json!([]));
    }

    #[tokio::test]
    async fn an_exported_report_is_published_under_its_file_name() {
        let _root = scratch_data_root();
        let out = call("runner.exportReport", json!({ "workspaceId": WS, "report": report(0), "format": "json" }))
            .await
            .unwrap();
        assert_eq!(out["suggestedName"], "F-report-1970-01-01T00-00-00-000Z.json");
        assert_eq!(out["mimeType"], "application/json");
    }

    #[tokio::test]
    async fn status_and_cancel_of_an_unknown_run() {
        assert_eq!(
            call("runner.status", json!({ "workspaceId": WS, "folderId": "never" })).await.unwrap(),
            Value::Null
        );
        let cancelled = call("runner.cancel", json!({ "workspaceId": WS, "folderId": "never" })).await.unwrap();
        assert_eq!(cancelled["ok"], false);
    }

    #[tokio::test]
    async fn a_run_sends_the_folders_requests_in_order_and_reports() {
        let _root = scratch_data_root();
        runs::reset_runs();
        let base = test_server::start(HashMap::new()).await;
        for (id, name) in [("r1", "first"), ("r2", "second")] {
            write_entity(
                WS,
                "requests",
                id,
                &json!({
                    "id": id, "name": name, "method": "GET", "url": format!("{base}/{id}"), "headers": {},
                    "body": "", "folderId": "f1", "workspaceId": WS, "createdAt": 0,
                    "testScript": "lp.test('ok', () => { lp.expect(lp.response.status).to.equal(200); });",
                }),
                None,
            )
            .unwrap();
        }
        call(
            "runner.saveConfig",
            json!({ "workspaceId": WS, "folderId": "f1", "config": { "requestOrder": ["r2", "r1"] } }),
        )
        .await
        .unwrap();

        let finished = Arc::new(parking_lot::Mutex::new(Vec::<Value>::new()));
        let sink = finished.clone();
        let listener = bus().on("runner.finished", move |payload| sink.lock().push(payload.clone()));

        let (registry, ctx) = context();
        let started =
            registry.invoke("runner.start", json!({ "workspaceId": WS, "folderId": "f1" }), ctx.clone()).await.unwrap();
        assert_eq!(started, json!({ "ok": true, "total": 2 }));
        let again =
            registry.invoke("runner.start", json!({ "workspaceId": WS, "folderId": "f1" }), ctx.clone()).await.unwrap();
        assert_eq!(again["ok"], false);

        for _ in 0..200 {
            if !finished.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bus().off(listener);

        let status =
            registry.invoke("runner.status", json!({ "workspaceId": WS, "folderId": "f1" }), ctx).await.unwrap();
        assert_eq!(status["running"], false);
        assert_eq!(status["completed"], 2);
        let names: Vec<&str> = status["results"].as_array().unwrap().iter().map(|r| str_of(r, "requestName")).collect();
        assert_eq!(names, vec!["second", "first"]);
        assert_eq!(status["results"][0]["status"], 200);
        assert_eq!(status["report"]["totalRequests"], 2);
        assert_eq!(status["report"]["passedTests"], 2);
        assert_eq!(finished.lock().len(), 1);
        let row = &status["results"][0];
        assert!(row["scripts"]["test"].as_str().unwrap().contains("lp.test('ok'"));
        assert_eq!(row["tests"][0]["name"], "ok");
        assert!(row["responseHeaders"].is_object());
        assert!(row["responseBody"].is_string());
        assert!(row["requestHeaders"].is_object() || row["requestHeaders"].is_array());
        let saved = call("runner.getHistory", json!({ "workspaceId": WS, "folderId": "f1" })).await.unwrap();
        assert_eq!(saved[0]["summary"], json!({ "total": 2, "passed": 2, "failed": 0 }));
    }
}
