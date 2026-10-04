//! `audit.export` and `capture.shareJson`: file-shaped results published inline or as a blob.

use serde_json::Value;

use crate::{
    kernel::CommandRegistry,
    utils::json::{array_of, opt_str, str_of},
    workspace::{
        blob::publish_artifact,
        config::load_config,
        git_store::{QueryLogOptions, query_log},
    },
};

pub fn register(registry: &mut CommandRegistry) {
    registry.register("audit.export", |params, _| async move {
        let workspace_id = match opt_str(&params, "workspaceId") {
            Some(id) => id.to_string(),
            None => str_of(&load_config(), "activeWorkspaceId").to_string(),
        };
        let (entries, _) = query_log(&QueryLogOptions {
            workspace_id: workspace_id.clone(),
            entity: None,
            action: None,
            entity_id: None,
            file_path: None,
            from_ts: None,
            to_ts: None,
            search: None,
            limit: Some(0),
            offset: None,
        })
        .await;
        let format = str_of(&params, "format");
        let (content, mime) = if format == "json" {
            (serde_json::to_string_pretty(&entries)?, "application/json")
        } else {
            (audit_entries_to_csv(&entries), "text/csv")
        };
        publish_artifact(&content, &format!("audit-log.{format}"), mime, Some(&workspace_id))
    });

    registry.register_sync("capture.shareJson", |params, _| {
        let entries = Value::Array(array_of(&params, "entries").to_vec());
        let name = match opt_str(&params, "suggestedName") {
            Some(name) if !name.is_empty() => name,
            _ => "captured-requests.json",
        };
        publish_artifact(
            &serde_json::to_string_pretty(&entries)?,
            name,
            "application/json",
            opt_str(&params, "workspaceId"),
        )
    });
}

/// JavaScript's string conversion of a value in a CSV cell: `undefined`/`null` become empty text.
fn cell(entry: &Value, key: &str) -> String {
    match entry.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn audit_entries_to_csv(entries: &[Value]) -> String {
    let mut lines = vec!["commitHash,ts,action,entity,entityId,entityName,workspaceId,actor".to_string()];
    for e in entries {
        let quoted_name = format!("\"{}\"", cell(e, "entityName").replace('"', "\"\""));
        lines.push(
            [
                cell(e, "commitHash"),
                cell(e, "ts"),
                cell(e, "action"),
                cell(e, "entity"),
                cell(e, "entityId"),
                quoted_name,
                cell(e, "workspaceId"),
                cell(e, "actor"),
            ]
            .join(","),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde_json::json;

    use super::*;
    use crate::{kernel::CommandContext, testing::scratch_data_root};

    fn ctx(registry: &Arc<CommandRegistry>) -> CommandContext {
        CommandContext::internal(registry.clone())
    }

    #[test]
    fn csv_quotes_names_and_leaves_missing_cells_empty() {
        let csv = audit_entries_to_csv(&[
            json!({ "commitHash": "abc", "ts": 5, "action": "create", "entity": "mock", "entityId": "m1",
                    "entityName": "say \"hi\"", "workspaceId": "w", "actor": "me" }),
            json!({ "commitHash": "d", "ts": 6, "action": "delete", "entity": "rule", "entityId": "r",
                    "entityName": "", "workspaceId": "w" }),
        ]);
        assert_eq!(
            csv,
            "commitHash,ts,action,entity,entityId,entityName,workspaceId,actor\n\
             abc,5,create,mock,m1,\"say \"\"hi\"\"\",w,me\n\
             d,6,delete,rule,r,\"\",w,"
        );
    }

    #[tokio::test]
    async fn share_json_publishes_pretty_json_inline() {
        let _root = scratch_data_root();
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let out = registry
            .invoke("capture.shareJson", json!({ "entries": [{ "a": 1 }], "suggestedName": "" }), ctx(&registry))
            .await
            .unwrap();
        assert_eq!(out["suggestedName"], "captured-requests.json");
        assert_eq!(out["mimeType"], "application/json");
        let body = STANDARD.decode(out["inline"].as_str().unwrap()).unwrap();
        assert_eq!(String::from_utf8(body).unwrap(), "[\n  {\n    \"a\": 1\n  }\n]");
    }

    #[tokio::test]
    async fn audit_export_without_history_is_an_empty_artifact() {
        let _root = scratch_data_root();
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let out = registry.invoke("audit.export", json!({ "format": "json" }), ctx(&registry)).await.unwrap();
        assert_eq!(out["suggestedName"], "audit-log.json");
        let body = STANDARD.decode(out["inline"].as_str().unwrap()).unwrap();
        assert_eq!(String::from_utf8(body).unwrap(), "[]");
        let csv = registry.invoke("audit.export", json!({ "format": "csv" }), ctx(&registry)).await.unwrap();
        assert_eq!(csv["mimeType"], "text/csv");
    }
}
