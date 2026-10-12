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
    /// The organization the workspace belongs to, when it is one's and not its user's own.
    pub organization: Option<OrganizationInfo>,
}

/// The organization a workspace belongs to, as the Bifurc Console last said. Its usage analytics are decided
/// there, by its owner: nothing on this device turns them on or off.
#[derive(Debug, Clone, PartialEq)]
pub struct OrganizationInfo {
    pub name: String,
    /// What is done in the workspace is reported to the organization.
    pub analytics: bool,
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
    /// The repository is someone else's and the workspace's account may only read it: what the remote gets
    /// comes in, and what is changed here stays on this device.
    pub read_only: bool,
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
    /// The version of Bifurc it needs, when a newer one than this last saved it. It is listed and nothing
    /// else: the engine does not read or change it until this install is updated.
    pub locked: Option<String>,
}

/// A proxy rule that is deleted here, with the deletion waiting to be confirmed or reverted.
#[derive(Debug, Clone, PartialEq)]
pub struct DeletedRule {
    pub id: String,
    pub name: String,
    pub pattern: String,
}

/// A proxy rule that a newer Bifurc than this one last saved: it is listed and nothing else.
#[derive(Debug, Clone, PartialEq)]
pub struct LockedRule {
    pub id: String,
    pub name: String,
    /// The version of Bifurc it needs.
    pub version: String,
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
    /// The engine looks for a newer version once a day.
    pub update_check_enabled: bool,
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

/// A flow as it is listed. The flow itself is loaded by id (`entity.load`) when it is opened.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowInfo {
    pub id: String,
    pub name: String,
    pub folder_id: Option<String>,
    /// Deleted here; the deletion is waiting to be confirmed or reverted.
    pub deleted: bool,
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
    /// The version of Bifurc it needs, when a newer one than this last saved it. It is listed and nothing
    /// else: the engine does not read or change it until this install is updated.
    pub locked: Option<String>,
}

/// A folder of the request tree. It holds requests of any type.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderInfo {
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
    pub flows: Vec<FlowInfo>,
    pub flow_folders: Vec<PlainFolder>,
    pub mappings: Vec<MappingInfo>,
    pub requests: Vec<RequestStub>,
    pub folders: Vec<FolderInfo>,
    pub mocks: Vec<MockStub>,
    pub mock_folders: Vec<PlainFolder>,
    pub proxy_rule_count: usize,
    /// The proxy rules that are deleted here and still listed. The live ones are read with `entity.list`.
    pub deleted_rules: Vec<DeletedRule>,
    /// The proxy rules a newer Bifurc last saved. `entity.list` does not return them either.
    pub locked_rules: Vec<LockedRule>,
    pub settings: EngineSettings,
}

/// The array of the request tree's folders in `config.get`.
const REQUEST_FOLDERS_KEY: &str = "requestFolders";

/// The `kind` of `folder.add` / `folder.rename` / `folder.delete` for a folder of the request tree.
pub const REQUEST_FOLDER_KIND: &str = "request";

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

/// The version of Bifurc something needs, when the engine marks it as last saved by a newer one than this.
fn locked_to(value: &Value) -> Option<String> {
    value.get("locked").and_then(Value::as_bool).unwrap_or(false).then(|| text(value, "requiredVersion")).flatten()
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
                        read_only: s.get("access").and_then(Value::as_str) == Some("read"),
                    })
                });
                let account = w.get("account").and_then(AccountInfo::from_value);
                let organization = w.get("organization").filter(|o| o.is_object()).map(|o| OrganizationInfo {
                    name: text(o, "name").unwrap_or_default(),
                    analytics: o.get("analytics").and_then(Value::as_bool).unwrap_or(false),
                });
                Some(WorkspaceInfo { id: text(w, "id")?, name: text(w, "name")?, account, sync, organization })
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

        let flows = array(config, "flows")
            .iter()
            .filter_map(|f| {
                Some(FlowInfo {
                    id: text(f, "id")?,
                    name: text(f, "name").unwrap_or_default(),
                    folder_id: non_empty(f, "folderId"),
                    deleted: is_deleted(f),
                })
            })
            .collect();
        let plain_folders = |key: &str| -> Vec<PlainFolder> {
            array(config, key)
                .iter()
                .filter_map(|f| {
                    Some(PlainFolder {
                        id: text(f, "id")?,
                        name: text(f, "name")?,
                        parent_id: non_empty(f, "parentId"),
                        deleted: is_deleted(f),
                    })
                })
                .collect()
        };

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
                    locked: locked_to(stub),
                });
            }
        }
        // One tree of folders for every request type.
        for folder in array(config, REQUEST_FOLDERS_KEY) {
            let (Some(id), Some(name)) = (text(folder, "id"), text(folder, "name")) else { continue };
            let parent_id = non_empty(folder, "parentId");
            folders.push(FolderInfo { id, name, parent_id, deleted: is_deleted(folder) });
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
                    locked: locked_to(m),
                })
            })
            .collect();
        let mock_folders = plain_folders("mockFolders");

        Ok(Self {
            workspaces,
            active_workspace_id,
            active_environment_id: non_empty(config, "activeEnvironmentId"),
            environments,
            flows,
            flow_folders: plain_folders("flowFolders"),
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
            locked_rules: array(config, "proxyRules")
                .iter()
                .filter_map(|r| {
                    Some(LockedRule { version: locked_to(r)?, id: text(r, "id")?, name: text(r, "name").unwrap_or_default() })
                })
                .collect(),
            settings: EngineSettings {
                proxy_port: port("port"),
                forward_proxy_port: port("forwardProxyPort"),
                companion_port: port("companionPort"),
                minimize_to_tray: flag("minimizeToTray"),
                tls_enabled: flag("tlsEnabled"),
                telemetry_enabled: config.get("telemetryEnabled").and_then(Value::as_bool).unwrap_or(true),
                update_check_enabled: config.get("updateCheckEnabled").and_then(Value::as_bool).unwrap_or(true),
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

    /// What `{{key}}` resolves to, merged the way the engine does it (`merge_env_vars`): the global
    /// variables, each replaced in place by the active environment's variable of the same key, then the
    /// environment's own. Each comes with the environment its value is from.
    pub fn resolved_variables(&self) -> Vec<(&EnvVariable, &EnvironmentInfo)> {
        let mut resolved: Vec<(&EnvVariable, &EnvironmentInfo)> = Vec::new();
        for env in [self.global_environment(), self.active_environment()].into_iter().flatten() {
            for variable in &env.variables {
                match resolved.iter_mut().find(|(known, _)| known.key == variable.key) {
                    Some(existing) => *existing = (variable, env),
                    None => resolved.push((variable, env)),
                }
            }
        }
        resolved
    }

    pub fn folder(&self, id: &str) -> Option<&FolderInfo> {
        self.folders.iter().find(|f| f.id == id)
    }

    /// The folders requests are kept in, whatever their type. A deleted folder is not one something can be
    /// kept in.
    pub fn request_folder_rows(&self) -> Vec<FolderRow<'_>> {
        self.folders
            .iter()
            .filter(|f| !f.deleted)
            .map(|f| FolderRow { id: &f.id, name: &f.name, parent_id: f.parent_id.as_deref() })
            .collect()
    }

    pub fn flow_folder_rows(&self) -> Vec<FolderRow<'_>> {
        self.flow_folders
            .iter()
            .filter(|f| !f.deleted)
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
        RequestKind::Rest | RequestKind::WebSocket | RequestKind::Sse | RequestKind::SocketIo | RequestKind::Mcp => {
            text(stub, "url").unwrap_or_default()
        }
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
            "requestFolders": [
                {"id": "f1", "name": "Users", "parentId": null},
                {"id": "wf", "name": "Live", "parentId": ""},
                {"id": "ef", "name": "Streams", "parentId": "wf"},
                {"id": "if", "name": "Realtime", "parentId": ""},
                {"id": "mf", "name": "Servers", "parentId": ""},
            ],
            "wsConnections": [{"id": "s1", "name": "Echo", "url": "wss://echo.test", "folderId": "wf"}],
            "grpcRequests": [{"id": "g1", "name": "Say hi", "serverAddress": "localhost:50051",
                              "serviceName": "Greeter", "methodName": "SayHello"}],
            "graphqlRequests": [{"id": "q1", "name": "Me", "endpointUrl": "https://gql.test"}],
            "sseRequests": [{"id": "e1", "name": "Feed", "url": "https://sse.test/events", "method": "GET", "folderId": "ef"}],
            "socketioRequests": [{"id": "i1", "name": "Chat", "url": "https://io.test", "namespace": "/chat", "folderId": "if"}],
            "mcpRequests": [{"id": "m1", "name": "Tools", "transport": "stdio", "url": "npx my-server", "folderId": "mf"}],
        })
    }

    #[test]
    fn the_active_environment_overrides_global_variables_in_place_and_adds_its_own() {
        let mut config = sample();
        config["environments"][0]["variables"] =
            json!([{"id": "a", "key": "HOST", "value": "prod"}, {"id": "b", "key": "TOKEN", "value": "t"}]);
        config["environments"][1]["variables"] =
            json!([{"id": "c", "key": "PORT", "value": "81"}, {"id": "v", "key": "HOST", "value": "stg"}]);
        let c = ConfigSnapshot::from_value(&config).unwrap();
        let rows: Vec<_> =
            c.resolved_variables().into_iter().map(|(v, env)| (v.key.as_str(), v.value.as_str(), env.name.as_str())).collect();
        assert_eq!(rows, vec![("HOST", "stg", "Staging"), ("TOKEN", "t", "Global"), ("PORT", "81", "Staging")]);
    }

    #[test]
    fn what_a_newer_bifurc_saved_is_read_as_locked_to_its_version() {
        let mut config = sample();
        config["requests"][1]["locked"] = json!(true);
        config["requests"][1]["requiredVersion"] = json!("9.0.0");
        config["mocks"] = json!([
            {"id": "k1", "name": "Open", "method": "GET", "urlPattern": "/a"},
            {"id": "k2", "name": "Newer", "locked": true, "requiredVersion": "9.0.0"},
        ]);
        config["proxyRules"] = json!([
            {"id": "p1", "name": "Open", "pattern": "/a"},
            {"id": "p2", "name": "Newer", "locked": true, "requiredVersion": "9.0.0"},
        ]);
        let c = ConfigSnapshot::from_value(&config).unwrap();
        assert_eq!(c.request(RequestKind::Rest, "r1").unwrap().locked, None);
        assert_eq!(c.request(RequestKind::Rest, "r2").unwrap().locked.as_deref(), Some("9.0.0"));
        assert_eq!(c.mock("k1").unwrap().locked, None);
        assert_eq!(c.mock("k2").unwrap().locked.as_deref(), Some("9.0.0"));
        let rule = LockedRule { id: "p2".into(), name: "Newer".into(), version: "9.0.0".into() };
        assert_eq!(c.locked_rules, vec![rule]);
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
                (RequestKind::Sse, "e1"),
                (RequestKind::SocketIo, "i1"),
                (RequestKind::Mcp, "m1"),
            ]
        );
    }

    #[test]
    fn an_mcp_request_is_listed_like_any_other_request_in_the_request_folders() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        let tools = c.request(RequestKind::Mcp, "m1").unwrap();
        assert_eq!(tools.name, "Tools");
        // Its target is the command that starts the server, or the server's address.
        assert_eq!(tools.target, "npx my-server");
        assert_eq!(tools.folder_id.as_deref(), Some("mf"));
        assert_eq!(c.folder("mf").unwrap().parent_id, None);
    }

    #[test]
    fn a_socketio_request_is_listed_like_any_other_request_in_the_request_folders() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        let chat = c.request(RequestKind::SocketIo, "i1").unwrap();
        assert_eq!(chat.name, "Chat");
        assert_eq!(chat.target, "https://io.test");
        assert_eq!(chat.folder_id.as_deref(), Some("if"));
        assert_eq!(c.folder("if").unwrap().parent_id, None);
    }

    #[test]
    fn an_sse_request_is_listed_like_any_other_request_in_the_request_folders() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        let sse = c.request(RequestKind::Sse, "e1").unwrap();
        assert_eq!(sse.name, "Feed");
        assert_eq!(sse.target, "https://sse.test/events");
        assert_eq!(sse.folder_id.as_deref(), Some("ef"));
        // Its folder is in the folder a WebSocket is in: the folders are every type's.
        assert_eq!(c.folder("ef").unwrap().parent_id.as_deref(), Some("wf"));
        assert_eq!(c.request_folder_rows().len(), 5);
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
        assert_eq!(c.folder("f1").unwrap().parent_id, None);
        assert_eq!(c.folder("wf").unwrap().parent_id, None);
    }

    #[test]
    fn the_global_environment_is_not_something_to_select() {
        let c = ConfigSnapshot::from_value(&sample()).unwrap();
        let names: Vec<_> = c.selectable_environments().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Staging"]);
        assert_eq!(c.active_environment().unwrap().variables[0].key, "HOST");
    }

    #[test]
    fn a_workspace_is_read_only_when_its_account_may_only_read_the_repository() {
        let workspace = |sync: Value| {
            let config = json!({ "activeWorkspaceId": "w1", "workspaces": [{ "id": "w1", "name": "W", "syncConfig": sync }] });
            ConfigSnapshot::from_value(&config).unwrap().workspaces[0].clone()
        };
        let read_only = |sync: Value| workspace(sync).sync.map(|sync| sync.read_only);
        assert_eq!(read_only(json!({ "remote": "r", "repo": "octo/team", "access": "read" })), Some(true));
        assert_eq!(read_only(json!({ "remote": "r", "repo": "octo/team", "access": "write" })), Some(false));
        assert_eq!(read_only(json!({ "remote": "r", "repo": "octo/team", "access": "owner" })), Some(false));
        // A remote assigned before access was recorded is its owner's.
        assert_eq!(read_only(json!({ "remote": "r", "repo": "octo/team" })), Some(false));
        assert_eq!(read_only(Value::Null), None);
    }

    #[test]
    fn a_workspace_of_an_organization_says_whose_it_is_and_whether_it_is_reported() {
        let workspace = |organization: Value| {
            let config = json!({
                "activeWorkspaceId": "w1",
                "workspaces": [{ "id": "w1", "name": "W", "organization": organization }],
            });
            ConfigSnapshot::from_value(&config).unwrap().workspaces[0].organization.clone()
        };
        let acme = |analytics: bool| Some(OrganizationInfo { name: "Acme".into(), analytics });
        assert_eq!(workspace(json!({ "name": "Acme", "analytics": true })), acme(true));
        assert_eq!(workspace(json!({ "name": "Acme", "analytics": false })), acme(false));
        // The user's own workspace, and what an engine from before organizations answers.
        assert_eq!(workspace(Value::Null), None);
        let config = json!({ "activeWorkspaceId": "w1", "workspaces": [{ "id": "w1", "name": "W" }] });
        assert_eq!(ConfigSnapshot::from_value(&config).unwrap().workspaces[0].organization, None);
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
