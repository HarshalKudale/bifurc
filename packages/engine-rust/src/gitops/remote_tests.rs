//! Tests of sign-in, import, publish and the owner and access rules, against a stand-in for GitHub whose repositories are
//! bare repositories in a temporary directory, and for the console that says who was given which workspace.
//! Skipped when git is missing.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::{
    auto_sync, console, get_sync_config, organization, get_workspace_git_state, get_workspace_sync_status,
    github::{Endpoints, set_test_endpoints},
    remote::forget_access,
};
use crate::{
    features::set_test_team_sync,
    kernel::{CommandContext, CommandRegistry},
    register_all::register_all_engine_commands,
    testing::{ScratchRoot, scratch_data_root},
    workspace::{
        accounts::{Account, account_id, find_account, save_account},
        fs::{write_entity, ws_dir},
        git_store::{check_git_installed, git_in, init_workspace_repo},
        settings::load_settings,
    },
};

const HOST: &str = "127.0.0.1";

/// The people the stand-in knows, by token.
fn user_of(token: &str) -> Option<(u64, &'static str)> {
    match token {
        "tok-octo" => Some((42, "octo")),
        "tok-eve" => Some((7, "eve")),
        _ => None,
    }
}

#[derive(Default)]
struct Hub {
    /// `owner/name` -> (owner id, bare repository).
    repos: HashMap<String, (u64, PathBuf)>,
    /// `owner/name` -> the people it is shared with, each with whether they may push.
    collaborators: HashMap<String, HashMap<u64, bool>>,
    root: PathBuf,
    /// Whether the user has typed the device code yet.
    granted: bool,
    /// The console: the repositories that are an organization's workspace, by id.
    managed: std::collections::HashSet<u64>,
    /// The console: (repository id, user id) -> `read`, `write`, or `seatInactive` for someone who was given
    /// the workspace and has no seat.
    grants: HashMap<(u64, u64), &'static str>,
    /// The console does not answer.
    console_down: bool,
    /// The console: the organization the managed repositories belong to has usage analytics on.
    analytics: bool,
    /// `owner/name` of the repositories that do not carry the workspace topic.
    untagged: std::collections::HashSet<String>,
}

impl Hub {
    fn shared_with(&self, full_name: &str, user: u64) -> Option<bool> {
        self.collaborators.get(full_name).and_then(|people| people.get(&user)).copied()
    }

    /// The repository as `viewer` is shown it: with what they may do with it.
    fn repo_json(&self, full_name: &str, viewer: u64) -> Option<Value> {
        let (owner, dir) = self.repos.get(full_name)?;
        let push = *owner == viewer || self.shared_with(full_name, viewer) == Some(true);
        let mut repo = repo_json(full_name, *owner, dir, push);
        if self.untagged.contains(full_name) {
            repo["topics"] = json!([]);
        }
        Some(repo)
    }
}

/// The id the stand-in gives a repository: the same every time for a name.
fn repo_id(full_name: &str) -> u64 {
    full_name.bytes().fold(7u64, |id, byte| id.wrapping_mul(31).wrapping_add(u64::from(byte))) % 1_000_000 + 1
}

fn repo_json(full_name: &str, owner_id: u64, path: &Path, push: bool) -> Value {
    json!({
        "id": repo_id(full_name), "full_name": full_name, "name": full_name.rsplit('/').next().unwrap(),
        "owner": { "id": owner_id, "login": full_name.split('/').next().unwrap() },
        "permissions": { "pull": true, "push": push },
        "private": true, "description": "", "clone_url": path.to_string_lossy(), "html_url": "",
        "default_branch": "main", "updated_at": "2026-01-01T00:00:00Z", "topics": [super::github::WORKSPACE_TOPIC],
    })
}

async fn answer(hub: &Arc<Mutex<Hub>>, method: &str, path: &str, token: &str, body: &str) -> (u16, Value) {
    let user = user_of(token);
    match (method, path) {
        ("POST", "/login/device/code") => (
            200,
            json!({ "device_code": "dev-1", "user_code": "ABCD-1234", "verification_uri": "http://hub/device",
                    "expires_in": 60, "interval": 1 }),
        ),
        ("POST", "/login/oauth/access_token") => match hub.lock().granted {
            true => (200, json!({ "access_token": "tok-octo" })),
            false => (200, json!({ "error": "authorization_pending" })),
        },
        // The console's one question. It is asked without a token.
        ("POST", "/api/v1/access/check") => {
            let asked: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let (Some(user_id), Some(repo)) = (asked["githubUserId"].as_u64(), asked["repoId"].as_u64()) else {
                return (400, json!({ "error": { "code": "invalid" } }));
            };
            let hub = hub.lock();
            if hub.console_down {
                return (503, json!({}));
            }
            if !hub.managed.contains(&repo) {
                let message = "Not a workspace of an organization.";
                return (200, json!({ "allowed": false, "reason": "unmanaged", "message": message }));
            }
            match hub.grants.get(&(repo, user_id)) {
                Some(&"seatInactive") => {
                    (200, json!({ "allowed": false, "reason": "seatInactive", "message": "Your seat is not active." }))
                }
                Some(permission) => (200, json!({ "allowed": true, "permission": permission })),
                None => (200, json!({ "allowed": false, "reason": "notGranted", "message": "Ask the owner to add you." })),
            }
        }
        // The console's other question: whose the workspace is. The owner of the repository is the owner of
        // the organization; anyone else has to have been given the workspace.
        ("POST", "/api/v1/workspaces/policy") => {
            let asked: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let (Some(user_id), Some(repo)) = (asked["githubUserId"].as_u64(), asked["repoId"].as_u64()) else {
                return (400, json!({ "error": { "code": "invalid" } }));
            };
            let hub = hub.lock();
            if hub.console_down {
                return (503, json!({}));
            }
            let owner = hub.repos.iter().any(|(name, (owner, _))| repo_id(name) == repo && *owner == user_id);
            if !hub.managed.contains(&repo) || !(owner || hub.grants.contains_key(&(repo, user_id))) {
                return (200, json!({ "managed": false }));
            }
            (
                200,
                json!({
                    "managed": true,
                    "organization": { "publicId": "pub_acme", "name": "Acme" },
                    "workspace": { "id": format!("ws_{repo}") },
                    "analytics": { "enabled": hub.analytics },
                }),
            )
        }
        ("GET", "/user") => match user {
            Some((id, login)) => (200, json!({ "id": id, "login": login, "name": "", "avatar_url": "" })),
            None => (401, json!({ "message": "Bad credentials" })),
        },
        ("POST", "/user/repos") => {
            let Some((id, login)) = user else { return (401, json!({ "message": "Bad credentials" })) };
            let wanted: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let full_name = format!("{login}/{}", wanted["name"].as_str().unwrap_or(""));
            if hub.lock().repos.contains_key(&full_name) {
                return (422, json!({ "message": "name already exists on this account" }));
            }
            let dir = hub.lock().root.join(format!("{}.git", full_name.replace('/', "-")));
            std::fs::create_dir_all(&dir).unwrap();
            git_in(&dir, &["init", "--bare"]).await.unwrap();
            hub.lock().repos.insert(full_name.clone(), (id, dir.clone()));
            (201, repo_json(&full_name, id, &dir, true))
        }
        ("GET", path) if path.starts_with("/user/repos") => {
            let Some((id, _)) = user else { return (401, json!({ "message": "Bad credentials" })) };
            // What is listed is what was asked for: the person's own, and what they were given access to.
            let shared = path.contains("collaborator");
            let hub = hub.lock();
            let mut repos: Vec<Value> = hub
                .repos
                .iter()
                .filter(|(name, (owner, _))| *owner == id || (shared && hub.shared_with(name, id).is_some()))
                .filter_map(|(name, _)| hub.repo_json(name, id))
                .collect();
            repos.sort_by_key(|repo| repo["full_name"].as_str().unwrap().to_string());
            (200, Value::Array(repos))
        }
        ("GET", path) if path.contains("/contents/") => {
            let (repo, file) = path["/repos/".len()..].split_once("/contents/").unwrap();
            let Some(bare) = hub.lock().repos.get(repo).map(|(_, dir)| dir.clone()).filter(|_| user.is_some()) else {
                return (404, json!({ "message": "Not Found" }));
            };
            match git_in(&bare, &["show", &format!("main:{file}")]).await {
                Ok(text) => (200, serde_json::from_str(&text).unwrap_or(Value::Null)),
                Err(_) => (404, json!({ "message": "Not Found" })),
            }
        }
        ("GET", path) if path.starts_with("/repos/") => {
            match user.and_then(|(id, _)| hub.lock().repo_json(&path["/repos/".len()..], id)) {
                Some(repo) => (200, repo),
                None => (404, json!({ "message": "Not Found" })),
            }
        }
        ("PUT", _) => (200, json!({})),
        _ => (404, json!({ "message": "Not Found" })),
    }
}

/// Start the stand-in and point the engine at it until the returned guard is dropped.
struct HubGuard {
    hub: Arc<Mutex<Hub>>,
    _dir: tempfile::TempDir,
}

impl Drop for HubGuard {
    fn drop(&mut self) {
        set_test_endpoints(None);
        console::set_test_url(None);
    }
}

impl HubGuard {
    /// In the console: make the repository an organization's workspace and give it to `user` (`read`, `write`,
    /// or `seatInactive`), or with `None` take it from them.
    fn grant(&self, full_name: &str, user: u64, permission: Option<&'static str>) {
        let mut hub = self.hub.lock();
        hub.managed.insert(repo_id(full_name));
        match permission {
            Some(permission) => hub.grants.insert((repo_id(full_name), user), permission),
            None => hub.grants.remove(&(repo_id(full_name), user)),
        };
    }
}

async fn start_hub() -> HubGuard {
    let dir = tempfile::tempdir().unwrap();
    let hub = Arc::new(Mutex::new(Hub { root: dir.path().to_path_buf(), ..Hub::default() }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{HOST}:{}", listener.local_addr().unwrap().port());
    set_test_endpoints(Some(Endpoints { web: base.clone(), api: base.clone() }));
    console::set_test_url(Some(base));
    let state = hub.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let state = state.clone();
            tokio::spawn(async move {
                let mut received = Vec::new();
                let mut buf = [0u8; 4096];
                let (head_end, length) = loop {
                    let Ok(n) = socket.read(&mut buf).await else { return };
                    if n == 0 {
                        return;
                    }
                    received.extend_from_slice(&buf[..n]);
                    if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&received[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while received.len() < head_end + length {
                    let Ok(n) = socket.read(&mut buf).await else { return };
                    if n == 0 {
                        break;
                    }
                    received.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&received[..head_end]).into_owned();
                let body = String::from_utf8_lossy(&received[head_end..]).into_owned();
                let mut request_line = head.lines().next().unwrap_or("").split(' ');
                let (method, path) = (request_line.next().unwrap_or(""), request_line.next().unwrap_or(""));
                let token = head
                    .lines()
                    .find(|line| line.to_lowercase().starts_with("authorization:"))
                    .and_then(|line| line.rsplit(' ').next())
                    .unwrap_or("");
                let (status, json) = answer(&state, method, path, token, &body).await;
                let text = json.to_string();
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    HubGuard { hub, _dir: dir }
}

struct Engine {
    registry: Arc<CommandRegistry>,
}

impl Engine {
    fn new() -> Engine {
        let mut registry = CommandRegistry::new();
        register_all_engine_commands(&mut registry);
        Engine { registry: Arc::new(registry) }
    }

    async fn call(&self, action: &str, payload: Value) -> Value {
        let ctx = CommandContext::internal(self.registry.clone());
        self.registry.invoke(action, payload, ctx).await.unwrap_or_else(|e| panic!("{action} failed: {e:?}"))
    }
}

/// Team sync switched for a test, and back to what the build ships when the test is over. Made after the
/// scratch root, so that it is undone while the test still has its turn.
struct TeamSync;

impl TeamSync {
    fn set(on: bool) -> TeamSync {
        set_test_team_sync(Some(on));
        forget_access();
        TeamSync
    }
}

impl Drop for TeamSync {
    fn drop(&mut self) {
        set_test_team_sync(None);
    }
}

fn sign_in(user_id: u64, login: &str) -> String {
    let id = account_id(HOST, user_id);
    save_account(&Account {
        id: id.clone(),
        host: HOST.into(),
        user_id,
        login: login.into(),
        name: String::new(),
        avatar_url: String::new(),
        token: Some(format!("tok-{login}")),
    })
    .unwrap();
    id
}

async fn run(dir: &Path, args: &[&str]) -> String {
    git_in(dir, args).await.unwrap_or_else(|e| panic!("git {args:?} failed: {e}"))
}

/// What `workspace.json` holds for a workspace made now.
fn workspace_file(ws_id: &str) -> Option<Value> {
    Some(json!({ "kind": "bifurc-workspace", "version": 1, "id": ws_id, "name": "Remote", "createdAt": 1 }))
}

/// Put a repository into the stand-in as `full_name`, the way another device would have pushed it, with
/// `workspace_file` as its `workspace.json` (or without one).
async fn seed_remote(guard: &HubGuard, full_name: &str, repo_owner: u64, workspace_file: Option<Value>) {
    let root = guard.hub.lock().root.clone();
    let bare = root.join(format!("{}.git", full_name.replace('/', "-")));
    std::fs::create_dir_all(&bare).unwrap();
    run(&bare, &["init", "--bare"]).await;
    let seed = root.join(format!("seed-{}", full_name.replace('/', "-")));
    std::fs::create_dir_all(seed.join("requests")).unwrap();
    run(&seed, &["init"]).await;
    run(&seed, &["symbolic-ref", "HEAD", "refs/heads/main"]).await;
    run(&seed, &["config", "user.email", "laptop@bifurc"]).await;
    run(&seed, &["config", "user.name", "Seed"]).await;
    if let Some(workspace_file) = workspace_file {
        std::fs::write(seed.join("workspace.json"), workspace_file.to_string()).unwrap();
    }
    std::fs::write(seed.join("requests/r1.json"), json!({ "id": "r1", "name": "First" }).to_string()).unwrap();
    run(&seed, &["add", "-A"]).await;
    run(&seed, &["commit", "-m", "create workspace Remote"]).await;
    run(&seed, &["push", &bare.to_string_lossy(), "HEAD:main"]).await;
    guard.hub.lock().repos.insert(full_name.to_string(), (repo_owner, bare));
}

fn request(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "method": "GET", "url": "http://localhost/x" })
}

/// A data root with no workspace in it, as a first run finds it.
fn empty_root() -> ScratchRoot {
    let root = scratch_data_root();
    let mut settings = load_settings();
    settings.set("workspaces", json!([]));
    settings.set("activeWorkspaceId", json!(""));
    crate::workspace::settings::save_settings(&settings).unwrap();
    root
}

#[tokio::test]
async fn a_first_run_is_onboarded_only_after_the_privacy_policy_is_accepted() {
    let _root = empty_root();
    let _guard = start_hub().await;
    let engine = Engine::new();

    let setup = engine.call("setup.getState", json!({})).await;
    assert_eq!((setup["privacyAccepted"].as_bool(), setup["onboarded"].as_bool()), (Some(false), Some(false)));
    assert!(setup["privacyPolicyUrl"].as_str().is_some_and(|url| url.starts_with("https://")));
    assert_eq!(setup["termsAccepted"], false);
    assert!(setup["termsOfServiceUrl"].as_str().is_some_and(|url| url.starts_with("https://")));

    assert_eq!(engine.call("setup.complete", json!({})).await["ok"], false);
    assert_eq!(engine.call("setup.getState", json!({})).await["onboarded"], false);

    // Each of the two has to be accepted: one without the other does not finish the setup.
    assert_eq!(engine.call("setup.acceptTerms", json!({})).await["ok"], true);
    assert_eq!(engine.call("setup.complete", json!({})).await["ok"], false);
    assert_eq!(engine.call("setup.acceptPrivacy", json!({})).await["ok"], true);
    assert_eq!(engine.call("setup.complete", json!({})).await["ok"], true);
    let setup = engine.call("setup.getState", json!({})).await;
    assert_eq!((setup["privacyAccepted"].as_bool(), setup["onboarded"].as_bool()), (Some(true), Some(true)));
    assert_eq!(setup["termsAccepted"], true);

    assert_eq!(engine.call("config.get", json!({})).await["telemetryEnabled"], true);
    engine.call("settings.update", json!({ "telemetryEnabled": false })).await;
    assert_eq!(engine.call("config.get", json!({})).await["telemetryEnabled"], false);
}

#[tokio::test]
async fn the_proxies_are_the_engines_to_start_only_once_the_servers_step_is_passed() {
    let _root = empty_root();
    let _guard = start_hub().await;
    let engine = Engine::new();
    assert_eq!(engine.call("setup.getState", json!({})).await["serversConfirmed"], false);
    assert!(!load_settings().servers_confirmed());

    // Two proxies on one port is refused, and nothing is recorded.
    let same = engine.call("setup.startServers", json!({ "port": 8123, "forwardProxyPort": 8123 })).await;
    assert_eq!(same["ok"], false);
    assert!(!load_settings().servers_confirmed());

    let started = engine.call("setup.startServers", json!({ "port": 8123 })).await;
    assert_eq!(started["ok"], true);
    // Ports from 1024 up need nobody's permission, and only Linux has one the engine can be given.
    assert_eq!(started["lowPortsDenied"], false);
    if !cfg!(target_os = "linux") {
        assert_eq!(engine.call("setup.allowLowPorts", json!({})).await["ok"], false);
    }
    let settings = load_settings();
    assert!(settings.servers_confirmed());
    assert_eq!((settings.port(), settings.forward_proxy_port()), (8123, 8080));
    let setup = engine.call("setup.getState", json!({})).await;
    assert_eq!((setup["serversConfirmed"].as_bool(), setup["onboarded"].as_bool()), (Some(true), Some(false)));
}

#[tokio::test]
async fn a_first_run_has_no_workspace_and_the_first_one_created_becomes_active() {
    if !check_git_installed().await {
        return;
    }
    let _root = empty_root();
    let guard = start_hub().await;
    let engine = Engine::new();

    let setup = engine.call("setup.getState", json!({})).await;
    assert_eq!(setup["gitInstalled"], true);
    assert_eq!(setup["needsWorkspace"], true);
    assert_eq!(setup["signInAvailable"], true);
    assert_eq!(setup["accounts"], json!([]));
    let config = engine.call("config.get", json!({})).await;
    assert_eq!(config["activeWorkspaceId"], "");
    assert_eq!(config["workspaces"], json!([]));

    // Signing in: a code to type, then the engine learns who it was.
    let login = engine.call("account.login.start", json!({})).await;
    assert_eq!(login["status"], "pending");
    assert_eq!(login["userCode"], "ABCD-1234");
    let login_id = login["loginId"].as_str().unwrap().to_string();
    guard.hub.lock().granted = true;
    let mut status = Value::Null;
    for _ in 0..100 {
        status = engine.call("account.login.status", json!({ "loginId": login_id })).await;
        if status["status"] != "pending" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(status["status"], "done", "{status}");
    assert_eq!(status["account"]["login"], "octo");
    assert!(status.to_string().find("tok-octo").is_none(), "a token never reaches a client");
    let account = status["account"]["id"].as_str().unwrap().to_string();
    assert_eq!(find_account(&account).unwrap().token.as_deref(), Some("tok-octo"));

    // The first workspace is created as that account, which makes the account its owner.
    let created = engine.call("workspace.add", json!({ "name": "Mine", "accountId": account })).await;
    assert_eq!(created["gitTracked"], true);
    let ws = created["id"].as_str().unwrap();
    assert_eq!(load_settings().active_workspace_id(), ws);
    let first = run(&ws_dir(ws), &["log", "--reverse", "--format=%ae|%s", "-1"]).await;
    assert_eq!(first.trim(), format!("42+octo@users.noreply.{HOST}|create workspace Mine"));
    // Its first commit holds the file that says the directory is a workspace.
    let info: Value = serde_json::from_str(&run(&ws_dir(ws), &["show", "HEAD:workspace.json"]).await).unwrap();
    assert_eq!((info["kind"].as_str(), info["id"].as_str()), (Some("bifurc-workspace"), Some(ws)));
    let state = engine.call("sync.getState", json!({ "workspaceId": ws })).await;
    assert_eq!(state["owner"]["login"], "octo");
    assert_eq!(state["account"]["login"], "octo");
    let config = engine.call("config.get", json!({})).await;
    assert_eq!(config["workspaces"][0]["account"]["login"], "octo");
    assert_eq!(engine.call("setup.getState", json!({})).await["needsWorkspace"], false);

    // A second workspace does not take over.
    let second = engine.call("workspace.add", json!({ "name": "Other" })).await;
    assert_eq!(load_settings().active_workspace_id(), ws);
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": second["id"] })).await["owner"], Value::Null);
}

#[tokio::test]
async fn publishing_makes_a_new_repository_and_then_syncs_as_the_account() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let guard = start_hub().await;
    let engine = Engine::new();
    let octo = sign_in(42, "octo");
    let eve = sign_in(7, "eve");

    // A workspace from before `workspace.json` said what it is.
    std::fs::write(ws_dir(ws).join("workspace.json"), json!({ "id": ws, "name": "Workspace 1", "createdAt": 1 }).to_string())
        .unwrap();
    init_workspace_repo(ws).await.unwrap();
    write_entity(ws, "requests", "r1", &request("r1", "First"), None).unwrap();
    engine.call("entity.publish", json!({ "workspaceId": ws, "paths": ["requests/r1.json"] })).await;
    assert_eq!(get_workspace_git_state(ws).await["hasRemote"], false);
    assert_eq!(engine.call("sync.push", json!({ "workspaceId": ws })).await["error"], "No remote configured");

    let published =
        engine.call("sync.publish", json!({ "workspaceId": ws, "accountId": octo, "repoName": "my-workspace" })).await;
    assert_eq!(published["ok"], true, "{published}");
    assert_eq!(published["repo"]["fullName"], "octo/my-workspace");
    let bare = guard.hub.lock().repos["octo/my-workspace"].1.clone();
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);
    let config = get_sync_config(ws).unwrap();
    assert_eq!(config["repo"], "octo/my-workspace");
    assert_eq!(config["branch"], "main");
    assert_eq!(load_settings().workspace(ws).unwrap()["accountId"], octo);

    // Its owner is the repository's owner, and what was pushed says it is a workspace.
    assert_eq!(config["owner"], json!({ "host": HOST, "userId": 42, "login": "octo" }));
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": ws })).await["owner"]["login"], "octo");
    let pushed: Value = serde_json::from_str(&run(&bare, &["show", "main:workspace.json"]).await).unwrap();
    assert_eq!((pushed["kind"].as_str(), pushed["id"].as_str()), (Some("bifurc-workspace"), Some(ws)));
    // The Global environment every workspace has went with it, so nothing is left unsynced on either side.
    let global: Value = serde_json::from_str(&run(&bare, &["show", "main:environments/__global__.json"]).await).unwrap();
    assert_eq!(global["id"], "__global__");
    let status = get_workspace_sync_status(ws).await;
    assert!(status.values().all(|state| state == "clean"), "{status:?}");
    // A published workspace is watched for what its remote gets; there is no switch for it. The watch is
    // stopped here so that the pulls below are the test's own.
    assert_eq!(auto_sync::poller_count(), 1);
    auto_sync::stop_auto_sync(ws);
    assert_eq!(
        get_workspace_git_state(ws).await,
        json!({ "hasRemote": true, "branch": "main", "ahead": false, "behind": false })
    );

    // The remote is assigned once.
    let again =
        engine.call("sync.publish", json!({ "workspaceId": ws, "accountId": octo, "repoName": "elsewhere" })).await;
    assert_eq!(again["ok"], false);
    assert!(again["error"].as_str().unwrap().contains("already has a remote"), "{again}");
    assert!(!guard.hub.lock().repos.contains_key("octo/elsewhere"));

    // A publish pushes by itself, as the workspace's account.
    write_entity(ws, "requests", "r2", &request("r2", "Second"), None).unwrap();
    let pushed = engine.call("entity.publish", json!({ "workspaceId": ws, "paths": ["requests/r2.json"] })).await;
    assert_eq!(pushed["ok"], true, "{pushed}");
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);
    assert_eq!(run(&ws_dir(ws), &["log", "-1", "--format=%ae"]).await.trim(), format!("42+octo@users.noreply.{HOST}"));
    assert!(run(&ws_dir(ws), &["log", "-1", "--format=%b"]).await.contains("actor: octo"));

    // A change made on another device comes in with a pull.
    let other = root.dir.path().join("other-device");
    run(root.dir.path(), &["clone", "--branch", "main", &bare.to_string_lossy(), &other.to_string_lossy()]).await;
    run(&other, &["config", "user.email", "o@example.com"]).await;
    run(&other, &["config", "user.name", "Other"]).await;
    std::fs::write(other.join("requests/r9.json"), serde_json::to_string_pretty(&request("r9", "Nine")).unwrap())
        .unwrap();
    // And a WebSocket in a folder in a folder: directories, and a file that says what it is.
    std::fs::create_dir_all(other.join("requests/Team/Live")).unwrap();
    let socket = json!({ "type": "websocket", "id": "s9", "name": "Feed", "url": "ws://a/feed" });
    std::fs::write(other.join("requests/Team/Live/s9.json"), serde_json::to_string_pretty(&socket).unwrap()).unwrap();
    run(&other, &["add", "-A"]).await;
    run(&other, &["commit", "-m", "create request Nine"]).await;
    run(&other, &["push", "origin", "HEAD:main"]).await;
    // The index of this device is not something the other one has, or sends.
    assert!(ws_dir(ws).join("requests/index.db").is_file() && !other.join("requests/index.db").exists());
    let pulled = engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    assert_eq!(pulled, json!({ "ok": true, "updated": true, "updatedIds": ["s9", "r9"] }));
    // What came in is listed at once, in the folders its directories are: the index read the tree again.
    let cfg = crate::workspace::config::load_config();
    let folder = |name: &str| cfg["requestFolders"].as_array().unwrap().iter().find(|f| f["name"] == name).cloned().unwrap();
    assert_eq!(folder("Live")["parentId"], folder("Team")["id"]);
    assert_eq!(cfg["wsConnections"][0]["name"], "Feed");
    assert_eq!(cfg["wsConnections"][0]["folderId"], folder("Live")["id"]);
    assert!(cfg["requests"].as_array().unwrap().iter().any(|r| r["id"] == "r9" && r["name"] == "Nine"));
    assert!(ws_dir(ws).join("requests/index.db").is_file(), "a pull leaves the index where it is");
    assert_eq!(engine.call("sync.pull", json!({ "workspaceId": ws })).await, json!({ "ok": true, "updated": false }));

    run(&ws_dir(ws), &["commit", "--allow-empty", "-m", "local only"]).await;
    assert_eq!(get_workspace_git_state(ws).await["ahead"], true);
    assert_eq!(engine.call("sync.push", json!({ "workspaceId": ws })).await, json!({ "ok": true }));
    assert_eq!(get_workspace_git_state(ws).await["ahead"], false);

    // What was changed here and not synced stays on top of what a pull brings, also in a file both changed.
    let local = serde_json::to_string_pretty(&request("r1", "Changed here")).unwrap();
    std::fs::write(ws_dir(ws).join("requests/r1.json"), &local).unwrap();
    write_entity(ws, "requests", "r3", &request("r3", "Only here"), None).unwrap();
    run(&other, &["pull", "origin", "main"]).await;
    std::fs::write(other.join("requests/r1.json"), serde_json::to_string_pretty(&request("r1", "Changed there")).unwrap())
        .unwrap();
    std::fs::write(other.join("requests/r9.json"), serde_json::to_string_pretty(&request("r9", "Nine again")).unwrap())
        .unwrap();
    run(&other, &["commit", "-am", "update requests"]).await;
    run(&other, &["push", "origin", "HEAD:main"]).await;
    let pulled = engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    assert_eq!(pulled["updated"], true, "{pulled}");
    assert_eq!(run(&ws_dir(ws), &["rev-parse", "HEAD"]).await, run(&bare, &["rev-parse", "main"]).await);
    assert_eq!(std::fs::read_to_string(ws_dir(ws).join("requests/r1.json")).unwrap().replace("\r\n", "\n"), local);
    assert!(run(&ws_dir(ws), &["show", "HEAD:requests/r1.json"]).await.contains("Changed there"));
    assert!(std::fs::read_to_string(ws_dir(ws).join("requests/r9.json")).unwrap().contains("Nine again"));
    assert!(run(&ws_dir(ws), &["stash", "list"]).await.trim().is_empty());
    crate::workspace::status_cache::invalidate_cache(ws);
    let status = get_workspace_sync_status(ws).await;
    assert_eq!((status["requests/r1.json"].as_str(), status["requests/r3.json"].as_str()), (Some("modified"), Some("new")));

    // A sync commits on top of what the remote has, even when the remote moved since the last pull.
    std::fs::write(other.join("requests/r9.json"), serde_json::to_string_pretty(&request("r9", "Nine, third")).unwrap())
        .unwrap();
    run(&other, &["commit", "-am", "update request Nine"]).await;
    run(&other, &["push", "origin", "HEAD:main"]).await;
    let synced = engine.call("entity.publish", json!({ "workspaceId": ws, "paths": ["requests/r1.json"] })).await;
    assert_eq!(synced["ok"], true, "{synced}");
    assert_eq!(run(&ws_dir(ws), &["rev-parse", "HEAD"]).await, run(&bare, &["rev-parse", "main"]).await);
    assert!(run(&bare, &["show", "main:requests/r1.json"]).await.contains("Changed here"));
    assert!(run(&bare, &["show", "main:requests/r9.json"]).await.contains("Nine, third"));

    // The account's listing says what its workspaces hold that the remote does not.
    let listed = engine.call("account.list", json!({})).await;
    let mine = listed["accounts"].as_array().unwrap().iter().find(|a| a["id"] == octo.as_str()).unwrap().clone();
    assert_eq!(
        mine["workspaces"],
        json!([{ "id": ws, "name": "Workspace 1", "repo": "octo/my-workspace", "changes": 1, "unpushed": false }])
    );

    // Signing out takes the account and its workspaces off the device; other workspaces stay.
    auto_sync::start_auto_sync(ws);
    let kept = engine.call("workspace.add", json!({ "name": "Kept" })).await;
    let out = engine.call("account.logout", json!({ "accountId": octo })).await;
    assert_eq!(out, json!({ "ok": true, "removedWorkspaces": [ws] }));
    assert_eq!(auto_sync::poller_count(), 0);
    assert!(find_account(&octo).is_none());
    let settings = load_settings();
    assert!(settings.workspace(ws).is_none());
    assert_eq!(settings.active_workspace_id(), kept["id"].as_str().unwrap());
    assert!(!ws_dir(ws).exists(), "the directory is set aside");
    let trash = crate::workspace::fs::data_root().join(".trash");
    assert_eq!(std::fs::read_dir(trash).unwrap().count(), 1);

    // A name that is taken is never pushed into, and another account cannot publish this workspace.
    sign_in(42, "octo");
    let second = engine.call("workspace.add", json!({ "name": "Second" })).await;
    let second = second["id"].as_str().unwrap();
    let taken = engine
        .call("sync.publish", json!({ "workspaceId": second, "accountId": octo, "repoName": "my-workspace" }))
        .await;
    assert_eq!(taken["ok"], false);
    assert!(taken["error"].as_str().unwrap().contains("already exists"), "{taken}");
    assert!(get_sync_config(second).is_none());

    let owned = engine.call("workspace.add", json!({ "name": "Owned", "accountId": octo })).await;
    let stolen = engine
        .call("sync.publish", json!({ "workspaceId": owned["id"], "accountId": eve, "repoName": "stolen" }))
        .await;
    assert_eq!(stolen["ok"], false);
    assert!(stolen["error"].as_str().unwrap().contains("another account"), "{stolen}");
    assert!(!guard.hub.lock().repos.contains_key("eve/stolen"));
}

#[tokio::test]
async fn without_team_sync_only_the_owner_imports_a_workspace_and_nothing_local_is_replaced() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let guard = start_hub().await;
    let _owner_only = TeamSync::set(false);
    let engine = Engine::new();
    let octo = sign_in(42, "octo");
    let eve = sign_in(7, "eve");

    seed_remote(&guard, "octo/team", 42, workspace_file("remote-ws")).await;
    seed_remote(&guard, "eve/own", 7, workspace_file("eve-ws")).await;
    // A repository whose workspace has the id of one that is already here.
    seed_remote(&guard, "octo/clash", 42, workspace_file(root.ws_id)).await;
    // One from before `workspace.json` said what it is.
    seed_remote(&guard, "octo/legacy", 42, Some(json!({ "id": "legacy-ws", "name": "Remote", "createdAt": 1 }))).await;
    // Repositories that are not workspaces: no `workspace.json`, someone else's file of that name, and one
    // whose id is not a plain name.
    seed_remote(&guard, "octo/code", 42, None).await;
    seed_remote(&guard, "octo/other-tool", 42, Some(json!({ "id": "x", "folders": [] }))).await;
    seed_remote(&guard, "octo/notes", 42, workspace_file("../escape")).await;
    // Sharing a repository on GitHub shares no workspace while team sync is off.
    guard.hub.lock().collaborators.entry("octo/team".into()).or_default().insert(7, true);

    // The listing shows an account only what it owns, and of that only what is marked as a workspace.
    seed_remote(&guard, "eve/plain", 7, None).await;
    guard.hub.lock().untagged.insert("eve/plain".into());
    let repos = engine.call("account.repos", json!({ "accountId": eve })).await;
    let names: Vec<&str> = repos["repos"].as_array().unwrap().iter().map(|r| r["fullName"].as_str().unwrap()).collect();
    assert_eq!(names, ["eve/own"]);

    let refuse = |result: Value, needle: &str| {
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["error"].as_str().unwrap().contains(needle), "{result}");
    };
    // Asking by name for a repository the account was only given access to is refused.
    let not_owner = engine.call("workspace.import", json!({ "accountId": eve, "repo": "octo/team" })).await;
    assert_eq!(not_owner["reason"], "notOwner", "{not_owner}");
    refuse(not_owner, "belongs to @octo, not @eve");
    let clash = engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/clash" })).await;
    assert!(clash.get("reason").is_none(), "{clash}");
    refuse(clash, "never replaces");
    for not_a_workspace in ["octo/code", "octo/other-tool", "octo/notes"] {
        refuse(
            engine.call("workspace.import", json!({ "accountId": octo, "repo": not_a_workspace })).await,
            "not a Bifurc",
        );
    }
    // A sign-in stored as one account whose token is another's is not believed.
    save_account(&Account { token: Some("tok-eve".into()), ..find_account(&octo).unwrap() }).unwrap();
    refuse(engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/team" })).await, "is not @octo's");
    sign_in(42, "octo");
    assert!(!ws_dir("remote-ws").exists());
    assert!(ws_dir(root.ws_id).join("workspace.json").exists());
    let leftovers: Vec<String> = std::fs::read_dir(crate::workspace::fs::data_root())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".import-"))
        .collect();
    assert!(leftovers.is_empty(), "a refused import leaves nothing behind: {leftovers:?}");

    let imported = engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/team" })).await;
    assert_eq!(imported["ok"], true, "{imported}");
    assert_eq!(imported["workspace"]["id"], "remote-ws");
    assert!(ws_dir("remote-ws").join("requests/r1.json").exists());
    let settings = load_settings();
    assert_eq!(settings.active_workspace_id(), "remote-ws");
    assert_eq!(settings.workspace("remote-ws").unwrap()["accountId"], octo);
    assert_eq!(get_sync_config("remote-ws").unwrap()["repo"], "octo/team");
    assert_eq!(get_sync_config("remote-ws").unwrap()["access"], "owner");
    assert_eq!(
        run(&ws_dir("remote-ws"), &["config", "user.email"]).await.trim(),
        format!("42+octo@users.noreply.{HOST}")
    );
    refuse(
        engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/team" })).await,
        "already a workspace",
    );
    let repos = engine.call("account.repos", json!({ "accountId": octo })).await;
    let team = repos["repos"].as_array().unwrap().iter().find(|r| r["fullName"] == "octo/team").unwrap().clone();
    assert_eq!(team["workspaceId"], "remote-ws");

    // The imported workspace syncs.
    write_entity("remote-ws", "requests", "r2", &request("r2", "Second"), None).unwrap();
    let pushed =
        engine.call("entity.publish", json!({ "workspaceId": "remote-ws", "paths": ["requests/r2.json"] })).await;
    assert_eq!(pushed["ok"], true, "{pushed}");
    let bare = guard.hub.lock().repos["octo/team"].1.clone();
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir("remote-ws"), &["rev-parse", "HEAD"]).await);

    let legacy = engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/legacy" })).await;
    assert_eq!(legacy["ok"], true, "{legacy}");
    assert_eq!(legacy["workspace"]["syncConfig"]["owner"]["login"], "octo");
    auto_sync::stop_all_auto_sync();

    // A workspace whose account is not its repository's owner (its settings were edited) does not sync.
    let mut settings = load_settings();
    settings.workspace_mut("remote-ws").unwrap()["accountId"] = json!(eve);
    crate::workspace::settings::save_settings(&settings).unwrap();
    let refused = engine.call("sync.push", json!({ "workspaceId": "remote-ws" })).await;
    refuse(refused, "belongs to @octo");
}

#[tokio::test]
async fn a_workspace_of_plain_files_becomes_a_repository_and_is_synced_whole() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let ws = root.ws_id;
    let guard = start_hub().await;
    let engine = Engine::new();
    let octo = sign_in(42, "octo");
    let subjects = |log: String| log.lines().map(str::to_string).collect::<Vec<_>>();

    // Made while git was missing: files, no repository.
    write_entity(ws, "requests", "r1", &request("r1", "First"), None).unwrap();
    let folder = crate::workspace::fs::create_folder(ws, "requests", "Folder", None, false).unwrap().0;
    write_entity(ws, "requests", "r2", &request("r2", "Second"), Some(&folder)).unwrap();
    write_entity(ws, "mocks", "m1", &json!({ "id": "m1", "name": "Mock", "urlPattern": "/x" }), None).unwrap();
    assert!(!ws_dir(ws).join(".git").exists());

    // Asking what the machine has is how a git installed since is noticed: the workspace is a repository
    // from then on, with everything in it new.
    assert_eq!(engine.call("setup.getState", json!({})).await["gitInstalled"], true);
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": ws })).await["gitTracked"], true);
    let status = get_workspace_sync_status(ws).await;
    assert_eq!(status.values().filter(|state| *state == "new").count(), 3, "{status:?}");

    // One sync commits each of them as created, in a commit of its own.
    let synced = engine.call("sync.all", json!({ "workspaceId": ws })).await;
    assert_eq!(synced["ok"], true, "{synced}");
    assert_eq!((synced["total"].as_u64(), synced["synced"].as_u64(), synced["pushed"].as_bool()), (Some(3), Some(3), Some(false)));
    let log = subjects(run(&ws_dir(ws), &["log", "--reverse", "--format=%s"]).await);
    assert_eq!(log, ["create workspace Workspace 1", "create mock Mock", "create request Second", "create request First"]);
    assert!(run(&ws_dir(ws), &["log", "-1", "--format=%b"]).await.contains("entity-id: r1"));
    assert!(get_workspace_sync_status(ws).await.values().all(|state| state == "clean"));
    let again = engine.call("sync.all", json!({ "workspaceId": ws })).await;
    assert_eq!((again["ok"].as_bool(), again["total"].as_u64()), (Some(true), Some(0)), "{again}");

    // Publishing takes what was never synced with it when asked to.
    write_entity(ws, "requests", "r3", &request("r3", "Third"), None).unwrap();
    write_entity(ws, "requests", "r1", &request("r1", "First again"), None).unwrap();
    let published = engine
        .call("sync.publish", json!({ "workspaceId": ws, "accountId": octo, "repoName": "whole", "syncAll": true }))
        .await;
    assert_eq!(published["ok"], true, "{published}");
    assert_eq!((published["sync"]["synced"].as_u64(), published["sync"]["pushed"].as_bool()), (Some(2), Some(true)));
    auto_sync::stop_auto_sync(ws);
    let bare = guard.hub.lock().repos["octo/whole"].1.clone();
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);
    let log = subjects(run(&bare, &["log", "--format=%s", "-2", "main"]).await);
    assert_eq!(log, ["create request Third", "update request First again"]);

    // With a remote, the commits of one sync are pushed together. A deletion is not part of it: it waits
    // to be confirmed or reverted.
    write_entity(ws, "requests", "r4", &request("r4", "Fourth"), None).unwrap();
    write_entity(ws, "requests", "r5", &request("r5", "Fifth"), None).unwrap();
    std::fs::remove_file(ws_dir(ws).join("mocks/m1.json")).unwrap();
    let synced = engine.call("sync.all", json!({ "workspaceId": ws })).await;
    assert_eq!(synced["ok"], true, "{synced}");
    assert_eq!((synced["synced"].as_u64(), synced["pushed"].as_bool(), synced["pendingDeletions"].as_u64()), (Some(2), Some(true), Some(1)));
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);
    assert!(run(&bare, &["show", "main:mocks/m1.json"]).await.contains("Mock"));
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": ws })).await["status"], "idle");
    auto_sync::stop_all_auto_sync();
}

#[tokio::test]
async fn a_shared_workspace_is_imported_and_synced_with_the_access_github_gives() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let guard = start_hub().await;
    let _team = TeamSync::set(true);
    let engine = Engine::new();
    let octo = sign_in(42, "octo");
    let eve = sign_in(7, "eve");
    let ws = "remote-ws";

    seed_remote(&guard, "octo/team", 42, workspace_file(ws)).await;
    seed_remote(&guard, "octo/other", 42, workspace_file("other-ws")).await;
    seed_remote(&guard, "octo/mine", 42, workspace_file("mine-ws")).await;
    seed_remote(&guard, "eve/own", 7, workspace_file("eve-ws")).await;
    let share = |repo: &str, push: bool| {
        guard.hub.lock().collaborators.entry(repo.to_string()).or_default().insert(7, push);
    };
    share("octo/team", false);
    // The organization gave eve both workspaces to write to, so what GitHub gives is what she has.
    guard.grant("octo/team", 7, Some("write"));
    guard.grant("octo/other", 7, Some("write"));
    let bare = guard.hub.lock().repos["octo/team"].1.clone();
    let refuse = |result: Value, needle: &str| {
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["error"].as_str().unwrap().contains(needle), "{result}");
    };

    // The listing shows what the account owns and what it was given access to, with what it may do there.
    assert_eq!(engine.call("setup.getState", json!({})).await["teamSync"], true);
    let repos = engine.call("account.repos", json!({ "accountId": eve })).await;
    let listed: Vec<(&str, &str, &str)> = repos["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["fullName"].as_str().unwrap(), r["owner"].as_str().unwrap(), r["access"].as_str().unwrap()))
        .collect();
    assert_eq!(listed, [("eve/own", "eve", "owner"), ("octo/team", "octo", "read")]);

    // Someone the repository is shared with imports the workspace, as themselves.
    let imported = engine.call("workspace.import", json!({ "accountId": eve, "repo": "octo/team" })).await;
    assert_eq!(imported["ok"], true, "{imported}");
    auto_sync::stop_all_auto_sync();
    assert_eq!(imported["workspace"]["syncConfig"]["access"], "read");
    assert_eq!(load_settings().workspace(ws).unwrap()["accountId"], eve);
    let state = engine.call("sync.getState", json!({ "workspaceId": ws })).await;
    assert_eq!(
        (state["access"].as_str(), state["owner"]["login"].as_str(), state["account"]["login"].as_str()),
        (Some("read"), Some("octo"), Some("eve"))
    );
    let config = engine.call("config.get", json!({})).await;
    let listed = config["workspaces"].as_array().unwrap().iter().find(|w| w["id"] == ws).unwrap().clone();
    assert_eq!(listed["syncConfig"]["access"], "read");

    // With read access the workspace is read-only: what is changed here is saved here, and no way of
    // syncing commits or pushes any of it.
    let first = run(&bare, &["rev-parse", "main"]).await;
    write_entity(ws, "requests", "r2", &request("r2", "Second"), None).unwrap();
    std::fs::remove_file(ws_dir(ws).join("requests/r1.json")).unwrap();
    engine.call("sync.getEntityStatus", json!({ "workspaceId": ws })).await;
    let paths = json!({ "workspaceId": ws, "paths": ["requests/r2.json"] });
    for (action, payload) in [
        ("entity.publish", paths.clone()),
        ("git.sync", paths),
        ("folder.publish", json!({ "workspaceId": ws, "kind": "requests", "folderName": "" })),
        ("sync.all", json!({ "workspaceId": ws })),
        ("sync.push", json!({ "workspaceId": ws })),
        ("entity.confirmDelete", json!({ "workspaceId": ws, "kind": "requests", "id": "r1" })),
    ] {
        let refused = engine.call(action, payload).await;
        assert_eq!(refused["ok"], false, "{action}: {refused}");
        let error = refused["error"].as_str().unwrap();
        assert!(error.contains("read-only") && error.contains("@octo"), "{action}: {refused}");
    }
    assert_eq!(run(&ws_dir(ws), &["rev-parse", "HEAD"]).await, first, "nothing is committed");
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, first, "nothing is pushed");
    crate::workspace::status_cache::invalidate_cache(ws);
    let status = get_workspace_sync_status(ws).await;
    assert_eq!(
        (status["requests/r2.json"].as_str(), status["requests/r1.json"].as_str()),
        (Some("new"), Some("deleted"))
    );
    // What does not reach the remote still works: a deletion made here is taken back.
    let reverted =
        engine.call("entity.revertDelete", json!({ "workspaceId": ws, "kind": "requests", "id": "r1" })).await;
    assert_eq!(reverted["ok"], true, "{reverted}");
    assert!(ws_dir(ws).join("requests/r1.json").exists());

    // What the remote gets still comes in, under what was changed here.
    let other = root.dir.path().join("other-device");
    run(root.dir.path(), &["clone", "--branch", "main", &bare.to_string_lossy(), &other.to_string_lossy()]).await;
    run(&other, &["config", "user.email", "o@example.com"]).await;
    run(&other, &["config", "user.name", "Other"]).await;
    let push_from_other = |id: &'static str, name: &'static str| {
        let other = other.clone();
        async move {
            run(&other, &["pull", "origin", "main"]).await;
            let file = other.join(format!("requests/{id}.json"));
            std::fs::write(file, serde_json::to_string_pretty(&request(id, name)).unwrap()).unwrap();
            run(&other, &["add", "-A"]).await;
            run(&other, &["commit", "-m", "create request"]).await;
            run(&other, &["push", "origin", "HEAD:main"]).await;
        }
    };
    push_from_other("r9", "Nine").await;
    let pulled = engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    assert_eq!((pulled["ok"].as_bool(), pulled["updated"].as_bool()), (Some(true), Some(true)), "{pulled}");
    assert_eq!(run(&ws_dir(ws), &["rev-parse", "HEAD"]).await, run(&bare, &["rev-parse", "main"]).await);
    assert!(ws_dir(ws).join("requests/r9.json").exists() && ws_dir(ws).join("requests/r2.json").exists());

    // Write access given on GitHub is found by the next pull asked for, and then the workspace syncs.
    share("octo/team", true);
    engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": ws })).await["access"], "write");
    let pushed = engine.call("entity.publish", json!({ "workspaceId": ws, "paths": ["requests/r2.json"] })).await;
    assert_eq!(pushed["ok"], true, "{pushed}");
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);
    assert_eq!(
        run(&bare, &["log", "-1", "--format=%ae", "main"]).await.trim(),
        format!("7+eve@users.noreply.{HOST}")
    );

    // Taken away again, it is gone at the next thing that would push: nothing waits for a cache to lapse.
    share("octo/team", false);
    write_entity(ws, "requests", "r3", &request("r3", "Third"), None).unwrap();
    let synced = run(&bare, &["rev-parse", "main"]).await;
    refuse(engine.call("sync.all", json!({ "workspaceId": ws })).await, "read-only");
    assert_eq!(run(&ws_dir(ws), &["rev-parse", "HEAD"]).await, synced);
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": ws })).await["access"], "read");

    // The switch: with team sync off the same data is the owner's alone again. Nothing shared is listed
    // or imported, and the workspace that was shared neither pushes nor pulls.
    share("octo/other", true);
    set_test_team_sync(Some(false));
    assert_eq!(engine.call("setup.getState", json!({})).await["teamSync"], false);
    let repos = engine.call("account.repos", json!({ "accountId": eve })).await;
    let names: Vec<&str> = repos["repos"].as_array().unwrap().iter().map(|r| r["fullName"].as_str().unwrap()).collect();
    assert_eq!(names, ["eve/own"]);
    let blocked = engine.call("workspace.import", json!({ "accountId": eve, "repo": "octo/other" })).await;
    assert_eq!(blocked["reason"], "notOwner", "{blocked}");
    push_from_other("r10", "Ten").await;
    refuse(engine.call("sync.pull", json!({ "workspaceId": ws })).await, "Only its owner can sync it");
    refuse(engine.call("sync.push", json!({ "workspaceId": ws })).await, "Only its owner can sync it");
    assert!(!ws_dir(ws).join("requests/r10.json").exists());
    // Switched back on, it carries on from where it was.
    set_test_team_sync(Some(true));
    let pulled = engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    assert_eq!(pulled["updated"], true, "{pulled}");
    assert!(ws_dir(ws).join("requests/r10.json").exists());
    // Someone given write access from the start syncs from the start.
    let other_ws = engine.call("workspace.import", json!({ "accountId": eve, "repo": "octo/other" })).await;
    assert_eq!(other_ws["workspace"]["syncConfig"]["access"], "write", "{other_ws}");
    auto_sync::stop_all_auto_sync();

    // A workspace of the version before this one has no access in its settings. It is its owner's, syncs
    // as it did, and has the access written down the first time GitHub is asked.
    let mine = engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/mine" })).await;
    assert_eq!(mine["ok"], true, "{mine}");
    auto_sync::stop_all_auto_sync();
    let mut settings = load_settings();
    settings.workspace_mut("mine-ws").unwrap()["syncConfig"].as_object_mut().unwrap().remove("access");
    crate::workspace::settings::save_settings(&settings).unwrap();
    forget_access();
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": "mine-ws" })).await["access"], Value::Null);
    write_entity("mine-ws", "requests", "r2", &request("r2", "Second"), None).unwrap();
    let pushed =
        engine.call("entity.publish", json!({ "workspaceId": "mine-ws", "paths": ["requests/r2.json"] })).await;
    assert_eq!(pushed["ok"], true, "{pushed}");
    let mine_bare = guard.hub.lock().repos["octo/mine"].1.clone();
    assert_eq!(run(&mine_bare, &["rev-parse", "main"]).await, run(&ws_dir("mine-ws"), &["rev-parse", "HEAD"]).await);
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": "mine-ws" })).await["access"], "owner");
}

#[tokio::test]
async fn someone_elses_workspace_is_imported_only_when_the_console_gave_it_to_the_account() {
    if !check_git_installed().await {
        return;
    }
    let _root = scratch_data_root();
    let guard = start_hub().await;
    let _team = TeamSync::set(true);
    let engine = Engine::new();
    let octo = sign_in(42, "octo");
    let eve = sign_in(7, "eve");
    let ws = "remote-ws";

    seed_remote(&guard, "octo/team", 42, workspace_file(ws)).await;
    seed_remote(&guard, "octo/before", 42, workspace_file("before-ws")).await;
    seed_remote(&guard, "octo/mine", 42, workspace_file("mine-ws")).await;
    // GitHub lets eve push to both: she was made a collaborator there.
    for repo in ["octo/team", "octo/before"] {
        guard.hub.lock().collaborators.entry(repo.to_string()).or_default().insert(7, true);
    }
    let down = |down: bool| guard.hub.lock().console_down = down;
    let import = |repo: &'static str| engine.call("workspace.import", json!({ "accountId": eve, "repo": repo }));
    let refused = |result: Value, reason: &str| {
        assert_eq!((result["ok"].as_bool(), result["reason"].as_str()), (Some(false), Some(reason)), "{result}");
        assert!(!ws_dir(ws).exists(), "a refused import brings nothing to this device");
    };
    let access = || async { engine.call("sync.getState", json!({ "workspaceId": ws })).await["access"].clone() };
    let publish = |id: &'static str| {
        write_entity(ws, "requests", id, &request(id, "Request"), None).unwrap();
        engine.call("entity.publish", json!({ "workspaceId": ws, "paths": [format!("requests/{id}.json")] }))
    };

    // Being a collaborator on GitHub imports nothing: not while no organization has the repository, not
    // while the organization has not given it to her, and not while she has no seat there.
    refused(import("octo/team").await, "unmanaged");
    guard.grant("octo/team", 7, None);
    refused(import("octo/team").await, "notGranted");
    guard.grant("octo/team", 7, Some("seatInactive"));
    refused(import("octo/team").await, "seatInactive");
    // No answer is not a yes.
    guard.grant("octo/team", 7, Some("write"));
    down(true);
    let unreachable = import("octo/team").await;
    assert!(unreachable["error"].as_str().unwrap().contains("could not be reached"), "{unreachable}");
    refused(unreachable, "consoleUnreachable");
    // The owner of a repository is never asked about.
    let mine = engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/mine" })).await;
    assert_eq!(mine["ok"], true, "{mine}");
    auto_sync::stop_all_auto_sync();
    down(false);

    // Given it to read, she has it read-only, whatever GitHub lets her do.
    guard.grant("octo/team", 7, Some("read"));
    let imported = import("octo/team").await;
    assert_eq!(imported["ok"], true, "{imported}");
    auto_sync::stop_all_auto_sync();
    assert_eq!(imported["workspace"]["syncConfig"]["access"], "read");
    let bare = guard.hub.lock().repos["octo/team"].1.clone();
    let first = run(&bare, &["rev-parse", "main"]).await;
    let blocked = publish("r2").await;
    assert!(blocked["error"].as_str().unwrap().contains("read-only"), "{blocked}");
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, first);
    // While the console cannot be asked, reading stays all she has.
    down(true);
    assert_eq!(publish("r2").await["ok"], false);
    assert_eq!(access().await, "read");
    down(false);

    // Given it to write, the next thing that pushes finds out.
    guard.grant("octo/team", 7, Some("write"));
    let pushed = publish("r2").await;
    assert_eq!(pushed["ok"], true, "{pushed}");
    assert_eq!(access().await, "write");
    assert_eq!(run(&bare, &["rev-parse", "main"]).await, run(&ws_dir(ws), &["rev-parse", "HEAD"]).await);
    // A console that cannot be asked takes nothing away from someone who had it to write.
    down(true);
    assert_eq!(publish("r3").await["ok"], true);
    down(false);
    // Losing the seat, or the workspace, in the console leaves her what is on this device, read-only.
    for lost in [Some("seatInactive"), None] {
        guard.grant("octo/team", 7, lost);
        assert_eq!(publish("r4").await["ok"], false, "{lost:?}");
        assert_eq!(access().await, "read");
    }

    // A workspace that was shared before there was a console is in settings from the version before this
    // one, on a repository no organization has. It keeps syncing with what GitHub gives.
    guard.grant("octo/before", 7, Some("write"));
    let before = import("octo/before").await;
    assert_eq!(before["ok"], true, "{before}");
    auto_sync::stop_all_auto_sync();
    {
        let mut hub = guard.hub.lock();
        hub.managed.remove(&repo_id("octo/before"));
        hub.grants.clear();
    }
    write_entity("before-ws", "requests", "r2", &request("r2", "Second"), None).unwrap();
    let synced =
        engine.call("entity.publish", json!({ "workspaceId": "before-ws", "paths": ["requests/r2.json"] })).await;
    assert_eq!(synced["ok"], true, "{synced}");
    assert_eq!(get_sync_config("before-ws").unwrap()["access"], "write");
}

#[tokio::test]
async fn an_organizations_workspace_has_its_analytics_decided_in_the_console() {
    if !check_git_installed().await {
        return;
    }
    let _root = scratch_data_root();
    let guard = start_hub().await;
    let _team = TeamSync::set(true);
    let engine = Engine::new();
    let octo = sign_in(42, "octo");
    let eve = sign_in(7, "eve");
    let (ws, mine, own) = ("remote-ws", "mine-ws", "own-ws");

    seed_remote(&guard, "octo/team", 42, workspace_file(ws)).await;
    seed_remote(&guard, "octo/mine", 42, workspace_file(mine)).await;
    seed_remote(&guard, "eve/own", 7, workspace_file(own)).await;
    guard.hub.lock().collaborators.entry("octo/team".to_string()).or_default().insert(7, true);
    guard.grant("octo/team", 7, Some("write"));
    guard.hub.lock().managed.insert(repo_id("octo/mine"));
    let down = |down: bool| guard.hub.lock().console_down = down;
    let stored = |id: &str| load_settings().workspace(id).unwrap().get("organization").cloned().unwrap_or(Value::Null);
    let shown = |id: &'static str| {
        let engine = &engine;
        async move {
            let config = engine.call("config.get", json!({})).await;
            let workspaces = config["workspaces"].as_array().unwrap();
            workspaces.iter().find(|workspace| workspace["id"] == id).unwrap()["organization"].clone()
        }
    };
    let acme = |repo: &str, user: u64, analytics: bool| {
        json!({
            "publicId": "pub_acme", "name": "Acme", "workspaceId": format!("ws_{}", repo_id(repo)),
            "analytics": analytics, "githubUserId": user,
        })
    };

    // Whose a workspace is is known as soon as it is on this device: the member's, the organization owner's
    // own, and one that is nobody's but its user's.
    for (account, repo) in [(&eve, "octo/team"), (&octo, "octo/mine"), (&eve, "eve/own")] {
        let imported = engine.call("workspace.import", json!({ "accountId": account, "repo": repo })).await;
        assert_eq!(imported["ok"], true, "{imported}");
        auto_sync::stop_all_auto_sync();
    }
    assert_eq!(stored(ws), acme("octo/team", 7, false));
    assert_eq!(stored(mine), acme("octo/mine", 42, false));
    assert_eq!(stored(own), Value::Null);
    assert_eq!(shown(ws).await, json!({ "name": "Acme", "analytics": false }));
    assert_eq!(shown(own).await, Value::Null);

    // The owner turns analytics on in the console: each workspace finds out the next time it asks, and the
    // member's own switch on this device has no say in it.
    engine.call("settings.update", json!({ "telemetryEnabled": false })).await;
    guard.hub.lock().analytics = true;
    engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    organization::refresh(mine).await;
    organization::refresh(own).await;
    assert_eq!(stored(ws), acme("octo/team", 7, true));
    assert_eq!(stored(mine), acme("octo/mine", 42, true));
    assert_eq!(stored(own), Value::Null);
    assert_eq!(shown(ws).await, json!({ "name": "Acme", "analytics": true }));
    assert_eq!(engine.call("config.get", json!({})).await["telemetryEnabled"], false);

    // A console that cannot be asked changes nothing.
    down(true);
    engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    organization::refresh(mine).await;
    assert_eq!((stored(ws), stored(mine)), (acme("octo/team", 7, true), acme("octo/mine", 42, true)));
    down(false);

    // Turned off again, and then the workspace taken out of the organization: it is its user's again.
    guard.hub.lock().analytics = false;
    organization::refresh(ws).await;
    assert_eq!(stored(ws), acme("octo/team", 7, false));
    guard.hub.lock().managed.remove(&repo_id("octo/team"));
    organization::refresh(ws).await;
    assert_eq!((stored(ws), shown(ws).await), (Value::Null, Value::Null));
}

#[tokio::test]
async fn a_workspace_the_console_made_says_whose_it_is_at_the_import() {
    if !check_git_installed().await {
        return;
    }
    let _root = scratch_data_root();
    let guard = start_hub().await;
    let engine = Engine::new();
    let octo = sign_in(42, "octo");
    let declares = |ws: &str, organization: Value| {
        let mut file = workspace_file(ws).unwrap();
        file["organization"] = organization;
        Some(file)
    };
    let acme = json!({ "publicId": "pub_acme", "name": "Acme", "workspaceId": "wsp_team" });
    seed_remote(&guard, "octo/team", 42, declares("team-ws", acme)).await;
    seed_remote(&guard, "octo/odd", 42, declares("odd-ws", json!({ "publicId": "../x", "workspaceId": "wsp_odd" }))).await;
    let stored = |id: &str| load_settings().workspace(id).unwrap().get("organization").cloned().unwrap_or(Value::Null);
    let import = |repo: &'static str| engine.call("workspace.import", json!({ "accountId": octo, "repo": repo }));

    // The console cannot be asked: the ids are known all the same, and nothing is reported until it says so.
    guard.hub.lock().console_down = true;
    for repo in ["octo/team", "octo/odd"] {
        let imported = import(repo).await;
        assert_eq!(imported["ok"], true, "{imported}");
        auto_sync::stop_all_auto_sync();
    }
    let known = json!({
        "publicId": "pub_acme", "name": "Acme", "workspaceId": "wsp_team", "analytics": false, "githubUserId": 42,
    });
    assert_eq!(stored("team-ws"), known);
    // An id that is not a plain one is not taken.
    assert_eq!(stored("odd-ws"), Value::Null);

    // What the console says once it can be asked is what stands.
    guard.hub.lock().console_down = false;
    guard.hub.lock().managed.insert(repo_id("octo/team"));
    guard.hub.lock().analytics = true;
    organization::refresh("team-ws").await;
    assert_eq!(stored("team-ws")["analytics"], true);
    assert_eq!(stored("team-ws")["workspaceId"], format!("ws_{}", repo_id("octo/team")));
}

#[tokio::test]
async fn a_workspace_synced_by_a_version_before_organizations_learns_whose_it_is() {
    if !check_git_installed().await {
        return;
    }
    let _root = scratch_data_root();
    let guard = start_hub().await;
    let _team = TeamSync::set(true);
    let engine = Engine::new();
    let octo = sign_in(42, "octo");
    let ws = "mine-ws";
    seed_remote(&guard, "octo/mine", 42, workspace_file(ws)).await;
    let imported = engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/mine" })).await;
    assert_eq!(imported["ok"], true, "{imported}");
    auto_sync::stop_all_auto_sync();

    // The workspace as 0.5.0 left it in `app.json`: a remote with no repository id, and no organization.
    let mut settings = load_settings();
    let workspace = settings.workspace_mut(ws).unwrap();
    let before = {
        let stored = workspace.as_object_mut().unwrap();
        stored.remove("organization");
        stored["syncConfig"].as_object_mut().unwrap().remove("repoId");
        stored.clone()
    };
    assert_eq!(
        before.keys().map(String::as_str).collect::<Vec<_>>(),
        ["id", "name", "activeEnvironmentId", "accountId", "syncConfig", "syncMeta"]
    );
    crate::workspace::settings::save_settings(&settings).unwrap();

    // Nobody's but its user's until an organization has it: nothing is added but the repository's id.
    organization::refresh(ws).await;
    let mut expected = before.clone();
    expected["syncConfig"]["repoId"] = json!(repo_id("octo/mine"));
    assert_eq!(load_settings().workspace(ws).unwrap(), &Value::Object(expected.clone()));

    // The owner adds it to an organization in the console and turns analytics on there.
    guard.hub.lock().managed.insert(repo_id("octo/mine"));
    guard.hub.lock().analytics = true;
    organization::refresh(ws).await;
    expected.insert(
        "organization".into(),
        json!({
            "publicId": "pub_acme", "name": "Acme", "workspaceId": format!("ws_{}", repo_id("octo/mine")),
            "analytics": true, "githubUserId": 42,
        }),
    );
    assert_eq!(load_settings().workspace(ws).unwrap(), &Value::Object(expected));
    // What it synced with and as whom is what it was.
    assert_eq!(engine.call("sync.getState", json!({ "workspaceId": ws })).await["access"], "owner");
}
