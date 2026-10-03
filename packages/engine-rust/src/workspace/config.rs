//! The config snapshot (`config.get`): the app settings joined with the active workspace's entities.
//!
//! Mocks, mappings and environments are read in full. The request-like kinds are listed from their names
//! index — a client opens one with `entity.load` — so the snapshot stays small.

use indexmap::IndexSet;
use serde_json::{Map, Value, json};

use super::{
    fs::{
        EntityStub, auto_sync_fs_directories, bootstrap_names_index, get_pending_deletions, load_enabled_set,
        read_all_entities, read_entity, read_entity_stubs, read_names_index, write_entity, write_flat_entity,
    },
    settings::{DEFAULT_COMPANION_PORT, DEFAULT_FORWARD_PROXY_PORT, load_settings, save_settings},
};
use crate::{
    env::effective_environment_id,
    utils::{
        id::{generate_id, now_ms},
        json::{array_of, opt_str, str_of},
    },
};

pub const GLOBAL_ENV_ID: &str = "__global__";

/// `entity` with `workspaceId` defaulted and `enabled` taken from the enabled set.
fn with_state(entity: &Value, ws_id: &str, enabled: &IndexSet<String>) -> Value {
    let mut out = entity.as_object().cloned().unwrap_or_default();
    if out.get("workspaceId").is_none_or(Value::is_null) {
        out.insert("workspaceId".into(), json!(ws_id));
    }
    out.insert("enabled".into(), json!(enabled.contains(str_of(entity, "id"))));
    Value::Object(out)
}

fn with_workspace(entity: &Value, ws_id: &str) -> Value {
    let mut out = entity.as_object().cloned().unwrap_or_default();
    if out.get("workspaceId").is_none_or(Value::is_null) {
        out.insert("workspaceId".into(), json!(ws_id));
    }
    Value::Object(out)
}

/// What the proxies need of one workspace: its mocks, mappings and environments.
#[derive(Debug, Clone)]
pub struct WorkspaceDispatchSlice {
    pub workspace_id: String,
    pub mocks: Vec<Value>,
    pub mappings: Vec<Value>,
    pub environments: Vec<Value>,
    pub active_environment_id: Option<String>,
}

pub fn load_workspace_dispatch_slice(ws_id: &str) -> WorkspaceDispatchSlice {
    let mock_enabled = load_enabled_set(ws_id, "mocks");
    let mapping_enabled = load_enabled_set(ws_id, "mappings");
    let environments = read_all_entities(ws_id, "environments");
    let stored = load_settings().stored_environment_id(ws_id);
    WorkspaceDispatchSlice {
        workspace_id: ws_id.to_string(),
        mocks: read_all_entities(ws_id, "mocks")
            .iter()
            .map(|m| {
                let mut out = m.as_object().cloned().unwrap_or_default();
                out.insert("enabled".into(), json!(mock_enabled.contains(str_of(m, "id"))));
                Value::Object(out)
            })
            .collect(),
        mappings: read_all_entities(ws_id, "mappings").iter().map(|m| with_state(m, ws_id, &mapping_enabled)).collect(),
        active_environment_id: effective_environment_id(&environments, stored.as_deref()),
        environments,
    }
}

/// The names index of a kind, rebuilt from the entity files when it is missing but entities exist.
fn names_or_bootstrap(ws_id: &str, kind: &str, stubs: &[EntityStub]) -> Map<String, Value> {
    let names = read_names_index(ws_id, kind);
    if !names.is_empty() || stubs.is_empty() { names } else { bootstrap_names_index(ws_id, kind) }
}

fn name_field<'a>(names: &'a Map<String, Value>, id: &str, field: &str) -> Option<&'a str> {
    names.get(id).and_then(|entry| opt_str(entry, field))
}

fn stub_base(stub: &EntityStub, ws_id: &str) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("id".into(), json!(stub.id));
    out.insert("folderId".into(), json!(stub.folder_id));
    out.insert("workspaceId".into(), json!(ws_id));
    out
}

fn folders(ws_id: &str, kind: &str) -> Value {
    Value::Array(auto_sync_fs_directories(ws_id, kind, generate_id).folders)
}

/// Entities deleted locally and not yet published, absent from `present`, shaped by `shape`.
fn pending(ws_id: &str, kind: &str, present: &[Value], shape: impl Fn(&Value) -> Value) -> Vec<Value> {
    get_pending_deletions(ws_id, kind)
        .iter()
        .filter(|p| !present.iter().any(|e| str_of(e, "id") == str_of(p, "id")))
        .map(shape)
        .collect()
}

/// `folderId` of a pending deletion: its value, or absent when it has none.
fn pending_folder(out: &mut Map<String, Value>, p: &Value) {
    if let Some(folder_id) = opt_str(p, "folderId") {
        out.insert("folderId".into(), json!(folder_id));
    }
}

/// The config snapshot for the active workspace.
pub fn load_config() -> Value {
    let settings = load_settings();
    let ws_id = settings.active_workspace_id();
    let ws_id = ws_id.as_str();

    let mock_enabled = load_enabled_set(ws_id, "mocks");
    let mapping_enabled = load_enabled_set(ws_id, "mappings");
    let rule_enabled = load_enabled_set(ws_id, "rules");

    let mocks: Vec<Value> =
        read_all_entities(ws_id, "mocks").iter().map(|m| with_state(m, ws_id, &mock_enabled)).collect();
    let mappings: Vec<Value> =
        read_all_entities(ws_id, "mappings").iter().map(|m| with_state(m, ws_id, &mapping_enabled)).collect();

    let mut environments = read_all_entities(ws_id, "environments");
    if !environments.iter().any(|e| str_of(e, "id") == GLOBAL_ENV_ID) {
        let global = json!({
            "id": GLOBAL_ENV_ID, "name": "Global", "variables": [], "createdAt": now_ms(), "workspaceId": ws_id,
        });
        let _ = write_flat_entity(ws_id, "environments", GLOBAL_ENV_ID, &global);
        environments.insert(0, global);
    }

    let request_stubs = read_entity_stubs(ws_id, "requests");
    let socket_stubs = read_entity_stubs(ws_id, "sockets");
    let rule_stubs = read_entity_stubs(ws_id, "rules");
    let request_names = names_or_bootstrap(ws_id, "requests", &request_stubs);
    let socket_names = names_or_bootstrap(ws_id, "sockets", &socket_stubs);
    let rule_names = names_or_bootstrap(ws_id, "rules", &rule_stubs);

    let mock_folders = folders(ws_id, "mocks");
    let request_folders = folders(ws_id, "requests");
    let ws_folders = folders(ws_id, "sockets");
    let rule_folders = folders(ws_id, "rules");

    let active_env = effective_environment_id(&environments, settings.stored_environment_id(ws_id).as_deref());

    let rule_shape = |id: &str, folder: Option<Value>, name: &str, pattern: &str, enabled: bool| -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        if let Some(folder) = folder {
            out.insert("folderId".into(), folder);
        }
        out.insert("workspaceId".into(), json!(ws_id));
        out.insert("name".into(), json!(name));
        out.insert("pattern".into(), json!(pattern));
        out.insert("useRegex".into(), json!(true));
        out.insert("targetType".into(), json!("mapping"));
        out.insert("targetMappingId".into(), json!(""));
        out.insert("targetExternal".into(), json!(""));
        out.insert("requestScript".into(), json!(""));
        out.insert("responseScript".into(), json!(""));
        out.insert("enabled".into(), json!(enabled));
        out.insert("createdAt".into(), json!(0));
        Value::Object(out)
    };
    let mut proxy_rules: Vec<Value> = rule_stubs
        .iter()
        .map(|s| {
            rule_shape(
                &s.id,
                Some(json!(s.folder_id)),
                name_field(&rule_names, &s.id, "name").unwrap_or(""),
                name_field(&rule_names, &s.id, "url").unwrap_or(""),
                rule_enabled.contains(&s.id),
            )
        })
        .collect();
    for p in get_pending_deletions(ws_id, "rules") {
        if !rule_stubs.iter().any(|s| s.id == str_of(&p, "id")) {
            let folder = opt_str(&p, "folderId").map(|f| json!(f));
            proxy_rules.push(rule_shape(str_of(&p, "id"), folder, str_of(&p, "name"), str_of(&p, "url"), false));
        }
    }

    let mut all_mappings = mappings.clone();
    all_mappings.extend(pending(ws_id, "mappings", &mappings, |p| {
        json!({
            "id": str_of(p, "id"), "workspaceId": ws_id, "domain": str_of(p, "url"), "target": "",
            "label": p.get("name").cloned().unwrap_or(Value::Null), "enabled": false, "createdAt": 0,
        })
    }));

    let mut all_mocks = mocks.clone();
    all_mocks.extend(pending(ws_id, "mocks", &mocks, |p| {
        let mut out = Map::new();
        out.insert("id".into(), json!(str_of(p, "id")));
        out.insert("workspaceId".into(), json!(ws_id));
        pending_folder(&mut out, p);
        out.insert("name".into(), p.get("name").cloned().unwrap_or(Value::Null));
        out.insert("method".into(), json!(opt_str(p, "method").unwrap_or("GET")));
        out.insert("urlPattern".into(), json!(str_of(p, "url")));
        out.insert("useRegex".into(), json!(false));
        out.insert("enabled".into(), json!(false));
        out.insert("createdAt".into(), json!(0));
        out.insert("capturedHeaders".into(), json!({}));
        out.insert("capturedBody".into(), json!(""));
        out.insert("responseStatus".into(), json!(200));
        out.insert("responseHeaders".into(), json!({}));
        out.insert("responseBody".into(), json!(""));
        Value::Object(out)
    }));

    let mut all_environments: Vec<Value> = environments.iter().map(|e| with_workspace(e, ws_id)).collect();
    all_environments.extend(pending(ws_id, "environments", &environments, |p| {
        json!({
            "id": str_of(p, "id"), "name": p.get("name").cloned().unwrap_or(Value::Null),
            "variables": [], "createdAt": 0, "workspaceId": ws_id,
        })
    }));

    let requests: Vec<Value> = request_stubs
        .iter()
        .map(|s| {
            let mut out = stub_base(s, ws_id);
            out.insert("name".into(), json!(name_field(&request_names, &s.id, "name").unwrap_or("")));
            out.insert("method".into(), json!(name_field(&request_names, &s.id, "method").unwrap_or("GET")));
            out.insert("url".into(), json!(name_field(&request_names, &s.id, "url").unwrap_or("")));
            out.insert("headers".into(), json!({}));
            out.insert("body".into(), json!(""));
            out.insert("createdAt".into(), json!(0));
            Value::Object(out)
        })
        .collect();

    let ws_connections: Vec<Value> = socket_stubs
        .iter()
        .map(|s| {
            let mut out = stub_base(s, ws_id);
            out.insert("name".into(), json!(name_field(&socket_names, &s.id, "name").unwrap_or("")));
            out.insert("url".into(), json!(name_field(&socket_names, &s.id, "url").unwrap_or("")));
            out.insert("headers".into(), json!({}));
            out.insert("createdAt".into(), json!(0));
            Value::Object(out)
        })
        .collect();

    // GraphQL, gRPC and SOAP requests: listed from the names index, with the fields a listing shows.
    let listed = |kind: &str, fields: &[&str], rest: Value| -> Vec<Value> {
        let names = read_names_index(ws_id, kind);
        read_entity_stubs(ws_id, kind)
            .iter()
            .map(|s| {
                let mut out = stub_base(s, ws_id);
                out.insert("name".into(), json!(name_field(&names, &s.id, "name").unwrap_or("")));
                for field in fields {
                    out.insert((*field).into(), json!(name_field(&names, &s.id, field).unwrap_or("")));
                }
                for (key, value) in rest.as_object().into_iter().flatten() {
                    out.insert(key.clone(), value.clone());
                }
                Value::Object(out)
            })
            .collect()
    };
    let graphql_requests = listed(
        "graphqlRequests",
        &["endpointUrl"],
        json!({ "headers": {}, "query": "", "variables": "", "operationName": "", "createdAt": 0 }),
    );
    let grpc_requests = listed(
        "grpcRequests",
        &["serverAddress", "serviceName", "methodName"],
        json!({
            "requestBody": "", "metadata": {}, "protoFileId": null, "useReflection": false,
            "streamingType": "unary", "createdAt": 0,
        }),
    );
    let soap_requests =
        listed("soapRequests", &["endpointUrl", "soapAction"], json!({ "headers": {}, "body": "", "createdAt": 0 }));

    let workspaces: Vec<Value> = settings
        .workspaces()
        .iter()
        .map(|w| {
            json!({
                "id": w.get("id").cloned().unwrap_or(Value::Null),
                "name": w.get("name").cloned().unwrap_or(Value::Null),
                "createdAt": 0,
                "activeEnvironmentId": w.get("activeEnvironmentId").cloned().unwrap_or(Value::Null),
                "syncConfig": w.get("syncConfig").cloned().unwrap_or(Value::Null),
                "syncMeta": w.get("syncMeta").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    let raw = settings.as_value();
    json!({
        "port": raw.get("port").cloned().unwrap_or(Value::Null),
        "forwardProxyPort": raw.get("forwardProxyPort").filter(|v| !v.is_null()).cloned().unwrap_or(json!(DEFAULT_FORWARD_PROXY_PORT)),
        "companionPort": raw.get("companionPort").filter(|v| !v.is_null()).cloned().unwrap_or(json!(DEFAULT_COMPANION_PORT)),
        "minimizeToTray": raw.get("minimizeToTray").cloned().unwrap_or(Value::Null),
        "tlsEnabled": settings.tls_enabled(),
        "tlsCaCertPath": settings.tls_ca_cert_path(),
        "tlsCaKeyPath": settings.tls_ca_key_path(),
        "workspaces": workspaces,
        "activeWorkspaceId": ws_id,
        "activeEnvironmentId": active_env,
        "mappings": all_mappings,
        "proxyRules": proxy_rules,
        "ruleFolders": rule_folders,
        "mocks": all_mocks,
        "requests": requests,
        "wsConnections": ws_connections,
        "mockFolders": mock_folders,
        "requestFolders": request_folders,
        "wsFolders": ws_folders,
        "environments": all_environments,
        "graphqlRequests": graphql_requests,
        "graphqlSchemas": [],
        "graphqlRequestFolders": folders(ws_id, "graphqlRequests"),
        "grpcRequests": grpc_requests,
        "protoFiles": read_all_entities(ws_id, "protoFiles"),
        "grpcRequestFolders": folders(ws_id, "grpcRequests"),
        "soapRequests": soap_requests,
        "savedWsdls": [],
        "soapRequestFolders": folders(ws_id, "soapRequests"),
    })
}

/// One entity, in full, from its file.
pub fn load_entity(ws_id: &str, kind: &str, id: &str) -> Option<Value> {
    read_entity(ws_id, kind, id)
}

/// Persist a config snapshot: the settings fields and workspace list to `app.json`, and the active
/// workspace's mocks, mappings and environments to their files. The request-like kinds are written by their
/// own commands, never from a snapshot (a snapshot holds only their stubs).
pub fn save_config(cfg: &Value) -> std::io::Result<()> {
    let mut settings = load_settings();
    settings.set("port", cfg.get("port").cloned().unwrap_or(Value::Null));
    for (key, default) in [("forwardProxyPort", DEFAULT_FORWARD_PROXY_PORT), ("companionPort", DEFAULT_COMPANION_PORT)]
    {
        settings.set(key, cfg.get(key).filter(|v| !v.is_null()).cloned().unwrap_or(json!(default)));
    }
    settings.set("minimizeToTray", cfg.get("minimizeToTray").cloned().unwrap_or(Value::Null));
    settings.set("tlsEnabled", cfg.get("tlsEnabled").filter(|v| !v.is_null()).cloned().unwrap_or(json!(false)));
    settings.set("tlsCaCertPath", cfg.get("tlsCaCertPath").cloned().unwrap_or(Value::Null));
    settings.set("tlsCaKeyPath", cfg.get("tlsCaKeyPath").cloned().unwrap_or(Value::Null));
    settings.set("activeWorkspaceId", cfg.get("activeWorkspaceId").cloned().unwrap_or(Value::Null));

    let cfg_workspaces = array_of(cfg, "workspaces");
    for ws in cfg_workspaces {
        let id = str_of(ws, "id");
        match settings.workspace_mut(id) {
            Some(existing) => {
                existing["name"] = ws.get("name").cloned().unwrap_or(Value::Null);
                existing["activeEnvironmentId"] = ws.get("activeEnvironmentId").cloned().unwrap_or(Value::Null);
            }
            None => {
                let mut meta = Map::new();
                meta.insert("id".into(), json!(id));
                meta.insert("name".into(), ws.get("name").cloned().unwrap_or(Value::Null));
                meta.insert(
                    "activeEnvironmentId".into(),
                    ws.get("activeEnvironmentId").cloned().unwrap_or(Value::Null),
                );
                for key in ["syncConfig", "syncMeta"] {
                    if let Some(value) = ws.get(key).filter(|v| !v.is_null()) {
                        meta.insert(key.into(), value.clone());
                    }
                }
                settings.workspaces_mut().push(Value::Object(meta));
            }
        }
    }
    settings.workspaces_mut().retain(|w| cfg_workspaces.iter().any(|c| str_of(c, "id") == str_of(w, "id")));
    save_settings(&settings)?;

    let ws_id = str_of(cfg, "activeWorkspaceId");
    let pending_mock_ids: Vec<String> =
        get_pending_deletions(ws_id, "mocks").iter().map(|p| str_of(p, "id").to_string()).collect();
    let mock_folders = array_of(cfg, "mockFolders");
    for mock in array_of(cfg, "mocks") {
        let id = str_of(mock, "id");
        if str_of(mock, "workspaceId") != ws_id || pending_mock_ids.iter().any(|p| p == id) {
            continue;
        }
        let folder_name = opt_str(mock, "folderId")
            .filter(|f| !f.is_empty())
            .and_then(|folder_id| mock_folders.iter().find(|f| str_of(f, "id") == folder_id))
            .map(|f| str_of(f, "name"));
        write_entity(ws_id, "mocks", id, mock, folder_name)?;
    }
    for kind in ["mappings", "environments"] {
        for entity in array_of(cfg, kind) {
            if str_of(entity, "workspaceId") == ws_id {
                write_flat_entity(ws_id, kind, str_of(entity, "id"), entity)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{sync_enabled_set, upsert_name_entry, write_entity},
    };

    #[test]
    fn the_snapshot_lists_requests_from_the_names_index_and_reads_mocks_in_full() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let request = json!({ "id": "r1", "name": "Ping", "method": "POST", "url": "http://a/ping", "body": "big" });
        write_entity(ws, "requests", "r1", &request, None).unwrap();
        upsert_name_entry(ws, "requests", "r1", json!({ "name": "Ping", "method": "POST", "url": "http://a/ping" }))
            .unwrap();
        let mock = json!({ "id": "m1", "name": "M", "method": "GET", "urlPattern": "/x", "responseBody": "{}" });
        write_entity(ws, "mocks", "m1", &mock, None).unwrap();
        sync_enabled_set(ws, "mocks", "m1", true).unwrap();

        let cfg = load_config();
        assert_eq!(cfg["activeWorkspaceId"], ws);
        assert_eq!(cfg["requests"][0]["name"], "Ping");
        assert_eq!(cfg["requests"][0]["method"], "POST");
        assert_eq!(cfg["requests"][0]["body"], "", "a listing does not carry the body");
        assert_eq!(cfg["mocks"][0]["responseBody"], "{}");
        assert_eq!(cfg["mocks"][0]["enabled"], true);
        assert_eq!(cfg["mocks"][0]["workspaceId"], ws);
        assert_eq!(cfg["environments"][0]["id"], GLOBAL_ENV_ID);
        assert_eq!(load_entity(ws, "requests", "r1").unwrap()["body"], "big");
    }

    #[test]
    fn saving_a_snapshot_updates_settings_and_flat_entities() {
        let root = scratch_data_root();
        let mut cfg = load_config();
        cfg["forwardProxyPort"] = json!(9090);
        cfg["mappings"] = json!([{ "id": "map1", "domain": "app.localhost", "target": "localhost:3000",
            "enabled": true, "workspaceId": root.ws_id }]);
        save_config(&cfg).unwrap();
        assert_eq!(load_settings().forward_proxy_port(), 9090);
        let stored = load_entity(root.ws_id, "mappings", "map1").unwrap();
        assert_eq!(stored["domain"], "app.localhost");
        assert!(stored.get("enabled").is_none(), "enabled is not stored in the entity file");
    }
}
