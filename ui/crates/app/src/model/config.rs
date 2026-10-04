//! A typed, read-only view of the engine's `config.get` result.
//!
//! The engine owns the workspace; this is a snapshot the UI renders from and replaces wholesale when the
//! engine says something changed. Nothing here is edited and written back — mutations are engine commands
//! addressed by entity id (`entity.create`, `entity.update`, `entity.delete`, `folder.*`), after which a new
//! snapshot is fetched.

use serde_json::Value;

use super::{kind::RequestKind, tree::FolderRow};

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    /// The account this workspace commits and syncs as, if it has one.
    pub account: Option<AccountInfo>,
    /// The remote repository this workspace syncs with, once it is published or was imported.
    pub sync: Option<SyncConfig>,
}

/// A git account signed in on this device. The engine keeps its token; the UI never sees one.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountInfo {
    pub id: String,
    pub login: String,
    pub host: String,
    /// `false` once GitHub stopped accepting the sign-in: the account needs to sign in again.
    pub signed_in: bool,
}

impl AccountInfo {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: non_empty(value, "id")?,
            login: text(value, "login").unwrap_or_default(),
            host: text(value, "host").unwrap_or_default(),
            signed_in: value.get("signedIn").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncConfig {
    pub remote: String,
    /// `owner/name` on the host, when the engine knows it.
    pub repo: Option<String>,
}

/// What the mocks tree shows for a mock rule. The whole rule is loaded by id when its tab opens.
#[derive(Debug, Clone, PartialEq)]
pub struct MockStub {
    pub id: String,
    pub name: String,
    pub method: String,
    pub url_pattern: String,
    pub enabled: bool,
    pub folder_id: Option<String>,
    /// Deleted here; the deletion is waiting to be confirmed or reverted.
    pub deleted: bool,
}

/// A proxy rule that is deleted here, with the deletion waiting to be confirmed or reverted.
#[derive(Debug, Clone, PartialEq)]
pub struct DeletedRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlainFolder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    /// Deleted here; the deletion is waiting to be confirmed or reverted.
    pub deleted: bool,
}

/// The engine-wide settings a client may change with `settings.update`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineSettings {
    pub proxy_port: u16,
    pub forward_proxy_port: u16,
    pub companion_port: u16,
    pub minimize_to_tray: bool,
    pub tls_enabled: bool,
    /// Anonymous usage analytics are not turned off.
    pub telemetry_enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvVariable {
    pub id: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentInfo {
    pub id: String,
    pub name: String,
    pub variables: Vec<EnvVariable>,
    /// Deleted here; the deletion is waiting to be confirmed or reverted.
    pub deleted: bool,
}

impl EnvironmentInfo {
    /// The always-present environment whose variables every other environment layers over.
    pub const GLOBAL_ID: &'static str = "__global__";

    pub fn is_global(&self) -> bool {
        self.id == Self::GLOBAL_ID
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MappingInfo {
    pub id: String,
    pub domain: String,
    pub target: String,
    pub label: Option<String>,
    pub enabled: bool,
    /// Deleted here; the deletion is waiting to be confirmed or reverted.
    pub deleted: bool,
}

/// What the explorer shows for a saved request: enough to list and find it, not the request itself.
/// The full entity is loaded by id (`entity.load`) when a tab opens.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestStub {
    pub kind: RequestKind,
    pub id: String,
    pub name: String,
    pub folder_id: Option<String>,
    /// The HTTP method, for the kinds that have one.
    pub method: Option<String>,
    /// The URL, address or endpoint this request points at.
    pub target: String,
    /// Deleted here; the deletion is waiting to be confirmed or reverted.
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FolderInfo {
    pub kind: RequestKind,
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    /// Deleted here; the deletion is waiting to be confirmed or reverted.
    pub deleted: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigSnapshot {
    pub workspaces: Vec<WorkspaceInfo>,
    pub active_workspace_id: String,
    pub active_environment_id: Option<String>,
    pub environments: Vec<EnvironmentInfo>,
    pub mappings: Vec<MappingInfo>,
    pub requests: Vec<RequestStub>,
    pub folders: Vec<FolderInfo>,
    pub mocks: Vec<MockStub>,
    pub mock_folders: Vec<PlainFolder>,
    pub proxy_rule_count: usize,
    /// The proxy rules that are deleted here and still listed. The live ones are read with `entity.list`.
    pub deleted_rules: Vec<DeletedRule>,
    pub settings: EngineSettings,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn non_empty(value: &Value, key: &str) -> Option<String> {
    text(value, key).filter(|s| !s.is_empty())
}

/// The engine marks what was deleted here and is still listed, waiting to be confirmed or reverted.
fn is_deleted(value: &Value) -> bool {
    value.get("deleted").and_then(Value::as_bool).unwrap_or(false)
}

fn array<'a>(config: &'a Value, key: &str) -> &'a [Value] {
    config.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

impl ConfigSnapshot {
    pub fn from_value(config: &Value) -> Result<Self, String> {
        let active_workspace_id = text(config, "activeWorkspaceId").ok_or("config.get has no activeWorkspaceId")?;

        let workspaces = array(config, "workspaces")
            .iter()
            .filter_map(|w| {
                let sync = w.get("syncConfig").filter(|s| s.is_object()).and_then(|s| {
                    Some(SyncConfig {
                        remote: non_empty(s, "remote")?,
                        repo: non_empty(s, "repo"),
                    })
                });
                let account = w.get("account").and_then(AccountInfo::from_value);
                Some(WorkspaceInfo { id: text(w, "id")?, name: text(w, "name")?, account, sync })
            })
            .collect();

        let environments = array(config, "environments")
            .iter()
            .filter_map(|e| {
                let variables = e
                    .get("variables")
                    .and_then(Value::as_array)
                    .map(|vars| {
                        vars.iter()
                            .filter_map(|v| {
                                Some(EnvVariable {
                                    id: text(v, "id").unwrap_or_default(),
                                    key: text(v, "key")?,
                                    value: text(v, "value")?,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(EnvironmentInfo { id: text(e, "id")?, name: text(e, "name")?, variables, deleted: is_deleted(e) })
            })
            .collect();

        let mappings = array(config, "mappings")
            .iter()
            .filter_map(|m| {
                Some(MappingInfo {
                    id: text(m, "id")?,
                    domain: text(m, "domain")?,
                    target: text(m, "target").unwrap_or_default(),
                    label: non_empty(m, "label"),
                    enabled: m.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                    deleted: is_deleted(m),
                })
            })
            .collect();

        let mut requests = Vec::new();
        let mut folders = Vec::new();
        for kind in RequestKind::ALL {
            for stub in array(config, kind.config_key()) {
                let Some(id) = text(stub, "id") else { continue };
                requests.push(RequestStub {
                    kind,
                    id,
                    name: text(stub, "name").unwrap_or_default(),
                    folder_id: non_empty(stub, "folderId"),
                    method: (kind == RequestKind::Rest).then(|| text(stub, "method")).flatten(),
                    target: stub_target(kind, stub),
                    deleted: is_deleted(stub),
                });
            }
            for folder in array(config, kind.folders_key()) {
                let (Some(id), Some(name)) = (text(folder, "id"), text(folder, "name")) else { continue };
                let parent_id = non_empty(folder, "parentId");
                folders.push(FolderInfo { kind, id, name, parent_id, deleted: is_deleted(folder) });
            }
        }

        let port = |key: &str| config.get(key).and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).unwrap_or(0);
        let flag = |key: &str| config.get(key).and_then(Value::as_bool).unwrap_or(false);

        let mocks = array(config, "mocks")
            .iter()
            .filter_map(|m| {
                Some(MockStub {
                    id: text(m, "id")?,
                    name: text(m, "name").unwrap_or_default(),
                    method: text(m, "method").unwrap_or_default(),
                    url_pattern: text(m, "urlPattern").unwrap_or_default(),
                    enabled: m.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                    folder_id: non_empty(m, "folderId"),
                    deleted: is_deleted(m),
                })
            })
            .collect();
        let mock_folders = array(config, "mockFolders")
            .iter()
            .filter_map(|f| {
                Some(PlainFolder {
                    id: text(f, "id")?,
                    name: text(f, "name")?,
                    parent_id: non_empty(f, "parentId"),
                    deleted: is_deleted(f),
                })
            })
            .collect();

        Ok(Self {
            workspaces,
            active_workspace_id,
            active_environment_id: non_empty(config, "activeEnvironmentId"),
            environments,
            mappings,
            requests,
            folders,
            mocks,
            mock_folders,
            proxy_rule_count: array(config, "proxyRules").iter().filter(|r| !is_deleted(r)).count(),
            deleted_rules: array(config, "proxyRules")
                .iter()
                .filter(|r| is_deleted(r))
                .filter_map(|r| {
                    Some(DeletedRule {
                        id: text(r, "id")?,
                        name: text(r, "name").unwrap_or_default(),
                        pattern: text(r, "pattern").unwrap_or_default(),
                    })
                })
                .collect(),
            settings: EngineSettings {
                proxy_port: port("port"),
                forward_proxy_port: port("forwardProxyPort"),
                companion_port: port("companionPort"),
                minimize_to_tray: flag("minimizeToTray"),
                tls_enabled: flag("tlsEnabled"),
                telemetry_enabled: config.get("telemetryEnabled").and_then(Value::as_bool).unwrap_or(true),
            },
        })
    }

    pub fn active_workspace(&self) -> Option<&WorkspaceInfo> {
        self.workspaces.iter().find(|w| w.id == self.active_workspace_id)
    }

    pub fn active_environment(&self) -> Option<&EnvironmentInfo> {
        let id = self.active_environment_id.as_deref()?;
        self.environments.iter().find(|e| e.id == id)
    }

    /// The environments a user chooses between: everything except the implicit global layer.
    pub fn selectable_environments(&self) -> impl Iterator<Item = &EnvironmentInfo> {
        self.environments.iter().filter(|e| !e.is_global() && !e.deleted)
    }

    pub fn request(&self, kind: RequestKind, id: &str) -> Option<&RequestStub> {
        self.requests.iter().find(|r| r.kind == kind && r.id == id)
    }

    pub fn mock(&self, id: &str) -> Option<&MockStub> {
        self.mocks.iter().find(|m| m.id == id)
    }

    /// The environment every other one layers over.
    pub fn global_environment(&self) -> Option<&EnvironmentInfo> {
        self.environments.iter().find(|e| e.is_global())
    }

    pub fn folder(&self, kind: RequestKind, id: &str) -> Option<&FolderInfo> {
        self.folders.iter().find(|f| f.kind == kind && f.id == id)
    }

    /// The folders requests of `kind` are kept in. Each request type has its own. A deleted folder is not
    /// one something can be kept in.
    pub fn request_folder_rows(&self, kind: RequestKind) -> Vec<FolderRow<'_>> {
        self.folders
            .iter()
            .filter(|f| f.kind == kind && !f.deleted)
            .map(|f| FolderRow { id: &f.id, name: &f.name, parent_id: f.parent_id.as_deref() })
            .collect()
    }

    pub fn mock_folder_rows(&self) -> Vec<FolderRow<'_>> {
        self.mock_folders
            .iter()
            .filter(|f| !f.deleted)
            .map(|f| FolderRow { id: &f.id, name: &f.name, parent_id: f.parent_id.as_deref() })
            .collect()
    }
}

fn stub_target(kind: RequestKind, stub: &Value) -> String {
    match kind {
        RequestKind::Rest | RequestKind::WebSocket => text(stub, "url").unwrap_or_default(),
        RequestKind::GraphQl | RequestKind::Soap => text(stub, "endpointUrl").unwrap_or_default(),
        RequestKind::Grpc => {
            let address = text(stub, "serverAddress").unwrap_or_default();
            let method = [text(stub, "serviceName"), text(stub, "methodName")]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            if method.is_empty() { address } else { format!("{address}/{method}") }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "port": 80, "forwardProxyPort": 8080,
            "activeWorkspaceId": "w1",
            "workspaces": [{"id": "w1", "name": "Work"}, {"id": "w2", "name": "Home"}],
            "activeEnvironmentId": "e1",
            "environments": [
                {"id": "__global__", "name": "Global", "variables": []},
                {"id": "e1", "name": "Staging", "variables": [{"id": "v", "key": "HOST", "value": "stg"}]},
            ],
            "mappings": [{"id": "m1", "domain": "api.localhost", "target": "localhost:3000", "enabled": true}],
            "requests": [
                {"id": "r1", "name": "List users", "method": "GET", "url": "{{HOST}}/users", "folderId": "f1"},
                {"id": "r2", "name": "Ping", "method": "POST", "url": "/ping"},
            ],
            "requestFolders": [{"id": "f1", "name": "Users", "parentId": null}],
            "wsConnections": [{"id": "s1", "name": "Echo", "url": "wss://echo.test", "folderId": "wf"}],
            "wsFolders": [{"id": "wf", "name": "Live", "parentId": ""}],
            "grpcRequests": [{"id": "g1", "name": "Say hi", "serverAddress": "localhost:50051",
                              "serviceName": "Greeter", "methodName": "SayHello"}],
            "graphqlRequests": [{"id": "q1", "name": "Me", "endpointUrl": "https://gql.test"}],
        })
    }

    #[test]
    fn a_snapshot_reads_every_request_type_into_one_list() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        let kinds: Vec<_> = c.requests.iter().map(|r| (r.kind, r.id.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (RequestKind::Rest, "r1"),
                (RequestKind::Rest, "r2"),
                (RequestKind::GraphQl, "q1"),
                (RequestKind::Grpc, "g1"),
                (RequestKind::WebSocket, "s1"),
            ]
        );
    }

    #[test]
    fn a_websocket_is_listed_like_any_other_request() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        let ws = c.request(RequestKind::WebSocket, "s1").unwrap();
        assert_eq!(ws.name, "Echo");
        assert_eq!(ws.target, "wss://echo.test");
        assert_eq!(ws.folder_id.as_deref(), Some("wf"));
        assert_eq!(ws.method, None);
    }

    #[test]
    fn only_rest_requests_carry_a_method_and_grpc_targets_name_the_call() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        assert_eq!(c.request(RequestKind::Rest, "r1").unwrap().method.as_deref(), Some("GET"));
        assert_eq!(c.request(RequestKind::Grpc, "g1").unwrap().method, None);
        assert_eq!(c.request(RequestKind::Grpc, "g1").unwrap().target, "localhost:50051/Greeter/SayHello");
    }

    #[test]
    fn empty_ids_for_root_level_items_read_as_none() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        assert_eq!(c.request(RequestKind::Rest, "r2").unwrap().folder_id, None);
        assert_eq!(c.folder(RequestKind::Rest, "f1").unwrap().parent_id, None);
        assert_eq!(c.folder(RequestKind::WebSocket, "wf").unwrap().parent_id, None);
    }

    #[test]
    fn the_global_environment_is_not_something_to_select() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        let names: Vec<_> = c.selectable_environments().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Staging"]);
        assert_eq!(c.active_environment().unwrap().variables[0].key, "HOST");
    }

    #[test]
    fn a_config_without_an_active_workspace_is_refused() {
        assert!(ConfigSnapshot::from_value(&json!({})).is_err());
    }

    #[test]
    fn missing_arrays_are_empty_rather_than_errors() {
        let c = ConfigSnapshot::from_value(&json!({"activeWorkspaceId": "w1"})).unwrap();
        assert!(c.requests.is_empty() && c.folders.is_empty() && c.mappings.is_empty());
    }
}
