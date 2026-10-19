//! `bifurc wait`: block until a URL answers, a TCP port accepts, or a new matching capture entry arrives.

use std::{
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    capture::{filters, now_ms, short_url, summaries},
    commands::{Context, find_one, workspace_id},
    error::{CliError, Result},
    format::js_template,
};

/// `500ms`, `30s`, `2m`, `1h`, or a plain number of seconds.
pub(crate) fn duration(raw: &str, flag: &str) -> Result<Duration> {
    let (number, unit) = match raw.strip_suffix("ms") {
        Some(number) => (number, 0.001),
        None => match (raw.strip_suffix('m'), raw.strip_suffix('s'), raw.strip_suffix('h')) {
            (Some(number), ..) => (number, 60.0),
            (_, Some(number), _) => (number, 1.0),
            (.., Some(number)) => (number, 3600.0),
            _ => (raw, 1.0),
        },
    };
    number
        .parse::<f64>()
        .ok()
        .filter(|n| *n >= 0.0 && n.is_finite())
        .map(|n| Duration::from_secs_f64(n * unit))
        .ok_or_else(|| CliError::usage(format!("--{flag} is 500ms, 30s, 2m, 1h or a number of seconds, not \"{raw}\"")))
}

/// One try at connecting to `host:port`; why not, when it fails.
pub(crate) fn probe_port(host: &str, port: u16) -> std::result::Result<(), String> {
    let mut addrs = (host, port).to_socket_addrs().map_err(|e| e.to_string())?;
    let addr = addrs.next().ok_or("no address")?;
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).map(|_| ()).map_err(|e| match e.kind() {
        std::io::ErrorKind::ConnectionRefused => "connection refused".to_string(),
        std::io::ErrorKind::TimedOut => "timed out".to_string(),
        _ => e.to_string(),
    })
}

/// What one attempt found: the success line, or why not yet.
type Attempt = std::result::Result<String, String>;

/// One try at the thing waited for.
type Probe = Box<dyn FnMut(&mut Context) -> Result<Attempt>>;

/// The URL a target names: itself, a saved health check's URL, or a bare address (`api.localhost/health`).
fn url_of(ctx: &mut Context, target: &str) -> Result<String> {
    let workspace = workspace_id(ctx)?;
    let url = if target.contains("://") {
        target.to_string()
    } else {
        let saved = ctx.request("healthbar.getServices", json!({ "workspaceId": workspace }))?;
        let all = saved.as_array().map(Vec::as_slice).unwrap_or_default();
        match find_one(all, target, &["name"], "health check", "health") {
            Ok(check) => js_template(check.get("url")),
            Err(_) if target.contains(['.', ':', '/']) => format!("http://{target}"),
            Err(err) => return Err(err),
        }
    };
    let resolved = ctx.request("env.resolveText", json!({ "workspaceId": workspace, "text": url }))?;
    Ok(resolved.get("text").and_then(Value::as_str).unwrap_or(&url).to_string())
}

/// `wait`: exits 0 with one `ready · …` line, or 4 after `--timeout` with the last reason.
pub fn wait(ctx: &mut Context) -> Result<String> {
    ctx.accept_top(
        // `--healthy` is what waiting on a URL already means; it is taken for curl-style habits.
        &["port", "host", "capture", "status", "timeout", "interval", "method", "path", "origin", "grep", "healthy"],
        1,
    )?;
    let args = ctx.args;
    let label = args.flag("timeout").unwrap_or("30s").to_string();
    let (timeout, interval) =
        (duration(&label, "timeout")?, duration(args.flag("interval").unwrap_or("500ms"), "interval")?);
    let started = Instant::now();
    let took = || format!("{:.1}s", started.elapsed().as_secs_f64());

    let mut attempt: Probe = if let Some(port) = args.flag("port") {
        let port: u16 =
            port.parse().map_err(|_| CliError::usage(format!("--port is a port number, not \"{port}\"")))?;
        let host = args.flag("host").unwrap_or("127.0.0.1").to_string();
        Box::new(move |_| {
            // No engine: the CLI connects itself.
            let connected = probe_port(&host, port);
            Ok(connected.map(|_| format!("port {port}")))
        })
    } else if args.has("capture") {
        let mut payload = filters(ctx)?;
        payload["since"] = json!(now_ms());
        payload["last"] = json!(1);
        let workspace = workspace_id(ctx)?;
        Box::new(move |ctx| {
            let answer = summaries(ctx, &workspace, payload.clone())?;
            Ok(match answer["entries"].as_array().and_then(|entries| entries.first()) {
                Some(entry) => {
                    let id = js_template(entry.get("id"));
                    let status =
                        entry.get("status").and_then(Value::as_u64).map_or("ERR".to_string(), |s| s.to_string());
                    Ok(format!(
                        "#{} {} {} {status}",
                        id.get(id.len().saturating_sub(4)..).unwrap_or(&id),
                        js_template(entry.get("method")),
                        short_url(&js_template(entry.get("url")))
                    ))
                }
                None => Err("no matching request yet".to_string()),
            })
        })
    } else {
        let target = args.positional(1).ok_or_else(|| {
            CliError::usage(
                "missing what to wait for: <url|health check>, --port <n> or --capture (see: bifurc wait --help)",
            )
        })?;
        let url = url_of(ctx, target)?;
        let wanted: Option<u64> = args
            .flag("status")
            .map(|raw| raw.parse().map_err(|_| CliError::usage(format!("--status is a status code, not \"{raw}\""))))
            .transpose()?;
        Box::new(move |ctx| {
            let result = ctx.request("healthbar.checkUrl", json!({ "url": url }))?;
            Ok(match (result.get("statusCode").and_then(Value::as_u64), result.get("error")) {
                (Some(status), _) if wanted.is_none_or(|w| w == status) => Ok(format!("{url} {status}")),
                (Some(status), _) => Err(format!("status {status}")),
                (None, error) => Err(error.and_then(Value::as_str).unwrap_or("no answer").to_lowercase()),
            })
        })
    };

    loop {
        let last = match attempt(ctx)? {
            Ok(found) => return Ok(format!("ready · {found} · {}", took())),
            Err(reason) => reason,
        };
        let left = timeout.saturating_sub(started.elapsed());
        if left.is_zero() {
            let shown = if label.ends_with(|c: char| c.is_ascii_digit()) { format!("{label}s") } else { label };
            return Err(CliError::network(format!("timed out after {shown} — last: {last}")));
        }
        std::thread::sleep(interval.min(left));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        run(argv, answers, wait)
    }

    fn url_engine(probe: Value) -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("env.resolveText", Ok(json!({"text": "http://api.localhost/health"}))),
            ("healthbar.checkUrl", Ok(probe)),
        ]
    }

    #[test]
    fn durations_are_ms_seconds_minutes_or_a_bare_number_of_seconds() {
        let d = |raw| duration(raw, "timeout").unwrap();
        assert_eq!(
            (d("500ms"), d("30s"), d("2m"), d("1.5")),
            (
                Duration::from_millis(500),
                Duration::from_secs(30),
                Duration::from_secs(120),
                Duration::from_millis(1500)
            )
        );
        assert!(duration("soon", "timeout").is_err());
    }

    #[test]
    fn a_url_that_answers_is_ready_with_any_status_or_the_one_asked_for() {
        let (out, calls) = go(&["wait", "http://api.localhost/health"], url_engine(json!({"statusCode": 503})));
        assert!(out.unwrap().starts_with("ready · http://api.localhost/health 503 · "));
        assert_eq!(payloads(&calls, "healthbar.checkUrl")[0], json!({"url": "http://api.localhost/health"}));
        let (out, _) = go(&["wait", "http://x/", "--status", "200"], url_engine(json!({"statusCode": 200})));
        assert!(out.unwrap().contains(" 200 · "));
    }

    #[test]
    fn a_url_that_never_answers_times_out_with_exit_4_and_the_last_reason() {
        let (out, _) = go(
            &["wait", "http://x/", "--timeout", "0"],
            url_engine(json!({"statusCode": null, "error": "Connection refused"})),
        );
        let err = out.unwrap_err();
        assert_eq!((err.code(), err.line()), (4, "bifurc: timed out after 0s — last: connection refused".to_string()));
        let (out, _) = go(
            &["wait", "http://x/", "--status", "200", "--timeout", "5ms", "--interval", "1ms"],
            url_engine(json!({"statusCode": 404})),
        );
        assert!(out.unwrap_err().line().ends_with("after 5ms — last: status 404"));
    }

    #[test]
    fn a_health_check_name_means_its_url_and_a_bare_address_gets_http() {
        let mut answers = url_engine(json!({"statusCode": 200}));
        answers.push(("healthbar.getServices", Ok(json!([{"id": "h1", "name": "api", "url": "{{baseUrl}}/health"}]))));
        let (_, calls) = go(&["wait", "api"], answers);
        assert_eq!(payloads(&calls, "env.resolveText")[0]["text"], "{{baseUrl}}/health");
        let mut answers = url_engine(json!({"statusCode": 200}));
        answers.push(("healthbar.getServices", Ok(json!([]))));
        let (_, calls) = go(&["wait", "api.localhost/health"], answers);
        assert_eq!(payloads(&calls, "env.resolveText")[0]["text"], "http://api.localhost/health");
        let mut answers = url_engine(json!({}));
        answers.push(("healthbar.getServices", Ok(json!([]))));
        assert_eq!(go(&["wait", "nope"], answers).0.unwrap_err().code(), 3);
    }

    #[test]
    fn a_port_is_waited_for_without_an_engine() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let (out, calls) = go(&["wait", "--port", &port], vec![]);
        assert!(out.unwrap().starts_with(&format!("ready · port {port} · ")));
        assert!(calls.is_empty());
        drop(listener);
        let err = go(&["wait", "--port", &port, "--timeout", "0"], vec![]).0.unwrap_err();
        assert_eq!(err.code(), 4);
        assert!(err.line().starts_with("bifurc: timed out after 0s — last: "), "{}", err.line());
    }

    #[test]
    fn a_capture_is_waited_for_from_now_on_with_the_filters() {
        let entry = json!({"id": "mgv2k3f9", "timestamp": 1, "method": "POST", "url": "http://api.localhost/orders?x=1", "status": 201});
        let answers = vec![("config.get", Ok(config())), ("capture.list", Ok(json!({"entries": [entry]})))];
        let (out, calls) = go(&["wait", "--capture", "--method", "POST", "--path", "/orders*"], answers);
        assert!(out.unwrap().starts_with("ready · #k3f9 POST api.localhost/orders?… 201 · "));
        let sent = &payloads(&calls, "capture.list")[0];
        assert_eq!(
            (&sent["method"], &sent["path"], &sent["last"], &sent["summary"]),
            (&json!("POST"), &json!("/orders*"), &json!(1), &json!(true))
        );
        assert!(sent["since"].as_i64().unwrap() > 1_700_000_000_000);
        let answers = vec![("config.get", Ok(config())), ("capture.list", Ok(json!({"entries": []})))];
        let err = go(&["wait", "--capture", "--timeout", "0"], answers).0.unwrap_err();
        assert_eq!(err.line(), "bifurc: timed out after 0s — last: no matching request yet");
    }

    #[test]
    fn it_needs_something_to_wait_for() {
        assert_eq!(go(&["wait"], vec![]).0.unwrap_err().code(), 2);
        assert_eq!(go(&["wait", "--port", "abc"], vec![]).0.unwrap_err().code(), 2);
    }
}
