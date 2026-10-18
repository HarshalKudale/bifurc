//! `record start|stop|status`: record the real answers of chosen hosts, as they pass through the proxy, into a
//! cassette of the workspace (`cassettes/<name>.json`, see `engine/src/cassettes.rs`). Secrets are masked by the
//! engine as it records. Replaying a cassette is a later verb.

use serde_json::{Value, json};

use crate::{
    commands::{Context, need, unknown_verb, workspace_id},
    error::{CliError, Result},
    format::js_template,
};

pub fn record(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub {
        Some("start") => start(ctx),
        Some("stop") => stop(ctx),
        Some("status") => status(ctx),
        other => Err(unknown_verb("record", other.unwrap_or(""))),
    }
}

fn start(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["hosts", "cassette"], 0)?;
    let raw = need(ctx.args.flag("hosts"), "--hosts a.com,b.com")?;
    let hosts: Vec<String> = raw.split(',').map(str::trim).filter(|h| !h.is_empty()).map(String::from).collect();
    let name = need(ctx.args.flag("cassette"), "--cassette <name>")?;
    let payload = json!({ "workspaceId": workspace_id(ctx)?, "name": name, "hosts": hosts });
    let started = ctx.request("cassette.recordStart", payload).map_err(|err| match err {
        CliError::Engine { code, message } if code == "CONFLICT" || code == "BAD_REQUEST" => CliError::usage(message),
        other => other,
    })?;
    Ok(ctx.print(&started, || {
        let mut lines = vec![format!("recording {name}: {} (proxy traffic only)", hosts.join(", "))];
        if started["replaced"].as_bool() == Some(true) {
            lines.push(format!("replaced {}", js_template(started.get("path"))));
        }
        lines.extend(
            started["warnings"].as_array().into_iter().flatten().map(|w| format!("! {}", js_template(Some(w)))),
        );
        lines.join("\n")
    }))
}

fn stop(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let done = ctx.request("cassette.recordStop", json!({}))?;
    Ok(ctx.print(&done, || {
        let (n, m) = (done["interactions"].as_u64().unwrap_or(0), done["masked"].as_u64().unwrap_or(0));
        format!(
            "saved {} · {n} interaction{} · {m} value{} masked",
            js_template(done.get("path")),
            if n == 1 { "" } else { "s" },
            if m == 1 { "" } else { "s" }
        )
    }))
}

fn status(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let answer = ctx.request("cassette.recordStatus", json!({}))?;
    Ok(ctx.print(&answer, || match answer.get("recording").filter(|r| r.is_object()) {
        Some(r) => format!("recording {}", summary(r)),
        None => "not recording".to_string(),
    }))
}

/// `checkout · 12 interactions · api.stripe.com` (also the part `bifurc status` shows).
pub fn summary(recording: &Value) -> String {
    let n = recording["interactions"].as_u64().unwrap_or(0);
    let hosts: Vec<String> =
        recording["hosts"].as_array().into_iter().flatten().map(|h| js_template(Some(h))).collect();
    format!(
        "{} · {n} interaction{} · {}",
        js_template(recording.get("name")),
        if n == 1 { "" } else { "s" },
        hosts.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        let sub = argv.get(1).copied();
        run(argv, answers, |ctx| record(ctx, sub))
    }

    fn started(replaced: bool, warnings: Value) -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            (
                "cassette.recordStart",
                Ok(
                    json!({"name": "checkout", "path": "cassettes/checkout.json", "replaced": replaced, "warnings": warnings}),
                ),
            ),
        ]
    }

    #[test]
    fn start_sends_the_hosts_and_prints_the_lines() {
        let argv = ["record", "start", "--hosts", "api.stripe.com, api.openai.com", "--cassette", "checkout"];
        let (out, calls) = go(&argv, started(false, json!([])));
        assert_eq!(out.unwrap(), "recording checkout: api.stripe.com, api.openai.com (proxy traffic only)");
        assert_eq!(
            payloads(&calls, "cassette.recordStart"),
            [json!({"workspaceId": "w1", "name": "checkout", "hosts": ["api.stripe.com", "api.openai.com"]})]
        );
        let (out, _) = go(&argv, started(true, json!(["HTTPS interception is off"])));
        assert_eq!(
            out.unwrap(),
            "recording checkout: api.stripe.com, api.openai.com (proxy traffic only)\nreplaced cassettes/checkout.json\n! HTTPS interception is off"
        );
    }

    #[test]
    fn start_needs_hosts_and_a_name_and_a_second_recording_is_a_usage_error() {
        for argv in [vec!["record", "start", "--cassette", "x"], vec!["record", "start", "--hosts", "a.com"]] {
            assert_eq!(go(&argv, vec![("config.get", Ok(config()))]).0.unwrap_err().code(), 2);
        }
        let busy = Err(CliError::Engine {
            code: "CONFLICT".into(),
            message: "already recording \"x\"; bifurc record stop".into(),
        });
        let answers = vec![("config.get", Ok(config())), ("cassette.recordStart", busy)];
        let err = go(&["record", "start", "--hosts", "a.com", "--cassette", "y"], answers).0.unwrap_err();
        assert_eq!((err.code(), err.line()), (2, "bifurc: already recording \"x\"; bifurc record stop".to_string()));
        assert_eq!(go(&["record", "nope"], vec![]).0.unwrap_err().code(), 2);
    }

    #[test]
    fn stop_and_status_lines_and_exit_codes() {
        let done = json!({"name": "checkout", "path": "cassettes/checkout.json", "interactions": 12, "masked": 3});
        let (out, _) = go(&["record", "stop"], vec![("cassette.recordStop", Ok(done))]);
        assert_eq!(out.unwrap(), "saved cassettes/checkout.json · 12 interactions · 3 values masked");
        let none = Err(CliError::Engine { code: "NOT_FOUND".into(), message: "Nothing is recording.".into() });
        assert_eq!(go(&["record", "stop"], vec![("cassette.recordStop", none)]).0.unwrap_err().code(), 3);
        let on = json!({"recording": {"name": "checkout", "hosts": ["api.stripe.com"], "interactions": 12}});
        let (out, _) = go(&["record", "status"], vec![("cassette.recordStatus", Ok(on))]);
        assert_eq!(out.unwrap(), "recording checkout · 12 interactions · api.stripe.com");
        let (out, _) = go(&["record", "status"], vec![("cassette.recordStatus", Ok(json!({"recording": null})))]);
        assert_eq!(out.unwrap(), "not recording");
        let (out, _) =
            go(&["record", "status", "--json"], vec![("cassette.recordStatus", Ok(json!({"recording": null})))]);
        assert!(out.unwrap().contains("\"recording\": null"));
    }
}
