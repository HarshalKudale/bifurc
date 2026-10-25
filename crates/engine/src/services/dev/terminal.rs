//! `terminal.*`: programs a client runs by hand in the app's terminal dock. They are dev server runs without a
//! definition: nothing is saved, the program's input stays open for `terminal.input`, and with no command it is
//! the OS shell. There is no pseudo-terminal, so a program that draws on the screen shows its plain lines only.
//! They end with the engine.

use std::{
    process::Stdio,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::AsyncWriteExt,
    sync::{mpsc, oneshot},
};

use super::{Run, Shared, flush, lock, pump, push};
use crate::{
    kernel::{CommandRegistry, bus},
    mcp::client::shell_command,
    protocol::{EngineError, Result},
    utils::{
        id::now_ms,
        json::{opt_str, str_of},
    },
};

struct Term {
    id: String,
    title: String,
    cmd: String,
    cwd: String,
    run: Shared,
    /// Lines for the program's input.
    input: mpsc::UnboundedSender<String>,
    /// Whether the program shows what it reads; when it does not, the engine puts each line in the output.
    echoes: bool,
}

/// The open terminals, in the order they were opened.
static TERMS: LazyLock<Mutex<Vec<Arc<Term>>>> = LazyLock::new(Default::default);
static NEXT: AtomicU32 = AtomicU32::new(1);

fn find(params: &Value) -> Result<Arc<Term>> {
    let id = str_of(params, "id");
    lock(&TERMS)
        .iter()
        .find(|term| term.id == id)
        .cloned()
        .ok_or_else(|| EngineError::not_found(format!("No terminal \"{id}\".")))
}

/// A terminal as clients see it.
fn describe(term: &Term) -> Value {
    let run = lock(&term.run);
    json!({
        "id": term.id, "title": term.title, "cmd": term.cmd, "cwd": term.cwd,
        "state": if run.running { "running" } else { "exited" },
        "pid": run.pid, "startedAt": run.started,
        "endedAt": (!run.running).then_some(run.ended), "exitCode": (!run.running).then_some(run.code),
    })
}

/// Tell clients a terminal changed; `terminal` is its description, null once it is closed.
fn announce(id: &str, terminal: Value) {
    bus().emit("terminal.changed", json!({ "id": id, "terminal": terminal }));
}

async fn open(params: &Value) -> Result<Value> {
    let cmd = str_of(params, "cmd").trim().to_string();
    let shell = cmd.is_empty();
    let line = match (shell, cfg!(windows)) {
        (false, _) => cmd.clone(),
        (true, true) => "cmd".to_string(),
        (true, false) => r#"exec "${SHELL:-/bin/sh}""#.to_string(),
    };
    // Where a terminal would start, not the engine's own directory.
    let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    let cwd = opt_str(params, "cwd").filter(|c| !c.is_empty()).map_or(home, str::to_string);
    if !cwd.is_empty() && !std::path::Path::new(&cwd).is_dir() {
        return Err(EngineError::bad_request(format!("The directory \"{cwd}\" does not exist.")));
    }
    let mut wrap = shell_command(&line);
    let command = wrap.command_mut();
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if !cwd.is_empty() {
        command.current_dir(&cwd);
    }
    let mut child = wrap.spawn().map_err(|e| EngineError::upstream(format!("Cannot start \"{line}\": {e}")))?;
    let (stop, stopped) = oneshot::channel();
    let run: Shared = Arc::new(Mutex::new(Run::new(child.id().unwrap_or(0), stop)));
    let readers: Vec<_> = [
        child.stdout().take().map(|s| pump(s, "stdout", run.clone())),
        child.stderr().take().map(|s| pump(s, "stderr", run.clone())),
    ]
    .into_iter()
    .flatten()
    .collect();
    let (input, mut lines) = mpsc::unbounded_channel::<String>();
    if let Some(mut stdin) = child.stdin().take() {
        tokio::spawn(async move {
            while let Some(text) = lines.recv().await {
                if stdin.write_all(format!("{text}\n").as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
    }
    let id = format!("t{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let title = opt_str(params, "title")
        .filter(|t| !t.is_empty())
        .map_or_else(|| if shell { "Terminal".to_string() } else { cmd.clone() }, str::to_string);
    // cmd.exe prints the prompt and the line it reads, which is what a terminal shows.
    let term =
        Arc::new(Term { id: id.clone(), title, cmd, cwd, run: run.clone(), input, echoes: shell && cfg!(windows) });
    lock(&TERMS).push(term.clone());
    flush("terminal.output", json!({ "id": id }), run.clone());
    let watched = term.clone();
    tokio::spawn(async move {
        let waited = tokio::select! { status = child.wait() => Some(status), _ = stopped => None };
        let status = match waited {
            Some(status) => status,
            None => {
                let _ = child.start_kill();
                child.wait().await
            }
        };
        let _ = tokio::time::timeout(Duration::from_millis(300), futures::future::join_all(readers)).await;
        {
            let mut run = lock(&watched.run);
            run.code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
            (run.ended, run.running) = (now_ms(), false);
        }
        announce(&watched.id, describe(&watched));
    });
    announce(&term.id, describe(&term));
    Ok(describe(&term))
}

/// End the program if it runs, and forget the terminal.
async fn close(term: &Term) {
    let stop = lock(&term.run).stop.take();
    if let Some(stop) = stop {
        let _ = stop.send(());
        for _ in 0..200 {
            if !lock(&term.run).running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    lock(&TERMS).retain(|other| other.id != term.id);
    announce(&term.id, Value::Null);
}

pub(super) fn register(registry: &mut CommandRegistry) {
    registry.register("terminal.open", |params, _| async move { Ok(json!({ "terminal": open(&params).await? })) });

    registry.register("terminal.list", |_, _| async move {
        let terminals: Vec<Value> = lock(&TERMS).iter().map(|term| describe(term)).collect();
        Ok(json!({ "terminals": terminals }))
    });

    registry.register("terminal.logs", |params, _| async move {
        let term = find(&params)?;
        let mut lines: Vec<Value> =
            lock(&term.run).lines.iter().map(|l| json!({ "ts": l.ts, "stream": l.stream, "text": l.text })).collect();
        if let Some(last) = params["last"].as_u64() {
            lines.drain(..lines.len().saturating_sub(last as usize));
        }
        Ok(json!({ "lines": lines }))
    });

    registry.register("terminal.input", |params, _| async move {
        let term = find(&params)?;
        if !lock(&term.run).running {
            return Err(EngineError::conflict("The program has ended."));
        }
        let text = str_of(&params, "text").to_string();
        if !term.echoes {
            push(&term.run, "stdout", format!("> {text}"));
        }
        term.input.send(text).map_err(|_| EngineError::conflict("The program has ended."))?;
        Ok(json!({ "ok": true }))
    });

    registry.register("terminal.close", |params, _| async move {
        let term = find(&params)?;
        close(&term).await;
        Ok(json!({ "closed": term.id }))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::CommandContext;

    async fn call(registry: &Arc<CommandRegistry>, name: &str, payload: Value) -> Result<Value> {
        registry.invoke(name, payload, CommandContext::internal(registry.clone())).await
    }

    async fn texts(registry: &Arc<CommandRegistry>, id: &Value, wanted: &str) -> Vec<String> {
        let mut seen = Vec::new();
        for _ in 0..100 {
            let logs = call(registry, "terminal.logs", json!({ "id": id })).await.unwrap();
            seen = logs["lines"].as_array().unwrap().iter().map(|l| l["text"].as_str().unwrap().to_string()).collect();
            if seen.iter().any(|line| line.contains(wanted)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        seen
    }

    #[tokio::test]
    async fn a_command_runs_in_a_tab_and_a_shell_takes_lines_until_it_is_closed() {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);

        let ran = call(&registry, "terminal.open", json!({ "cmd": "echo hello" })).await.unwrap();
        let id = ran["terminal"]["id"].clone();
        assert_eq!(ran["terminal"]["title"], "echo hello");
        assert!(texts(&registry, &id, "hello").await.contains(&"hello".to_string()));
        for _ in 0..100 {
            let listed = call(&registry, "terminal.list", json!({})).await.unwrap();
            if listed["terminals"][0]["state"] == "exited" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let listed = call(&registry, "terminal.list", json!({})).await.unwrap();
        assert_eq!(
            (&listed["terminals"][0]["state"], &listed["terminals"][0]["exitCode"]),
            (&json!("exited"), &json!(0))
        );
        let ended = call(&registry, "terminal.input", json!({ "id": id, "text": "x" })).await;
        assert!(ended.is_err());
        call(&registry, "terminal.close", json!({ "id": id })).await.unwrap();

        let shell = call(&registry, "terminal.open", json!({})).await.unwrap();
        let id = shell["terminal"]["id"].clone();
        assert_eq!(shell["terminal"]["title"], "Terminal");
        call(&registry, "terminal.input", json!({ "id": id, "text": "echo from-the-shell" })).await.unwrap();
        let seen = texts(&registry, &id, "from-the-shell").await;
        assert!(seen.iter().any(|line| line == "from-the-shell"), "{seen:?}");
        call(&registry, "terminal.close", json!({ "id": id })).await.unwrap();
        let listed = call(&registry, "terminal.list", json!({})).await.unwrap();
        assert!(listed["terminals"].as_array().unwrap().is_empty());
        assert!(call(&registry, "terminal.logs", json!({ "id": id })).await.is_err());
    }

    #[tokio::test]
    async fn a_directory_that_does_not_exist_is_refused() {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let refused = call(&registry, "terminal.open", json!({ "cmd": "echo x", "cwd": "no/such/dir" })).await;
        assert!(refused.is_err());
    }
}
