//! `replay <cassette>|stop|status`: serve a recorded cassette (see `record`) instead of the real hosts, through the
//! proxy and the local mock server. A request the cassette does not hold is a 404 naming the nearest recorded
//! ones, or goes to the real host with `--passthrough`; `replay stop` lists what was not found and exits 1.

use serde_json::{Value, json};

use crate::{
    commands::{Context, workspace_id},
    error::{CliError, Result},
    format::js_template,
    record::summary,
};

pub fn replay(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    match sub {
        Some("stop") => stop(ctx),
        Some("status") => status(ctx),
        // shortcut: a cassette named "stop" or "status" cannot be replayed by name; upgrade with a `start` verb if it matters.
        Some(name) => start(ctx, name),
        None => {
            Err(CliError::usage("replay needs a cassette name: bifurc replay <cassette> (see: bifurc replay --help)"))
        }
    }
}

fn start(ctx: &mut Context, name: &str) -> Result<String> {
    ctx.accept(&["passthrough"], 0)?;
    let passthrough = ctx.args.has("passthrough");
    let payload = json!({ "workspaceId": workspace_id(ctx)?, "name": name, "passthrough": passthrough });
    let started = ctx.request("cassette.replayStart", payload).map_err(|err| match err {
        CliError::Engine { code, message } if code == "CONFLICT" || code == "BAD_REQUEST" => CliError::usage(message),
        other => other,
    })?;
    Ok(ctx.print(&started, || {
        let mut lines =
            vec![format!("replaying {}{}", summary(&started), if passthrough { " · passthrough" } else { "" })];
        lines.extend(
            started["warnings"].as_array().into_iter().flatten().map(|w| format!("! {}", js_template(Some(w)))),
        );
        lines.join("\n")
    }))
}

fn stop(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let done = ctx.request("cassette.replayStop", json!({}))?;
    let (served, passed, missed) = (
        done["served"].as_u64().unwrap_or(0),
        done["passedThrough"].as_u64().unwrap_or(0),
        done["unmatchedCount"].as_u64().unwrap_or(0),
    );
    let text = ctx.print(&done, || {
        let through = if passed > 0 { format!(" · {passed} passed through") } else { String::new() };
        let mut lines =
            vec![format!("stopped {} · {served} served · {missed} unmatched{through}", js_template(done.get("name")))];
        let list = done["unmatched"].as_array().into_iter().flatten();
        for miss in list.clone() {
            lines.push(format!("✗ {} {}", js_template(miss.get("method")), js_template(miss.get("url"))));
            if let Some(closest) = miss["closest"].as_str() {
                lines.push(format!("    closest: {closest}"));
            }
        }
        let shown = list.count() as u64;
        if missed > shown {
            lines.push(format!("… {} more", missed - shown));
        }
        lines.join("\n")
    });
    if missed > 0 { Err(CliError::Failed(text)) } else { Ok(text) }
}

fn status(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 0)?;
    let answer = ctx.request("cassette.replayStatus", json!({}))?;
    Ok(ctx.print(&answer, || match answer.get("replaying").filter(|r| r.is_object()) {
        Some(r) => format!("replaying {}", progress(r)),
        None => "not replaying".to_string(),
    }))
}

/// `checkout · 3 served · 1 unmatched` (also the part `bifurc status` shows).
pub fn progress(replaying: &Value) -> String {
    format!(
        "{} · {} served · {} unmatched",
        js_template(replaying.get("name")),
        replaying["served"].as_u64().unwrap_or(0),
        replaying["unmatchedCount"].as_u64().unwrap_or(0)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        let sub = argv.get(1).copied();
        run(argv, answers, |ctx| replay(ctx, sub))
    }

    fn started(passthrough: bool, warnings: Value) -> Vec<Answer> {
        let answer = json!({"name": "checkout", "hosts": ["api.stripe.com"], "interactions": 12,
                            "passthrough": passthrough, "warnings": warnings});
        vec![("config.get", Ok(config())), ("cassette.replayStart", Ok(answer))]
    }

    #[test]
    fn start_sends_the_cassette_and_prints_the_lines() {
        let (out, calls) = go(&["replay", "checkout"], started(false, json!([])));
        assert_eq!(out.unwrap(), "replaying checkout · 12 interactions · api.stripe.com");
        assert_eq!(
            payloads(&calls, "cassette.replayStart"),
            [json!({"workspaceId": "w1", "name": "checkout", "passthrough": false})]
        );
        let (out, calls) =
            go(&["replay", "checkout", "--passthrough"], started(true, json!(["HTTPS interception is off"])));
        assert_eq!(
            out.unwrap(),
            "replaying checkout · 12 interactions · api.stripe.com · passthrough\n! HTTPS interception is off"
        );
        assert_eq!(payloads(&calls, "cassette.replayStart")[0]["passthrough"], true);
        let (out, _) = go(&["replay", "checkout", "--json"], started(false, json!([])));
        assert!(out.unwrap().contains("\"interactions\": 12"));
    }

    #[test]
    fn usage_errors_and_a_missing_cassette() {
        assert_eq!(go(&["replay"], vec![]).0.unwrap_err().code(), 2);
        assert_eq!(go(&["replay", "x", "--nope=1"], vec![("config.get", Ok(config()))]).0.unwrap_err().code(), 2);
        assert_eq!(go(&["replay", "stop", "--passthrough"], vec![]).0.unwrap_err().code(), 2);
        let busy = Err(CliError::Engine { code: "CONFLICT".into(), message: "already replaying \"x\"".into() });
        let err =
            go(&["replay", "y"], vec![("config.get", Ok(config())), ("cassette.replayStart", busy)]).0.unwrap_err();
        assert_eq!((err.code(), err.line()), (2, "bifurc: already replaying \"x\"".to_string()));
        let none = Err(CliError::Engine { code: "NOT_FOUND".into(), message: "No cassette".into() });
        let answers = vec![("config.get", Ok(config())), ("cassette.replayStart", none)];
        assert_eq!(go(&["replay", "nope"], answers).0.unwrap_err().code(), 3);
    }

    #[test]
    fn stop_exits_1_and_lists_what_was_not_found() {
        let clean = json!({"name": "checkout", "served": 10, "passedThrough": 0, "unmatched": [], "unmatchedCount": 0});
        let (out, _) = go(&["replay", "stop"], vec![("cassette.replayStop", Ok(clean))]);
        assert_eq!(out.unwrap(), "stopped checkout · 10 served · 0 unmatched");
        let dirty = json!({"name": "checkout", "served": 10, "passedThrough": 2, "unmatchedCount": 3, "unmatched": [
            {"method": "GET", "url": "https://api.stripe.com/v1/chargez", "closest": "GET /v1/charges"},
            {"method": "POST", "url": "https://api.stripe.com/v1/x", "closest": null}]});
        let (out, _) = go(&["replay", "stop"], vec![("cassette.replayStop", Ok(dirty.clone()))]);
        let err = out.unwrap_err();
        assert_eq!(err.code(), 1);
        assert_eq!(
            err,
            CliError::Failed(
                "stopped checkout · 10 served · 3 unmatched · 2 passed through\n✗ GET https://api.stripe.com/v1/chargez\n    closest: GET /v1/charges\n✗ POST https://api.stripe.com/v1/x\n… 1 more"
                    .to_string()
            )
        );
        let (out, _) = go(&["replay", "stop", "--json"], vec![("cassette.replayStop", Ok(dirty))]);
        let Err(CliError::Failed(text)) = out else { panic!("{out:?}") };
        assert!(text.contains("\"unmatchedCount\": 3"));
        let none = Err(CliError::Engine { code: "NOT_FOUND".into(), message: "Nothing is replaying.".into() });
        assert_eq!(go(&["replay", "stop"], vec![("cassette.replayStop", none)]).0.unwrap_err().code(), 3);
    }

    #[test]
    fn status_lines() {
        let on = json!({"replaying": {"name": "checkout", "served": 3, "unmatchedCount": 1}});
        let (out, _) = go(&["replay", "status"], vec![("cassette.replayStatus", Ok(on))]);
        assert_eq!(out.unwrap(), "replaying checkout · 3 served · 1 unmatched");
        let off = json!({"replaying": null});
        let (out, _) = go(&["replay", "status"], vec![("cassette.replayStatus", Ok(off.clone()))]);
        assert_eq!(out.unwrap(), "not replaying");
        let (out, _) = go(&["replay", "status", "--json"], vec![("cassette.replayStatus", Ok(off))]);
        assert!(out.unwrap().contains("\"replaying\": null"));
    }
}
