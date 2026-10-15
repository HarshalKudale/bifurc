//! The collection runner: run a folder's requests and keep its run configuration under
//! `<workspace>/requests/.runs/<folderId>/`.

pub mod report_html;
pub mod runs;

use std::path::PathBuf;

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
        let folder = crate::workspace::fs::create_folder(WS, "requests", "F", None, false).unwrap().0;
        let f1 = folder.as_str();
        for (id, name) in [("r1", "first"), ("r2", "second")] {
            write_entity(
                WS,
                "requests",
                id,
                &json!({
                    "id": id, "name": name, "method": "GET", "url": format!("{base}/{id}"), "headers": {},
                    "body": "", "folderId": f1, "workspaceId": WS, "createdAt": 0,
                    "testScript": "bc.test('ok', () => { bc.expect(bc.response.status).to.equal(200); });",
                }),
                Some(f1),
            )
            .unwrap();
        }
        call(
            "runner.saveConfig",
            json!({ "workspaceId": WS, "folderId": f1, "config": { "requestOrder": ["r2", "r1"] } }),
        )
        .await
        .unwrap();

        let finished = Arc::new(parking_lot::Mutex::new(Vec::<Value>::new()));
        let sink = finished.clone();
        let listener = bus().on("runner.finished", move |payload| sink.lock().push(payload.clone()));

        let (registry, ctx) = context();
        let started =
            registry.invoke("runner.start", json!({ "workspaceId": WS, "folderId": f1 }), ctx.clone()).await.unwrap();
        assert_eq!(started, json!({ "ok": true, "total": 2 }));
        let again =
            registry.invoke("runner.start", json!({ "workspaceId": WS, "folderId": f1 }), ctx.clone()).await.unwrap();
        assert_eq!(again["ok"], false);

        for _ in 0..200 {
            if !finished.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bus().off(listener);

        let status =
            registry.invoke("runner.status", json!({ "workspaceId": WS, "folderId": f1 }), ctx).await.unwrap();
        assert_eq!(status["running"], false);
        assert_eq!(status["completed"], 2);
        let names: Vec<&str> = status["results"].as_array().unwrap().iter().map(|r| str_of(r, "requestName")).collect();
        assert_eq!(names, vec!["second", "first"]);
        assert_eq!(status["results"][0]["status"], 200);
        assert_eq!(status["report"]["totalRequests"], 2);
        assert_eq!(status["report"]["passedTests"], 2);
        assert_eq!(finished.lock().len(), 1);
        let row = &status["results"][0];
        assert!(row["scripts"]["test"].as_str().unwrap().contains("bc.test('ok'"));
        assert_eq!(row["tests"][0]["name"], "ok");
        assert!(row["responseHeaders"].is_object());
        assert!(row["responseBody"].is_string());
        assert!(row["requestHeaders"].is_object() || row["requestHeaders"].is_array());
    }
}
