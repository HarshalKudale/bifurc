//! The request export: every request of the workspace, whatever its type, as Bifurc JSON.

use serde_json::{Value, json};

use crate::{
    importexport::{
        support::{in_workspace, pretty, workspace_name},
        types::{ExportFile, ExportOutcome},
    },
    utils::json::str_of,
    workspace::{config::load_config, fs::read_all_entities, tree::Scope},
};

/// The requests of workspace `ws_id`, of every type. Each names its type in `type`.
pub fn workspace_requests(ws_id: &str) -> Vec<Value> {
    Scope::Requests
        .types()
        .iter()
        .flat_map(|(kind, _)| read_all_entities(ws_id, kind))
        .filter(|request| str_of(request, "workspaceId") == ws_id)
        .collect()
}

pub fn json(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let payload = json!({
        "schema": "lp-requests-v1",
        "name": workspace_name(&cfg, ws_id, "Bifurc"),
        "requests": workspace_requests(ws_id),
        "folders": in_workspace(&cfg, "requestFolders", ws_id),
    });
    Ok(ExportFile { content: pretty(&payload), suggested_name: "requests-export.json" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        importexport::{
            importers::requests as importer,
            types::{ImportSource, Strategy},
        },
        testing::scratch_data_root,
        workspace::{
            fs::{create_folder, write_entity},
            tree,
        },
    };

    #[tokio::test]
    async fn every_request_type_is_exported_and_imports_back() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let (folder, _) = create_folder(ws, "requests", "Live", None, false).unwrap();
        let rest = json!({ "id": "r1", "name": "List", "method": "GET", "url": "https://api.test/users", "headers": {},
            "body": "", "createdAt": 1, "workspaceId": ws });
        let socket = json!({ "id": "s1", "name": "Feed", "url": "ws://x", "headers": {}, "message": "hi", "createdAt": 2, "workspaceId": ws });
        let grpc = json!({ "id": "g1", "name": "Rpc", "serverAddress": "h:1", "serviceName": "a.B", "methodName": "C",
            "requestBody": "{}", "metadata": {}, "createdAt": 3, "workspaceId": ws });
        write_entity(ws, "requests", "r1", &rest, Some(&folder)).unwrap();
        write_entity(ws, "sockets", "s1", &socket, Some(&folder)).unwrap();
        write_entity(ws, "grpcRequests", "g1", &grpc, Some(&folder)).unwrap();

        let out = json(ws).unwrap();
        assert_eq!(out.suggested_name, "requests-export.json");
        let file: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(file["schema"], "lp-requests-v1");
        assert_eq!(file["name"], "Workspace 1");
        let types: Vec<_> =
            file["requests"].as_array().unwrap().iter().map(|r| (str_of(r, "id"), str_of(r, "type"))).collect();
        assert_eq!(types, [("r1", "rest"), ("g1", "grpc"), ("s1", "websocket")]);
        assert_eq!(file["requests"][0]["folderId"], folder.as_str());

        let source = ImportSource::text(out.content, "requests-export.json");
        let done = importer::json_run(ws.to_string(), source, Strategy::New).await.unwrap();
        assert_eq!(done.imported, 3);
        let copies = tree::folders(ws, tree::Scope::Requests).into_iter().filter(|f| f.name == "Live").count();
        assert_eq!(copies, 2, "the folder comes with its requests");
        let sockets = read_all_entities(ws, "sockets");
        assert_eq!(sockets.len(), 2);
        assert!(sockets.iter().all(|s| s["message"] == "hi"));
    }
}
