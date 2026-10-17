//! `devservice.*`: dev servers the engine runs for the CLI. A definition is a file in the workspace
//! (`devservices/<name>.json`: `{name, cmd, port?, cwd?, vault?, env?}`); a started service is a process tree the
//! engine supervises, with the last lines of its output in memory. Processes end with the engine (Windows: the
//! job object; unix: the process group, when the runtime drops its tasks) and are never written to disk.

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    process::Stdio,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    sync::oneshot,
};

use super::discovery::discover_services;
use crate::{
    kernel::CommandRegistry,
    mcp::client::shell_command,
    protocol::{EngineError, Result},
    utils::{
        id::now_ms,
        json::{read_json, str_of, write_json_pretty},
        process::hidden_command,
    },
    workspace::{
        contracts::plain,
        fs::ws_dir,
        git_store::{CommitMutation, commit_mutation},
    },
};

const DIR: &str = "devservices";
/// Lines kept per service, and the longest one kept whole.
const RING: usize = 5000;
const LINE_CHARS: usize = 2000;

struct Line {
    ts: i64,
    stream: &'static str,
    text: String,
}

/// One run of a service. `code` is the exit code once `running` is false.
struct Run {
    pid: u32,
    started: i64,
    ended: i64,
    stopping: bool,
    running: bool,
    code: i32,
    lines: VecDeque<Line>,
    stop: Option<oneshot::Sender<()>>,
}

type Shared = Arc<Mutex<Run>>;

/// The latest run of each service, by workspace and name.
static RUNS: LazyLock<Mutex<HashMap<(String, String), Shared>>> = LazyLock::new(Default::default);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn dir_of(params: &Value) -> Result<PathBuf> {
    let ws = str_of(params, "workspaceId");
    if !plain(ws) || !ws_dir(ws).is_dir() {
        return Err(EngineError::not_found(format!("No workspace \"{ws}\".")));
    }
    Ok(ws_dir(ws).join(DIR))
}

fn definition(params: &Value) -> Result<Value> {
    let name = str_of(params, "name");
    let dir = dir_of(params)?;
    if !plain(name) {
        return Err(EngineError::not_found(format!("No service \"{name}\".")));
    }
    read_json(&dir.join(format!("{name}.json")))
        .filter(Value::is_object)
        .ok_or_else(|| EngineError::not_found(format!("No service \"{name}\".")))
}

fn run_of(params: &Value) -> Option<Shared> {
    lock(&RUNS).get(&(str_of(params, "workspaceId").to_string(), str_of(params, "name").to_string())).cloned()
}

fn push(run: &Shared, stream: &'static str, mut text: String) {
    text.truncate(text.char_indices().nth(LINE_CHARS).map_or(text.len(), |(i, _)| i));
    let mut run = lock(run);
    if run.lines.len() == RING {
        run.lines.pop_front();
    }
    run.lines.push_back(Line { ts: now_ms(), stream, text });
}

/// Read `stream` line by line into the run's ring until it closes.
fn pump(
    stream: impl AsyncRead + Unpin + Send + 'static,
    name: &'static str,
    run: Shared,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (mut reader, mut buf) = (BufReader::new(stream), Vec::new());
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => push(&run, name, String::from_utf8_lossy(&buf).trim_end().to_string()),
            }
        }
    })
}

async fn listening(port: u64) -> bool {
    let connect = tokio::net::TcpStream::connect(("127.0.0.1", port as u16));
    matches!(tokio::time::timeout(Duration::from_millis(300), connect).await, Ok(Ok(_)))
}

/// A definition with how its latest run is going.
async fn describe(def: &Value, run: Option<Shared>) -> Value {
    let mut out = def.clone();
    let (state, extra) = match run {
        None => ("stopped", json!({})),
        Some(run) => {
            let run = lock(&run);
            let state = if run.running {
                "running"
            } else if run.stopping || run.code == 0 {
                "stopped"
            } else {
                "crashed"
            };
            let extra = json!({ "pid": run.pid, "startedAt": run.started, "endedAt": (!run.running).then_some(run.ended),
                                "exitCode": (!run.running).then_some(run.code) });
            (state, extra)
        }
    };
    out["state"] = json!(state);
    out.as_object_mut().expect("object").extend(extra.as_object().cloned().unwrap_or_default());
    if let (Some(port), "running") = (def["port"].as_u64(), state) {
        out["listening"] = json!(listening(port).await);
    }
    out
}

fn definitions(dir: &PathBuf) -> Vec<Value> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files.iter().filter_map(|p| read_json(p)).filter(Value::is_object).collect()
}

async fn commit(params: &Value, action: &str, name: &str) {
    // Best effort: a workspace without a repository keeps the files and has no audit entry.
    let rel = format!("{DIR}/{name}.json");
    let _ = commit_mutation(CommitMutation {
        action,
        entity: "devservice",
        entity_id: name,
        entity_name: name,
        workspace_id: str_of(params, "workspaceId"),
        rel_path: &rel,
        actor: None,
        changed_fields: &[],
    })
    .await;
}

async fn start(params: &Value) -> Result<Value> {
    let def = definition(params)?;
    if let Some(run) = run_of(params)
        && lock(&run).running
    {
        return Ok(describe(&def, Some(run)).await);
    }
    let cwd = def["cwd"].as_str().filter(|c| !c.is_empty());
    if let Some(cwd) = cwd.filter(|c| !std::path::Path::new(c).is_dir()) {
        return Err(EngineError::bad_request(format!("The directory \"{cwd}\" does not exist.")));
    }
    // A leftover process on the port would answer `--wait` in place of the one started here.
    if let Some(port) = def["port"].as_u64().filter(|p| *p > 0 && *p <= u16::MAX as u64)
        && let Some(holder) = discover_services().await.into_iter().find(|s| u64::from(s.port) == port)
    {
        return Err(EngineError::conflict(format!(
            "Port {port} is held by {} (pid {}) — bifurc port free {port} --kill",
            holder.process_name, holder.pid
        )));
    }
    let mut wrap = shell_command(str_of(&def, "cmd"));
    let command = wrap.command_mut();
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    // The vault's secrets come with the call and are never kept.
    for (key, value) in def["env"].as_object().into_iter().chain(params["secrets"].as_object()).flatten() {
        command.env(key, value.as_str().unwrap_or_default());
    }
    let mut child =
        wrap.spawn().map_err(|e| EngineError::upstream(format!("Cannot start \"{}\": {e}", str_of(&def, "cmd"))))?;
    let (stop, stopped) = oneshot::channel();
    let run: Shared = Arc::new(Mutex::new(Run {
        pid: child.id().unwrap_or(0),
        started: now_ms(),
        ended: 0,
        stopping: false,
        running: true,
        code: 0,
        lines: VecDeque::new(),
        stop: Some(stop),
    }));
    let readers: Vec<_> = [
        child.stdout().take().map(|s| pump(s, "stdout", run.clone())),
        child.stderr().take().map(|s| pump(s, "stderr", run.clone())),
    ]
    .into_iter()
    .flatten()
    .collect();
    lock(&RUNS).insert((str_of(params, "workspaceId").to_string(), str_of(params, "name").to_string()), run.clone());
    let watched = run.clone();
    tokio::spawn(async move {
        let waited = tokio::select! { status = child.wait() => Some(status), _ = stopped => None };
        let status = match waited {
            Some(status) => status,
            None => {
                let _ = child.start_kill();
                child.wait().await
            }
        };
        // Let the readers take what is left of the output, but not wait for a grandchild that holds the pipe.
        let _ = tokio::time::timeout(Duration::from_millis(300), futures::future::join_all(readers)).await;
        let mut run = lock(&watched);
        run.code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
        (run.ended, run.running) = (now_ms(), false);
    });
    Ok(describe(&def, Some(run)).await)
}

async fn stop(params: &Value) -> Result<Value> {
    let def = definition(params)?;
    let run = run_of(params);
    if let Some(run) = &run {
        let sender = {
            let mut run = lock(run);
            run.stopping = true;
            run.stop.take()
        };
        if let Some(sender) = sender {
            let _ = sender.send(());
            for _ in 0..200 {
                if !lock(run).running {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
    Ok(describe(&def, run).await)
}

/// End a process and everything it started. One level of children on unix.
async fn kill_tree(pid: u32) {
    let pid = pid.to_string();
    if cfg!(windows) {
        let _ = hidden_command("taskkill").args(["/PID", &pid, "/T", "/F"]).output().await;
    } else {
        let _ = hidden_command("pkill").args(["-KILL", "-P", &pid]).output().await;
        let _ = hidden_command("kill").args(["-KILL", &pid]).output().await;
    }
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("devservice.define", |params, _| async move {
        let dir = dir_of(&params)?;
        let name = str_of(&params, "name");
        if !plain(name) {
            return Err(EngineError::bad_request(format!(
                "\"{name}\" is not a service name (letters, digits, - and _)."
            )));
        }
        if str_of(&params, "cmd").trim().is_empty() {
            return Err(EngineError::bad_request("A service needs a command."));
        }
        let mut def = json!({ "name": name, "cmd": str_of(&params, "cmd") });
        for key in ["port", "cwd", "vault", "env"] {
            if !params[key].is_null() {
                def[key] = params[key].clone();
            }
        }
        let file = dir.join(format!("{name}.json"));
        let existed = file.exists();
        std::fs::create_dir_all(&dir)?;
        write_json_pretty(&file, &def)?;
        commit(&params, if existed { "update" } else { "create" }, name).await;
        Ok(json!({ "service": def }))
    });

    registry.register("devservice.list", |params, _| async move {
        let mut services = Vec::new();
        for def in definitions(&dir_of(&params)?) {
            let key = (str_of(&params, "workspaceId").to_string(), str_of(&def, "name").to_string());
            let run = lock(&RUNS).get(&key).cloned();
            services.push(describe(&def, run).await);
        }
        Ok(json!({ "services": services }))
    });

    registry.register("devservice.remove", |params, _| async move {
        stop(&params).await?;
        let name = str_of(&params, "name");
        std::fs::remove_file(dir_of(&params)?.join(format!("{name}.json")))?;
        lock(&RUNS).remove(&(str_of(&params, "workspaceId").to_string(), name.to_string()));
        commit(&params, "delete", name).await;
        Ok(json!({ "removed": name }))
    });

    registry.register("devservice.up", |params, _| async move { Ok(json!({ "service": start(&params).await? })) });
    registry.register("devservice.down", |params, _| async move { Ok(json!({ "service": stop(&params).await? })) });

    registry.register("devservice.logs", |params, _| async move {
        definition(&params)?;
        let since = params["sinceTs"].as_i64().unwrap_or(0);
        let mut lines: Vec<Value> = run_of(&params)
            .iter()
            .flat_map(|run| {
                let run = lock(run);
                run.lines
                    .iter()
                    .filter(|l| l.ts >= since)
                    .map(|l| json!({ "ts": l.ts, "stream": l.stream, "text": l.text }))
                    .collect::<Vec<_>>()
            })
            .collect();
        if let Some(last) = params["last"].as_u64() {
            lines.drain(..lines.len().saturating_sub(last as usize));
        }
        Ok(json!({ "lines": lines }))
    });

    registry.register("devservice.freePort", |params, _| async move {
        let port = params["port"].as_u64().unwrap_or(0) as u16;
        let Some(holder) = discover_services().await.into_iter().find(|s| s.port == port) else {
            return Ok(json!({ "held": false, "port": port }));
        };
        let kill = params["kill"].as_bool().unwrap_or(false);
        if kill && holder.pid == std::process::id() {
            return Err(EngineError::bad_request("That port is the engine's own."));
        }
        if kill {
            kill_tree(holder.pid).await;
        }
        Ok(json!({ "held": true, "port": port, "pid": holder.pid, "processName": holder.process_name, "killed": kill }))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{kernel::CommandContext, testing::scratch_data_root};

    async fn call(registry: &Arc<CommandRegistry>, name: &str, payload: Value) -> Result<Value> {
        registry.invoke(name, payload, CommandContext::internal(registry.clone())).await
    }

    #[tokio::test]
    async fn a_definition_round_trips_and_a_service_that_fails_is_crashed_with_its_output() {
        let _root = scratch_data_root();
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ws = json!("ws1");
        let define = json!({"workspaceId": ws, "name": "api", "cmd": "echo hello && exit 3", "env": {"A": "1"}});
        call(&registry, "devservice.define", define).await.unwrap();
        let listed = call(&registry, "devservice.list", json!({"workspaceId": ws})).await.unwrap();
        assert_eq!(listed["services"][0]["cmd"], "echo hello && exit 3");
        assert_eq!(listed["services"][0]["state"], "stopped");
        call(&registry, "devservice.up", json!({"workspaceId": ws, "name": "api"})).await.unwrap();
        for _ in 0..100 {
            let state = call(&registry, "devservice.list", json!({"workspaceId": ws})).await.unwrap();
            if state["services"][0]["state"] == "crashed" {
                assert_eq!(state["services"][0]["exitCode"], 3);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let logs = call(&registry, "devservice.logs", json!({"workspaceId": ws, "name": "api"})).await.unwrap();
        assert_eq!(logs["lines"][0]["text"], "hello");
        assert_eq!(logs["lines"][0]["stream"], "stdout");
        let missing = call(&registry, "devservice.up", json!({"workspaceId": ws, "name": "nope"})).await;
        assert!(missing.is_err());
        call(&registry, "devservice.remove", json!({"workspaceId": ws, "name": "api"})).await.unwrap();
        let listed = call(&registry, "devservice.list", json!({"workspaceId": ws})).await.unwrap();
        assert!(listed["services"].as_array().unwrap().is_empty());
    }
}
