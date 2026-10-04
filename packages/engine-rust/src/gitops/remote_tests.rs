//! Tests of sign-in, import, publish and the owner rules, against a stand-in for GitHub whose repositories are
//! bare repositories in a temporary directory. Skipped when git is missing.

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
    auto_sync, get_sync_config, get_workspace_git_state, get_workspace_sync_status,
    github::{Endpoints, set_test_endpoints},
};
use crate::{
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
    root: PathBuf,
    /// Whether the user has typed the device code yet.
    granted: bool,
}

fn repo_json(full_name: &str, owner_id: u64, path: &Path) -> Value {
    json!({
        "full_name": full_name, "name": full_name.rsplit('/').next().unwrap(),
        "owner": { "id": owner_id, "login": full_name.split('/').next().unwrap() },
        "private": true, "description": "", "clone_url": path.to_string_lossy(), "html_url": "",
        "default_branch": "main", "updated_at": "2026-01-01T00:00:00Z", "topics": [],
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
            (201, repo_json(&full_name, id, &dir))
        }
        ("GET", path) if path.starts_with("/user/repos") => {
            let Some((id, _)) = user else { return (401, json!({ "message": "Bad credentials" })) };
            let mut repos: Vec<Value> = hub
                .lock()
                .repos
                .iter()
                .filter(|(_, (owner, _))| *owner == id)
                .map(|(name, (owner, dir))| repo_json(name, *owner, dir))
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
        ("GET", path) if path.starts_with("/repos/") => match hub.lock().repos.get(&path["/repos/".len()..]) {
            Some((owner, dir)) if user.is_some() => (200, repo_json(&path["/repos/".len()..], *owner, dir)),
            _ => (404, json!({ "message": "Not Found" })),
        },
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
    }
}

async fn start_hub() -> HubGuard {
    let dir = tempfile::tempdir().unwrap();
    let hub = Arc::new(Mutex::new(Hub { root: dir.path().to_path_buf(), ..Hub::default() }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{HOST}:{}", listener.local_addr().unwrap().port());
    set_test_endpoints(Some(Endpoints { web: base.clone(), api: base }));
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
    run(&other, &["add", "-A"]).await;
    run(&other, &["commit", "-m", "create request Nine"]).await;
    run(&other, &["push", "origin", "HEAD:main"]).await;
    let pulled = engine.call("sync.pull", json!({ "workspaceId": ws })).await;
    assert_eq!(pulled, json!({ "ok": true, "updated": true, "updatedIds": ["r9"] }));
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
async fn only_the_owner_imports_a_workspace_and_nothing_local_is_replaced() {
    if !check_git_installed().await {
        return;
    }
    let root = scratch_data_root();
    let guard = start_hub().await;
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

    // The listing shows an account only what it owns.
    let repos = engine.call("account.repos", json!({ "accountId": eve })).await;
    let names: Vec<&str> = repos["repos"].as_array().unwrap().iter().map(|r| r["fullName"].as_str().unwrap()).collect();
    assert_eq!(names, ["eve/own"]);

    let refuse = |result: Value, needle: &str| {
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["error"].as_str().unwrap().contains(needle), "{result}");
    };
    // Asking by name for a repository the account was only given access to is refused.
    refuse(
        engine.call("workspace.import", json!({ "accountId": eve, "repo": "octo/team" })).await,
        "belongs to @octo, not @eve",
    );
    refuse(engine.call("workspace.import", json!({ "accountId": octo, "repo": "octo/clash" })).await, "never replaces");
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
    write_entity(ws, "requests", "r2", &request("r2", "Second"), Some("Folder")).unwrap();
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
