//! `services define|up|down|restart|remove|ps|show`, `logs` and `port free`: the dev servers of a workspace,
//! which the engine runs in the background (see `engine/src/services/dev.rs`). The output of a service is read
//! from the engine's ring of recent lines; `--errors` keeps only what went wrong.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::{
    capture::{now_ms, number},
    changes::ago,
    commands::{Context, find_one, need, unknown_verb, workspace_id},
    error::{CliError, Result},
    format::{aligned, js_template},
    response::{is_sensitive_name, mask},
    vault,
    wait::{duration, probe_port},
};

/// `services <verb>` for the dev servers; `list` (and anything else) is the port discovery of `commands.rs`.
pub fn services(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub.unwrap_or("list") {
        "define" => define(ctx),
        "up" => up(ctx, false),
        "restart" => up(ctx, true),
        "down" => down(ctx),
        "remove" => remove(ctx),
        "ps" => ps(ctx),
        "show" => show(ctx),
        "snippet" => crate::logtrace::snippet(ctx),
        _ => crate::commands::services(ctx, sub),
    }
}

fn listed(ctx: &mut Context, workspace: &str) -> Result<Vec<Value>> {
    let answer = ctx.request("devservice.list", json!({ "workspaceId": workspace }))?;
    Ok(answer["services"].as_array().cloned().unwrap_or_default())
}

/// The service the first operand names (by name, any case, or a unique prefix).
fn named(ctx: &Context, rows: &[Value]) -> Result<Value> {
    let name = need(ctx.args.positional(2), "service name")?;
    find_one(rows, &name, &["name"], "service", "services ps").cloned()
}

/// The services a command acts on: `--all`, the one named, or (when `all_by_default`) every one.
fn targets(ctx: &Context, rows: &[Value], all_by_default: bool) -> Result<Vec<Value>> {
    if ctx.args.has("all") || (all_by_default && ctx.args.positional(2).is_none()) {
        return Ok(rows.to_vec());
    }
    named(ctx, rows).map(|row| vec![row])
}

fn define(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["cmd", "port", "cwd", "vault", "env", "log-file", "mapping"], 1)?;
    let args = ctx.args;
    let name = need(args.positional(2), "service name")?;
    let cmd = need(args.flag("cmd"), "--cmd <command>")?;
    let mut payload = json!({ "workspaceId": workspace_id(ctx)?, "name": name, "cmd": cmd });
    if let Some(raw) = args.flag("port") {
        match raw.parse::<u16>() {
            Ok(port) if port > 0 => payload["port"] = json!(port),
            _ => return Err(CliError::usage(format!("--port is a port number, not \"{raw}\""))),
        }
    }
    if let Some(cwd) = args.flag("cwd") {
        // Relative to where this is run, stored absolute: the engine runs from elsewhere.
        let full = std::path::absolute(cwd).map_err(|e| CliError::usage(format!("--cwd \"{cwd}\": {e}")))?;
        if !full.is_dir() {
            return Err(CliError::usage(format!("--cwd \"{cwd}\" is not a directory")));
        }
        payload["cwd"] = json!(full.to_string_lossy());
    }
    if let Some(file) = args.flag("log-file") {
        // Like --cwd: the engine tails it from elsewhere.
        let full = std::path::absolute(file).map_err(|e| CliError::usage(format!("--log-file \"{file}\": {e}")))?;
        payload["logFile"] = json!(full.to_string_lossy());
    }
    if let Some(vault) = args.flag("vault") {
        payload["vault"] = json!(vault);
    }
    if let Some(mapping) = args.flag("mapping") {
        payload["mapping"] = json!(mapping);
    }
    if !args.flags("env").is_empty() {
        let mut env = Map::new();
        for pair in args.flags("env") {
            match pair.split_once('=') {
                Some((key, value)) if !key.is_empty() => env.insert(key.to_string(), json!(value)),
                _ => return Err(CliError::usage(format!("--env is KEY=VALUE, not \"{pair}\""))),
            };
        }
        payload["env"] = Value::Object(env);
    }
    let done = ctx.request("devservice.define", payload)?;
    let service = &done["service"];
    Ok(ctx.print(&done, || {
        let port = service["port"].as_u64().map(|p| format!(" · port {p}")).unwrap_or_default();
        format!("defined {name} · {cmd}{port}")
    }))
}

/// The last lines a service wrote, indented, for a reason.
fn tail(ctx: &mut Context, workspace: &str, name: &str, count: u64) -> String {
    let asked = json!({ "workspaceId": workspace, "name": name, "last": count });
    let lines = ctx.request("devservice.logs", asked).map(|got| texts(&got)).unwrap_or_default();
    lines.iter().map(|l| format!("\n  {l}")).collect()
}

/// Wait for the service's port to listen, giving up when it stops or after `timeout`.
fn wait_for(ctx: &mut Context, workspace: &str, service: &Value, timeout: Duration, label: &str) -> Result<String> {
    let (name, port) = (js_template(service.get("name")), service["port"].as_u64().unwrap_or(0) as u16);
    let started = Instant::now();
    loop {
        let last = match probe_port("localhost", port) {
            Ok(()) => return Ok(format!("ready · {name} · port {port} · {:.1}s", started.elapsed().as_secs_f64())),
            Err(reason) => reason,
        };
        if let Some(now) = listed(ctx, workspace)?.iter().find(|s| s["name"] == service["name"])
            && now["state"] != "running"
        {
            let how = state_text(now, now_ms());
            return Err(CliError::msg(format!(
                "{name} {how} before port {port} listened — last output:{}",
                tail(ctx, workspace, &name, 5)
            )));
        }
        let left = timeout.saturating_sub(started.elapsed());
        if left.is_zero() {
            let message = format!(
                "{name}: port {port} not listening after {label} ({last}) — last output:{}",
                tail(ctx, workspace, &name, 5)
            );
            return Err(CliError::network(message));
        }
        std::thread::sleep(Duration::from_millis(250).min(left));
    }
}

/// `services up [<name>|--all] [--wait [--timeout 30s]]` and `services restart <name>`.
fn up(ctx: &mut Context, restart: bool) -> Result<String> {
    ctx.accept(
        if restart {
            &["wait", "timeout", "passphrase-stdin"]
        } else {
            &["all", "wait", "timeout", "passphrase-stdin"]
        },
        1,
    )?;
    if restart {
        need(ctx.args.positional(2), "service name")?;
    }
    let workspace = workspace_id(ctx)?;
    let rows = listed(ctx, &workspace)?;
    let chosen = targets(ctx, &rows, true)?;
    if chosen.is_empty() {
        return Err(CliError::not_found("no services defined (see: bifurc services define --help)"));
    }
    let label = ctx.args.flag("timeout").unwrap_or("30s").to_string();
    let timeout = duration(&label, "timeout")?;
    if ctx.args.has("wait")
        && let Some(none) = chosen.iter().find(|s| s["port"].is_null())
    {
        return Err(CliError::usage(format!(
            "{} has no port to wait for (define it with --port)",
            js_template(none.get("name"))
        )));
    }
    let (mut lines, mut started) = (Vec::new(), Vec::new());
    for service in &chosen {
        let name = js_template(service.get("name"));
        if restart {
            ctx.request("devservice.down", json!({ "workspaceId": workspace, "name": name }))?;
        }
        let mut payload = json!({ "workspaceId": workspace, "name": name });
        if let Some(vault_name) = service["vault"].as_str().filter(|v| !v.is_empty()) {
            let secrets: Map<String, Value> =
                vault::secrets_of(ctx, &workspace, vault_name)?.into_iter().map(|(k, v)| (k, json!(v))).collect();
            payload["secrets"] = Value::Object(secrets);
        }
        let done = ctx.request("devservice.up", payload)?["service"].clone();
        let verb = if service["state"] == "running" && !restart { "already running" } else { "started" };
        lines.push(format!("{verb} {name} (pid {})", js_template(done.get("pid"))));
        started.push(done);
    }
    if ctx.args.has("wait") {
        for service in &chosen {
            lines.push(wait_for(ctx, &workspace, service, timeout, &label)?);
        }
    }
    Ok(ctx.print(&json!({ "services": started }), || lines.join("\n")))
}

/// `services down <name>|--all`.
fn down(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["all"], 1)?;
    let workspace = workspace_id(ctx)?;
    let rows = listed(ctx, &workspace)?;
    let (mut lines, mut stopped) = (Vec::new(), Vec::new());
    for service in targets(ctx, &rows, false)? {
        let name = js_template(service.get("name"));
        let done = ctx.request("devservice.down", json!({ "workspaceId": workspace, "name": name }))?;
        lines.push(format!("stopped {name}"));
        stopped.push(done["service"].clone());
    }
    Ok(ctx.print(&json!({ "services": stopped }), || {
        if lines.is_empty() { "no services defined".into() } else { lines.join("\n") }
    }))
}

fn remove(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 1)?;
    let workspace = workspace_id(ctx)?;
    let rows = listed(ctx, &workspace)?;
    let name = js_template(named(ctx, &rows)?.get("name"));
    let done = ctx.request("devservice.remove", json!({ "workspaceId": workspace, "name": name }))?;
    Ok(ctx.print(&done, || format!("removed {name}")))
}

/// `3s`, `12m`, `2h`, `3d`.
fn span(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// `running`, `stopped`, `crashed (exit 1) 12s ago`.
fn state_text(service: &Value, now: i64) -> String {
    let state = service["state"].as_str().unwrap_or("stopped");
    match (state, service["exitCode"].as_i64(), service["endedAt"].as_i64()) {
        ("crashed", Some(code), Some(ended)) => format!("crashed (exit {code}) {}", ago(now - ended)),
        _ => state.to_string(),
    }
}

/// `services ps`: a line per service, and under a crashed one the last 5 error lines.
fn ps(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let workspace = workspace_id(ctx)?;
    let mut rows = listed(ctx, &workspace)?;
    let now = now_ms();
    let mut cells = Vec::new();
    let mut errors: Vec<Vec<String>> = Vec::new();
    for service in &mut rows {
        let running = service["state"] == "running";
        let health = match (running, service["listening"].as_bool()) {
            (true, Some(true)) => "listening",
            (true, Some(false)) => "not listening",
            _ => "-",
        };
        cells.push(vec![
            js_template(service.get("name")),
            state_text(service, now),
            service["port"].as_u64().map_or("-".into(), |p| p.to_string()),
            if running { js_template(service.get("pid")) } else { "-".into() },
            service["startedAt"].as_i64().filter(|_| running).map_or("-".into(), |t| span(now - t)),
            health.to_string(),
        ]);
        let mut last = Vec::new();
        if service["state"] == "crashed" {
            let asked = json!({ "workspaceId": workspace, "name": service["name"], "last": 200 });
            last = error_lines(&texts(&ctx.request("devservice.logs", asked)?));
            last.drain(..last.len().saturating_sub(5));
            service["lastErrors"] = json!(last);
        }
        errors.push(last);
    }
    Ok(ctx.print(&json!({ "schemaVersion": 1, "services": rows }), || {
        if cells.is_empty() {
            return "no services defined (see: bifurc services define --help)".into();
        }
        let table = aligned(&cells);
        let mut out = Vec::new();
        for (line, last) in table.lines().zip(&errors) {
            out.push(line.to_string());
            out.extend(last.iter().map(|l| format!("    {l}")));
        }
        out.join("\n")
    }))
}

/// `services show <name>`: the definition, with values that look secret masked.
fn show(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 1)?;
    let workspace = workspace_id(ctx)?;
    let rows = listed(ctx, &workspace)?;
    let mut service = named(ctx, &rows)?;
    for (key, value) in service["env"].as_object_mut().into_iter().flatten() {
        if is_sensitive_name(key) {
            *value = json!(mask(value.as_str().unwrap_or_default()));
        }
    }
    Ok(ctx.print(&service, || {
        let mut lines = vec![format!("{}  {}", js_template(service.get("name")), js_template(service.get("cmd")))];
        for key in ["port", "cwd", "logFile", "vault", "mapping"] {
            if !service[key].is_null() {
                lines.push(format!("{key}: {}", js_template(service.get(key))));
            }
        }
        for (key, value) in service["env"].as_object().into_iter().flatten() {
            lines.push(format!("env {key}={}", js_template(Some(value))));
        }
        lines.join("\n")
    }))
}

fn texts(logs: &Value) -> Vec<String> {
    logs["lines"].as_array().into_iter().flatten().map(|l| js_template(l.get("text"))).collect()
}

/// Whether a line is at warn or error level.
fn is_error(line: &str) -> bool {
    let lower = line.to_lowercase();
    ["error", "err!", "warn", "fatal", "exception", "panic", "critical", "traceback"].iter().any(|w| lower.contains(w))
}

/// Whether a line continues a stack trace: indented, `at …`, `File "…"`, `Caused by`.
fn is_continuation(line: &str) -> bool {
    line.starts_with(char::is_whitespace) || ["at ", "File \"", "Caused by", "... "].iter().any(|p| line.starts_with(p))
}

/// The warn/error lines of `lines` with the stack traces under them, consecutive repeats as `line ×42`.
fn error_lines(lines: &[String]) -> Vec<String> {
    let (mut kept, mut previous): (Vec<(&str, usize)>, bool) = (Vec::new(), false);
    for line in lines {
        previous = is_error(line) || (previous && is_continuation(line));
        if !previous {
            continue;
        }
        match kept.last_mut() {
            Some((text, count)) if *text == line => *count += 1,
            _ => kept.push((line, 1)),
        }
    }
    kept.into_iter()
        .map(|(text, count)| if count > 1 { format!("{text} ×{count}") } else { text.to_string() })
        .collect()
}

/// Unix milliseconds `--since` (`30s`, `2m`, `1h`) ago.
fn since_ms(raw: &str, now: i64) -> Result<i64> {
    Ok(now - duration(raw, "since")?.as_millis() as i64)
}

/// `logs <name> [--errors] [--since 2m] [--grep x] [--last 50]`.
pub fn logs(ctx: &mut Context) -> Result<String> {
    ctx.accept_top(&["errors", "since", "grep", "last"], 1)?;
    let workspace = workspace_id(ctx)?;
    let rows = listed(ctx, &workspace)?;
    let name = need(ctx.args.positional(1), "service name")?;
    let name = js_template(find_one(&rows, &name, &["name"], "service", "services ps")?.get("name"));
    let (errors, grep) = (ctx.args.has("errors"), ctx.args.flag("grep").map(str::to_lowercase));
    let last = number(ctx, "last")?.unwrap_or(50);
    let mut payload = json!({ "workspaceId": workspace, "name": name });
    if let Some(raw) = ctx.args.flag("since") {
        payload["sinceTs"] = json!(since_ms(raw, now_ms())?);
    }
    if !errors && grep.is_none() {
        payload["last"] = json!(last);
    }
    let got = ctx.request("devservice.logs", payload)?;
    let mut lines = texts(&got);
    if errors {
        lines = error_lines(&lines);
    }
    if let Some(word) = &grep {
        lines.retain(|l| l.to_lowercase().contains(word));
    }
    lines.drain(..lines.len().saturating_sub(last as usize));
    let shown = json!({ "schemaVersion": 1, "lines": lines });
    Ok(ctx.print(&shown, || if lines.is_empty() { "no log lines".to_string() } else { lines.join("\n") }))
}

/// `port free <port> [--kill]`.
pub fn port(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    if sub != Some("free") {
        return Err(unknown_verb("port", sub.unwrap_or("")));
    }
    ctx.accept(&["kill"], 1)?;
    let raw = need(ctx.args.positional(2), "port number")?;
    let port: u16 = raw
        .parse()
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| CliError::usage(format!("a port is 1-65535, not \"{raw}\"")))?;
    let kill = ctx.args.has("kill");
    let done = ctx.request("devservice.freePort", json!({ "port": port, "kill": kill }))?;
    Ok(ctx.print(&done, || {
        let who = format!("{} (pid {})", js_template(done.get("processName")), js_template(done.get("pid")));
        match (done["held"].as_bool().unwrap_or(false), kill) {
            (false, _) => "free".to_string(),
            (true, false) => format!("{port} held by {who} — stop it with: bifurc port free {port} --kill"),
            (true, true) => format!("killed {who}; {port} is free"),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run};

    fn strings(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    fn row(name: &str, state: &str, port: Value) -> Value {
        json!({"name": name, "cmd": "npm run dev", "port": port, "state": state, "pid": 77,
               "startedAt": now_ms() - 125_000, "listening": true})
    }

    fn engine(rows: Vec<Value>) -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("devservice.list", Ok(json!({"services": rows}))),
            ("devservice.define", Ok(json!({"service": {"name": "api", "port": 3000}}))),
            ("devservice.up", Ok(json!({"service": {"name": "api", "pid": 77}}))),
            ("devservice.down", Ok(json!({"service": {"name": "api"}}))),
            ("devservice.logs", Ok(json!({"lines": [{"text": "Error: boom"}]}))),
        ]
    }

    #[test]
    fn errors_keep_warn_and_error_lines_with_their_stack_and_collapse_repeats() {
        let lines = strings(&[
            "ready on 3000",
            "GET /a 200",
            "Error: boom",
            "    at handler (app.js:1:2)",
            "    at next (router.js:3:4)",
            "    indented info, not part of a trace",
            "WARN slow query",
            "WARN slow query",
            "WARN slow query",
            "Traceback (most recent call last):",
            "  File \"app.py\", line 3, in run",
            "ValueError: bad",
        ]);
        assert_eq!(
            error_lines(&lines),
            [
                "Error: boom",
                "    at handler (app.js:1:2)",
                "    at next (router.js:3:4)",
                "    indented info, not part of a trace",
                "WARN slow query ×3",
                "Traceback (most recent call last):",
                "  File \"app.py\", line 3, in run",
                "ValueError: bad",
            ]
        );
        assert!(error_lines(&strings(&["    at lonely (x.js:1:1)", "all fine"])).is_empty());
    }

    #[test]
    fn since_is_a_time_ago() {
        assert_eq!(since_ms("2m", 1_000_000).unwrap(), 880_000);
        assert_eq!(since_ms("1h", 4_000_000).unwrap(), 400_000);
        assert_eq!(since_ms("soon", 0).unwrap_err().code(), 2);
    }

    #[test]
    fn define_stores_the_cwd_absolute_and_validates_the_rest() {
        let argv = [
            "services",
            "define",
            "api",
            "--cmd",
            "npm run dev",
            "--port",
            "3000",
            "--cwd",
            ".",
            "--env",
            "A=1",
            "--env",
            "B=x=y",
            "--vault",
            "keys",
        ];
        let (out, calls) = run(&argv, engine(vec![]), services_define);
        assert_eq!(out.unwrap(), "defined api · npm run dev · port 3000");
        let sent = &payloads(&calls, "devservice.define")[0];
        assert!(std::path::Path::new(sent["cwd"].as_str().unwrap()).is_absolute());
        assert_eq!(
            (&sent["env"], &sent["vault"], &sent["port"]),
            (&json!({"A": "1", "B": "x=y"}), &json!("keys"), &json!(3000))
        );
        for bad in [
            vec!["services", "define", "api"],
            vec!["services", "define", "api", "--cmd", "x", "--port", "99999"],
            vec!["services", "define", "api", "--cmd", "x", "--env", "oops"],
            vec!["services", "define", "api", "--cmd", "x", "--cwd", "no/such/dir"],
            vec!["services", "define", "api", "--cmd", "x", "--bogus", "1"],
        ] {
            assert_eq!(run(&bad, engine(vec![]), services_define).0.unwrap_err().code(), 2, "{bad:?}");
        }
    }

    fn services_define(ctx: &mut Context) -> Result<String> {
        services(ctx, Some("define"))
    }

    #[test]
    fn up_starts_the_named_service_or_all_and_an_unknown_name_is_not_found() {
        let rows = vec![row("api", "stopped", json!(3000)), row("web", "running", json!(5173))];
        let go = |argv: &[&str]| run(argv, engine(rows.clone()), |c| services(c, Some("up")));
        let (out, calls) = go(&["services", "up", "ap"]);
        assert_eq!(out.unwrap(), "started api (pid 77)");
        assert_eq!(payloads(&calls, "devservice.up"), [json!({"workspaceId": "w1", "name": "api"})]);
        let (out, calls) = go(&["services", "up", "--all"]);
        assert_eq!(out.unwrap(), "started api (pid 77)\nalready running web (pid 77)");
        assert_eq!(payloads(&calls, "devservice.up").len(), 2);
        assert_eq!(go(&["services", "up", "nope"]).0.unwrap_err().code(), 3);
    }

    #[test]
    fn up_wait_times_out_with_exit_4_and_the_last_output() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let rows = vec![row("api", "running", json!(port))];
        let argv = ["services", "up", "api", "--wait", "--timeout", "0"];
        let err = run(&argv, engine(rows), |c| services(c, Some("up"))).0.unwrap_err();
        assert_eq!(err.code(), 4);
        assert!(
            err.line().contains("not listening after 0") && err.line().ends_with("last output:\n  Error: boom"),
            "{}",
            err.line()
        );
        let no_port = vec![row("api", "stopped", Value::Null)];
        let err = run(&argv, engine(no_port), |c| services(c, Some("up"))).0.unwrap_err();
        assert_eq!(err.code(), 2);
    }

    #[test]
    fn ps_aligns_a_line_each_and_shows_a_crash_with_its_last_error_lines() {
        let mut crashed = row("worker", "crashed", Value::Null);
        crashed["exitCode"] = json!(1);
        crashed["endedAt"] = json!(now_ms() - 12_000);
        let rows = vec![row("api", "running", json!(3000)), crashed];
        let (out, calls) = run(&["services", "ps"], engine(rows), |c| services(c, Some("ps")));
        let out = out.unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0].split_whitespace().collect::<Vec<_>>(),
            ["api", "running", "3000", "77", "2m", "listening"]
        );
        assert_eq!(
            lines[1].split_whitespace().collect::<Vec<_>>(),
            ["worker", "crashed", "(exit", "1)", "12s", "ago", "-", "-", "-", "-"]
        );
        assert_eq!(lines[2], "    Error: boom");
        assert_eq!(payloads(&calls, "devservice.logs").len(), 1);
    }

    #[test]
    fn logs_default_to_the_last_50_lines_and_filter_on_the_cli() {
        let lines = json!({"lines": [{"text": "a ok"}, {"text": "B failed Error"}, {"text": "c ok"}]});
        let mut answers = engine(vec![row("api", "running", json!(3000))]);
        answers.insert(0, ("devservice.logs", Ok(lines)));
        let (out, calls) = run(&["logs", "api"], answers.clone(), logs);
        assert_eq!(
            out.unwrap(),
            "a ok
B failed Error
c ok"
        );
        assert_eq!(payloads(&calls, "devservice.logs")[0]["last"], 50);
        let (out, calls) =
            run(&["logs", "api", "--errors", "--grep", "failed", "--last", "1", "--since", "2m"], answers, logs);
        assert_eq!(out.unwrap(), "B failed Error");
        let sent = &payloads(&calls, "devservice.logs")[0];
        assert!(sent.get("last").is_none() && sent["sinceTs"].as_i64().unwrap() > 1_700_000_000_000);
    }

    #[test]
    fn show_masks_sensitive_env_values() {
        let mut service = row("api", "stopped", json!(3000));
        service["env"] = json!({"API_TOKEN": "s3cret", "MODE": "dev"});
        let (out, _) = run(&["services", "show", "api"], engine(vec![service]), |c| services(c, Some("show")));
        let out = out.unwrap();
        assert!(
            out.contains("env API_TOKEN=••••(6)") && out.contains("env MODE=dev") && !out.contains("s3cret"),
            "{out}"
        );
    }

    #[test]
    fn port_free_says_free_names_the_holder_or_kills_it() {
        let held = json!({"held": true, "port": 3000, "pid": 42, "processName": "node", "killed": false});
        let answers = |v: Value| vec![("devservice.freePort", Ok(v))];
        let go = |argv: &[&str], v: Value| run(argv, answers(v), |c| port(c, Some("free")));
        assert_eq!(go(&["port", "free", "3000"], json!({"held": false})).0.unwrap(), "free");
        assert_eq!(
            go(&["port", "free", "3000"], held.clone()).0.unwrap(),
            "3000 held by node (pid 42) — stop it with: bifurc port free 3000 --kill"
        );
        let (out, calls) = go(&["port", "free", "3000", "--kill"], held);
        assert_eq!(out.unwrap(), "killed node (pid 42); 3000 is free");
        assert_eq!(payloads(&calls, "devservice.freePort"), [json!({"port": 3000, "kill": true})]);
        assert_eq!(go(&["port", "free", "x"], json!({})).0.unwrap_err().code(), 2);
    }
}
