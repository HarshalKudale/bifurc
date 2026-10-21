//! `audit.list`.

use serde_json::{Value, json};

use crate::{
    kernel::CommandRegistry,
    utils::json::{opt_str, str_of},
    workspace::{
        config::load_config,
        git_store::{QueryLogOptions, query_log},
    },
};

fn text(params: &Value, key: &str) -> Option<String> {
    opt_str(params, key).map(str::to_string)
}

fn number(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

fn count(params: &Value, key: &str) -> Option<usize> {
    number(params, key).and_then(|n| usize::try_from(n).ok())
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("audit.list", |params, _| async move {
        let workspace_id = match opt_str(&params, "workspaceId") {
            Some(id) => id.to_string(),
            None => str_of(&load_config(), "activeWorkspaceId").to_string(),
        };
        let options = QueryLogOptions {
            workspace_id,
            entity: text(&params, "entity"),
            action: text(&params, "action"),
            entity_id: text(&params, "entityId"),
            file_path: text(&params, "filePath"),
            from_ts: number(&params, "fromTs"),
            to_ts: number(&params, "toTs"),
            search: text(&params, "search"),
            limit: count(&params, "limit"),
            offset: count(&params, "offset"),
        };
        let (entries, total) = query_log(&options).await;
        Ok(json!({ "entries": entries, "total": total }))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        kernel::CommandContext,
        testing::scratch_data_root,
        workspace::{
            fs::{entity_dir, write_entity},
            git_store::{CommitMutation, check_git_installed, commit_mutation, init_workspace_repo},
        },
    };

    #[tokio::test]
    async fn a_commit_made_in_a_named_session_carries_its_name() {
        if !check_git_installed().await {
            return;
        }
        let _root = scratch_data_root();
        init_workspace_repo("ws1").await.unwrap();
        write_entity("ws1", "mocks", "m1", &json!({ "id": "m1", "name": "First" }), None).unwrap();
        let commit = CommitMutation {
            action: "create",
            entity: "mock",
            entity_id: "m1",
            entity_name: "First",
            workspace_id: "ws1",
            rel_path: "mocks/m1.json",
            actor: Some("tester"),
            changed_fields: &[],
        };
        crate::env::session::enter("ws1", "backend", commit_mutation(commit)).await.unwrap();
        let (entries, _) = query_log(&QueryLogOptions { workspace_id: "ws1".into(), ..Default::default() }).await;
        assert_eq!((entries[0]["session"].as_str(), entries[0]["actor"].as_str()), (Some("backend"), Some("tester")));
        assert!(entries[1].get("session").is_none());
    }

    #[tokio::test]
    async fn the_audit_log_is_listed_filtered_and_paged() {
        if !check_git_installed().await {
            return;
        }
        let _root = scratch_data_root();
        init_workspace_repo("ws1").await.unwrap();
        for (id, name) in [("m1", "First"), ("m2", "Second")] {
            write_entity("ws1", "mocks", id, &json!({ "id": id, "name": name }), None).unwrap();
            let rel_path = format!("mocks/{id}.json");
            let commit = CommitMutation {
                action: "create",
                entity: "mock",
                entity_id: id,
                entity_name: name,
                workspace_id: "ws1",
                rel_path: &rel_path,
                actor: Some("tester"),
                changed_fields: &[],
            };
            commit_mutation(commit).await.unwrap();
        }
        assert!(entity_dir("ws1", "mocks").join("m2.json").exists());

        let mut registry = CommandRegistry::new();
        crate::workspace::entities::register(&mut registry);
        let ctx = CommandContext::internal(Arc::new(registry));

        let all = ctx.registry.invoke("audit.list", json!({}), ctx.clone()).await.unwrap();
        // The two mocks, and the commit that created the workspace.
        assert_eq!(all["total"], 3);
        assert_eq!(all["entries"][0]["entityName"], "Second");
        assert_eq!(all["entries"][0]["actor"], "tester");
        assert_eq!(all["entries"][2]["entity"], "workspace");
        assert_eq!(all["entries"][2]["action"], "create");

        let one = ctx
            .registry
            .invoke("audit.list", json!({ "workspaceId": "ws1", "search": "first" }), ctx.clone())
            .await
            .unwrap();
        assert_eq!(one["total"], 1);
        assert_eq!(one["entries"][0]["entityId"], "m1");

        let page = ctx.registry.invoke("audit.list", json!({ "limit": 1, "offset": 1 }), ctx.clone()).await.unwrap();
        assert_eq!(page["total"], 3);
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page["entries"][0]["entityName"], "First");
    }
}
