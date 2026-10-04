//! Running applications: spawns the stored command through the OS shell, captures its output into a bounded
//! log, tracks its state and stops its process tree.
//!
//! State lives in a process-wide table keyed by application id. Output chunks are emitted as `process.output`
//! and state changes as `process.statusChange` on the engine bus.

use std::{
    collections::VecDeque,
    process::Stdio,
    sync::{Arc, LazyLock},
    time::Duration,
};

use indexmap::IndexMap;
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncReadExt};

use super::types::{MAX_LOG_LINES, js_str};
use crate::{
    kernel::bus,
    utils::{
        id::now_ms,
        json::{opt_str, string_map, truthy},
        process::hidden_command,
    },
    workspace::fs::read_entity,
};

/// How long a stopping process gets before it is killed forcibly.
const FORCE_KILL_AFTER: Duration = Duration::from_secs(5);
/// How long the exit report waits for the output pipes to drain.
const PIPE_DRAIN_GRACE: Duration = Duration::from_millis(250);

struct Inner {
    /// The `AppProcessState` as it goes on the wire.
    state: Map<String, Value>,
    logs: VecDeque<Value>,
    pid: u32,
    /// A stop was initiated; further stops are no-ops.
    stopping: bool,
    /// The process is gone.
    exited: bool,
}

struct Entry {
    app_id: String,
    inner: Mutex<Inner>,
}

static PROCESSES: LazyLock<Mutex<IndexMap<String, Arc<Entry>>>> = LazyLock::new(|| Mutex::new(IndexMap::new()));

/// Append `chunk` to `logs`, dropping the oldest chunks beyond `max`.
pub fn push_bounded(logs: &mut VecDeque<Value>, chunk: Value, max: usize) {
    logs.push_back(chunk);
    while logs.len() > max {
        logs.pop_front();
    }
}

fn emit_status(payload: Value) {
    bus().emit("process.statusChange", payload);
}

fn emit_log(entry: &Entry, stream: &str, data: &str) {
    let chunk = json!({ "appId": entry.app_id, "stream": stream, "data": data, "ts": now_ms() });
    push_bounded(&mut entry.inner.lock().logs, chunk.clone(), MAX_LOG_LINES);
    bus().emit("process.output", chunk);
}

fn error_state(app_id: &str, message: &str) -> Value {
    let state = json!({ "appId": app_id, "status": "error", "error": message });
    emit_status(state.clone());
    state
}

/// Start an application by id: read its stored definition, spawn the stored command and report the state.
/// An application that is already running is stopped first. Must be called inside the tokio runtime.
pub fn start(ws_id: &str, app_id: &str, mode: &str) -> Value {
    if PROCESSES.lock().contains_key(app_id) {
        stop(app_id);
    }

    let Some(config) = read_entity(ws_id, "applications", app_id) else {
        return error_state(app_id, &format!("Application config not found: {app_id}"));
    };

    let debug = mode == "debug";
    let command = if debug && truthy(&config, "resolvedDebugCommand") {
        opt_str(&config, "resolvedDebugCommand")
    } else {
        opt_str(&config, "resolvedCommand")
    }
    .unwrap_or("")
    .to_string();
    if command.trim().is_empty() {
        return error_state(app_id, "No command to run. Save the configuration to generate the command.");
    }

    let cwd = if truthy(&config, "resolvedCwd") {
        opt_str(&config, "resolvedCwd").unwrap_or("").to_string()
    } else if truthy(&config, "workingDirectory") {
        opt_str(&config, "workingDirectory").unwrap_or("").to_string()
    } else {
        std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| ".".to_string())
    };
    let extra_env = string_map(&config, "resolvedEnv");

    let mut state = Map::new();
    state.insert("appId".into(), json!(app_id));
    state.insert("status".into(), json!("starting"));
    state.insert("startedAt".into(), json!(now_ms()));
    if debug {
        if let Some(port) = config.get("resolvedDebugPort").filter(|v| !v.is_null()) {
            state.insert("debugPort".into(), port.clone());
        }
    }

    let entry = Arc::new(Entry {
        app_id: app_id.to_string(),
        inner: Mutex::new(Inner { state, logs: VecDeque::new(), pid: 0, stopping: false, exited: false }),
    });

    emit_log(&entry, "system", &format!("> {command}\n"));
    emit_log(&entry, "system", &format!("  cwd: {cwd}\n"));
    if debug && truthy(&config, "resolvedDebugPort") {
        let port = js_str(&config["resolvedDebugPort"]);
        emit_log(&entry, "system", &format!("  debug port: {port}\n"));
    }
    emit_log(&entry, "system", "\n");

    let mut cmd = shell_command(&command);
    cmd.current_dir(&cwd)
        .envs(extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let spawned = cmd.spawn();

    let running = if debug { "debugging" } else { "running" };
    let pid = spawned.as_ref().map(|child| child.id().unwrap_or(0)).unwrap_or(0);
    let started = {
        let mut inner = entry.inner.lock();
        inner.pid = pid;
        inner.state.insert("pid".into(), json!(pid));
        inner.state.insert("status".into(), json!(running));
        let mut payload = json!({ "appId": app_id, "status": running, "pid": pid });
        for key in ["debugPort", "startedAt"] {
            if let Some(v) = inner.state.get(key) {
                payload[key] = v.clone();
            }
        }
        payload
    };
    PROCESSES.lock().insert(app_id.to_string(), entry.clone());

    match spawned {
        Ok(mut child) => {
            let readers: Vec<_> = [
                child.stdout.take().map(|pipe| spawn_reader(entry.clone(), "stdout", pipe)),
                child.stderr.take().map(|pipe| spawn_reader(entry.clone(), "stderr", pipe)),
            ]
            .into_iter()
            .flatten()
            .collect();
            tokio::spawn(wait_for_exit(entry.clone(), child, readers));
            emit_status(started);
        }
        Err(err) => {
            emit_status(started);
            let message = err.to_string();
            emit_log(&entry, "system", &format!("\n[ERROR] {message}\n"));
            {
                let mut inner = entry.inner.lock();
                inner.exited = true;
                inner.state.insert("status".into(), json!("error"));
                inner.state.insert("error".into(), json!(message));
            }
            emit_status(json!({ "appId": app_id, "status": "error", "error": message }));
        }
    }
    let state = entry.inner.lock().state.clone();
    Value::Object(state)
}

/// The command string interpreted by the OS shell: `cmd.exe /d /s /c "…"` on Windows, `sh -c …` elsewhere.
fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let shell = std::env::var("COMSPEC").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "cmd.exe".to_string());
        let mut cmd = hidden_command(shell);
        cmd.raw_arg("/d /s /c").raw_arg(format!("\"{command}\""));
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = hidden_command("/bin/sh");
        cmd.arg("-c").arg(command).process_group(0);
        cmd
    }
}

/// Read a pipe until it closes, logging each chunk as it arrives.
fn spawn_reader<R>(entry: Arc<Entry>, stream: &'static str, mut pipe: R) -> tokio::task::JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match pipe.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => emit_log(&entry, stream, &String::from_utf8_lossy(&buf[..n])),
            }
        }
    })
}

/// The exit code and, for a process ended by a signal, the signal's name.
fn exit_parts(status: &std::io::Result<std::process::ExitStatus>) -> (Option<i32>, Option<String>) {
    let Ok(status) = status else { return (None, None) };
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return (None, Some(signal_name(signal)));
        }
    }
    (status.code(), None)
}

#[cfg(unix)]
fn signal_name(signal: i32) -> String {
    match signal {
        libc::SIGHUP => "SIGHUP".into(),
        libc::SIGINT => "SIGINT".into(),
        libc::SIGQUIT => "SIGQUIT".into(),
        libc::SIGABRT => "SIGABRT".into(),
        libc::SIGKILL => "SIGKILL".into(),
        libc::SIGSEGV => "SIGSEGV".into(),
        libc::SIGPIPE => "SIGPIPE".into(),
        libc::SIGALRM => "SIGALRM".into(),
        libc::SIGTERM => "SIGTERM".into(),
        other => format!("SIG{other}"),
    }
}

async fn wait_for_exit(entry: Arc<Entry>, mut child: tokio::process::Child, readers: Vec<tokio::task::JoinHandle<()>>) {
    let status = child.wait().await;
    // The pipes normally close with the process; a grandchild that keeps them open must not hold the report back.
    let _ = tokio::time::timeout(PIPE_DRAIN_GRACE, async {
        for reader in readers {
            let _ = reader.await;
        }
    })
    .await;

    let (code, signal) = exit_parts(&status);
    let message = match &signal {
        Some(signal) => format!("\n[Process terminated by signal {signal}]\n"),
        None => format!("\n[Process exited with code {}]\n", code.map_or("null".to_string(), |c| c.to_string())),
    };
    emit_log(&entry, "system", &message);

    let pid = {
        let mut inner = entry.inner.lock();
        inner.exited = true;
        inner.state.insert("status".into(), json!("exited"));
        inner.state.insert("exitCode".into(), json!(code));
        inner.state.insert("stoppedAt".into(), json!(now_ms()));
        inner.pid
    };
    emit_status(json!({ "appId": entry.app_id, "status": "exited", "exitCode": code, "pid": pid }));
}

/// Stop a running application: ask its process tree to end now, force it after a grace period. A second call
/// for the same run, or a call for a process that already ended, does nothing.
pub fn stop(app_id: &str) {
    let Some(entry) = PROCESSES.lock().get(app_id).cloned() else { return };
    let pid = {
        let mut inner = entry.inner.lock();
        if inner.stopping || inner.exited {
            return;
        }
        inner.stopping = true;
        inner.state.insert("status".into(), json!("stopping"));
        inner.pid
    };
    emit_log(&entry, "system", "\n[Stopping process...]\n");
    emit_status(json!({ "appId": app_id, "status": "stopping" }));

    kill_tree(pid, false);

    std::thread::spawn(move || {
        std::thread::sleep(FORCE_KILL_AFTER);
        if !entry.inner.lock().exited {
            emit_log(&entry, "system", "[Force killing...]\n");
            kill_tree(pid, true);
        }
    });
}

/// End `pid` and its descendants: `taskkill /T /F` on Windows; on Unix a signal to the process group the
/// application was started in (`SIGKILL` when `force`, else `SIGTERM`), or to the process alone if that fails.
fn kill_tree(pid: u32, force: bool) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        let _ = force;
        let _ = crate::utils::process::hidden_std_command("taskkill")
            .args(["/pid", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(unix)]
    {
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        // SAFETY: `kill` takes plain integers and has no memory-safety preconditions.
        unsafe {
            if libc::kill(-(pid as libc::pid_t), signal) != 0 {
                libc::kill(pid as libc::pid_t, signal);
            }
        }
    }
}

/// The state of one application, or `null` if the engine never started it.
pub fn get_state(app_id: &str) -> Value {
    match PROCESSES.lock().get(app_id).cloned() {
        Some(entry) => Value::Object(entry.inner.lock().state.clone()),
        None => Value::Null,
    }
}

/// The state of every application the engine started, in the order they were first started.
pub fn get_all_states() -> Value {
    let entries: Vec<Arc<Entry>> = PROCESSES.lock().values().cloned().collect();
    Value::Array(entries.iter().map(|e| Value::Object(e.inner.lock().state.clone())).collect())
}

/// The buffered output of one application (at most [`MAX_LOG_LINES`] chunks).
pub fn get_logs(app_id: &str) -> Value {
    match PROCESSES.lock().get(app_id).cloned() {
        Some(entry) => Value::Array(entry.inner.lock().logs.iter().cloned().collect()),
        None => Value::Array(Vec::new()),
    }
}

/// Stop every application. Entries stay in the table so their state and logs remain readable.
pub fn stop_all() {
    let ids: Vec<String> = PROCESSES.lock().keys().cloned().collect();
    for id in ids {
        stop(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_data_root;
    use crate::workspace::fs::write_entity;

    #[test]
    fn log_buffer_keeps_the_newest_chunks() {
        let mut logs = VecDeque::new();
        for i in 0..12 {
            push_bounded(&mut logs, json!(i), 5);
        }
        assert_eq!(logs.iter().cloned().collect::<Vec<_>>(), vec![json!(7), json!(8), json!(9), json!(10), json!(11)]);
    }

    const ECHO: &str = "echo hello";
    #[cfg(windows)]
    const SLEEP: &str = "ping -n 30 127.0.0.1";
    #[cfg(not(windows))]
    const SLEEP: &str = "sleep 30";

    fn save_app(ws: &str, id: &str, command: &str) {
        let app = json!({
            "id": id, "name": id, "type": "shell", "workspaceId": ws, "workingDirectory": ".",
            "resolvedCommand": command, "resolvedCwd": ".", "resolvedEnv": { "BIFURC_TEST_VAR": "x" },
        });
        write_entity(ws, "applications", id, &app, None).unwrap();
    }

    async fn wait_for_status(app_id: &str, wanted: &str) -> Value {
        for _ in 0..200 {
            let state = get_state(app_id);
            if state["status"] == wanted {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{app_id} never reached {wanted}: {}", get_state(app_id));
    }

    #[tokio::test]
    async fn runs_to_completion_and_reports_output() {
        let root = scratch_data_root();
        save_app(root.ws_id, "app-echo", ECHO);
        let outputs = Arc::new(Mutex::new(Vec::<Value>::new()));
        let statuses = Arc::new(Mutex::new(Vec::<Value>::new()));
        let (o, s) = (outputs.clone(), statuses.clone());
        let out_id = bus().on("process.output", move |p| {
            if p["appId"] == "app-echo" {
                o.lock().push(p.clone());
            }
        });
        let status_id = bus().on("process.statusChange", move |p| {
            if p["appId"] == "app-echo" {
                s.lock().push(p.clone());
            }
        });

        let started = start(root.ws_id, "app-echo", "run");
        assert_eq!(started["status"], "running");
        assert!(started["pid"].as_u64().unwrap() > 0);
        let done = wait_for_status("app-echo", "exited").await;
        assert_eq!(done["exitCode"], 0);
        assert!(done["stoppedAt"].is_i64() || done["stoppedAt"].is_u64());

        bus().off(out_id);
        bus().off(status_id);

        let outputs = outputs.lock();
        let stdout: String =
            outputs.iter().filter(|p| p["stream"] == "stdout").map(|p| p["data"].as_str().unwrap()).collect();
        assert!(stdout.contains("hello"), "stdout was {stdout:?}");
        assert!(outputs[0]["data"].as_str().unwrap().starts_with(&format!("> {ECHO}")));
        assert_eq!(outputs.last().unwrap()["data"], "\n[Process exited with code 0]\n");

        let statuses: Vec<String> = statuses.lock().iter().map(|p| p["status"].as_str().unwrap().to_string()).collect();
        assert_eq!(statuses, vec!["running", "exited"]);

        let logs = get_logs("app-echo");
        assert_eq!(logs.as_array().unwrap().len(), outputs.len());
        assert!(get_all_states().as_array().unwrap().iter().any(|s| s["appId"] == "app-echo"));
    }

    #[tokio::test]
    async fn missing_config_and_empty_command_are_error_states() {
        let root = scratch_data_root();
        let missing = start(root.ws_id, "nope", "run");
        assert_eq!(missing["status"], "error");
        assert_eq!(missing["error"], "Application config not found: nope");
        save_app(root.ws_id, "app-empty", "   ");
        let empty = start(root.ws_id, "app-empty", "run");
        assert_eq!(empty["error"], "No command to run. Save the configuration to generate the command.");
        assert_eq!(get_state("nope"), Value::Null);
        assert_eq!(get_logs("nope"), json!([]));
    }

    #[tokio::test]
    async fn stop_terminates_a_long_running_child() {
        let root = scratch_data_root();
        save_app(root.ws_id, "app-sleep", SLEEP);
        let started = start(root.ws_id, "app-sleep", "run");
        assert_eq!(started["status"], "running");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(get_state("app-sleep")["status"], "running");

        stop("app-sleep");
        assert_eq!(get_state("app-sleep")["status"], "stopping");
        stop("app-sleep");
        let done = wait_for_status("app-sleep", "exited").await;
        assert!(done.get("stoppedAt").is_some());

        let logs = get_logs("app-sleep");
        let text: String = logs.as_array().unwrap().iter().map(|c| c["data"].as_str().unwrap()).collect();
        assert_eq!(text.matches("[Stopping process...]").count(), 1);

        // Stopping what already ended changes nothing.
        stop("app-sleep");
        assert_eq!(get_state("app-sleep")["status"], "exited");
    }

    #[tokio::test]
    async fn debug_mode_uses_the_debug_command() {
        let root = scratch_data_root();
        let app = json!({
            "id": "app-dbg", "workspaceId": root.ws_id,
            "resolvedCommand": "echo run", "resolvedDebugCommand": "echo debug", "resolvedDebugPort": 9229, "resolvedCwd": ".",
        });
        write_entity(root.ws_id, "applications", "app-dbg", &app, None).unwrap();
        let state = start(root.ws_id, "app-dbg", "debug");
        assert_eq!(state["status"], "debugging");
        assert_eq!(state["debugPort"], 9229);
        wait_for_status("app-dbg", "exited").await;
        let logs = get_logs("app-dbg");
        assert_eq!(logs[0]["data"], "> echo debug\n");
        assert_eq!(logs[2]["data"], "  debug port: 9229\n");
    }

    #[tokio::test]
    async fn bad_working_directory_is_an_error_state() {
        let root = scratch_data_root();
        let app = json!({
            "id": "app-badcwd", "workspaceId": root.ws_id,
            "resolvedCommand": "echo x", "resolvedCwd": root.dir.path().join("no-such-dir").to_string_lossy(),
        });
        write_entity(root.ws_id, "applications", "app-badcwd", &app, None).unwrap();
        let state = start(root.ws_id, "app-badcwd", "run");
        assert_eq!(state["status"], "error");
        assert!(state["error"].is_string());
    }
}
