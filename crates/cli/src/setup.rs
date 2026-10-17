//! `bifurc setup`: the app's first-run setup without the app, for a machine that has none (a container, CI).
//! It is never run implicitly, and the two `--accept-*` flags are the person's acceptance of the terms and
//! the privacy policy at the URLs it prints.
//!
//! The steps and their order are the app's own (`screens/onboarding.rs`): accept the terms, accept the
//! policy, the servers step (`setup.startServers`, which also records it), the first workspace if there is
//! none, and `setup.complete`.

use serde_json::{Value, json};

use crate::{
    commands::Context,
    error::{CliError, Result},
};

const FINISH: &str = "bifurc setup --accept-terms --accept-privacy [--workspace <name>]";

fn flag_of(state: &Value, key: &str) -> bool {
    state.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The state in a few lines; what is not accepted yet carries the URL to read. `given` are the acceptances
/// this run has just recorded.
fn describe(state: &Value, given: &[(&str, bool)]) -> String {
    let text = |key: &str| state.get(key).and_then(Value::as_str).unwrap_or_default();
    let line = |label: &str, key: &str, url: &str, given_now: bool| {
        if flag_of(state, key) || given_now {
            format!("{label:<10} accepted")
        } else {
            format!("{label:<10} not accepted  {}", text(url))
        }
    };
    let now = |flag: &str| given.iter().any(|(name, on)| *on && *name == flag);
    let workspace = if flag_of(state, "needsWorkspace") { "none" } else { "yes" };
    [
        line("terms", "termsAccepted", "termsOfServiceUrl", now("accept-terms")),
        line("privacy", "privacyAccepted", "privacyPolicyUrl", now("accept-privacy")),
        format!("{:<10} {workspace}", "workspace"),
    ]
    .join("\n")
}

fn port_of(ctx: &Context, flag: &str) -> Result<Option<u16>> {
    ctx.args
        .flag(flag)
        .map(|raw| {
            raw.parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| CliError::usage(format!("--{flag} is a port number, 1 to 65535, not \"{raw}\"")))
        })
        .transpose()
}

/// A step the engine reported as `{ok: false, error}`.
fn checked(answer: Value) -> Result<Value> {
    match answer.get("ok").and_then(Value::as_bool) {
        Some(false) => Err(CliError::msg(answer.get("error").and_then(Value::as_str).unwrap_or("setup failed"))),
        _ => Ok(answer),
    }
}

pub fn setup(ctx: &mut Context) -> Result<String> {
    ctx.accept_top(&["accept-terms", "accept-privacy", "port", "proxy-port"], 0)?;
    let args = ctx.args;
    let (terms, privacy) = (args.has("accept-terms"), args.has("accept-privacy"));
    let ports = (port_of(ctx, "port")?, port_of(ctx, "proxy-port")?);
    let state = ctx.request("setup.getState", json!({}))?;
    if flag_of(&state, "onboarded") {
        let text = format!("{}\nsetup done", describe(&state, &[]));
        return Ok(ctx.print(&state, || text));
    }
    if !terms && !privacy {
        return Err(CliError::needs_human(format!(
            "setup is not finished\n{}\nfinish it: {FINISH}\n(the flags mean you accept the terms and the privacy policy at the URLs above)",
            describe(&state, &[])
        )));
    }
    // Each acceptance is recorded as given, so a second run only has the other left to do.
    if terms && !flag_of(&state, "termsAccepted") {
        checked(ctx.request("setup.acceptTerms", json!({}))?)?;
    }
    if privacy && !flag_of(&state, "privacyAccepted") {
        checked(ctx.request("setup.acceptPrivacy", json!({}))?)?;
    }
    let missing: Vec<String> = [("terms", "termsAccepted", terms), ("privacy", "privacyAccepted", privacy)]
        .into_iter()
        .filter(|(_, key, given)| !given && !flag_of(&state, key))
        .map(|(name, ..)| format!("--accept-{name}"))
        .collect();
    if !missing.is_empty() {
        let given = [("accept-terms", terms), ("accept-privacy", privacy)];
        return Err(CliError::needs_human(format!(
            "setup is not finished\n{}\nstill needed from you: {}",
            describe(&state, &given),
            missing.join(" ")
        )));
    }

    let mut payload = json!({});
    if let Some(port) = ports.0 {
        payload["port"] = json!(port);
    }
    if let Some(port) = ports.1 {
        payload["forwardProxyPort"] = json!(port);
    }
    let servers = checked(ctx.request("setup.startServers", payload)?)?;
    let problems: Vec<String> = ["reverse", "forward"]
        .into_iter()
        .filter_map(|key| {
            let listener = servers.get(key)?;
            let error = listener.get("error").and_then(Value::as_str).filter(|error| !error.is_empty())?;
            Some(format!("the {key} proxy could not start on :{}: {error}", listener["port"]))
        })
        .collect();
    if !problems.is_empty() {
        return Err(CliError::msg(format!(
            "{}\nchoose other ports: bifurc setup --accept-terms --accept-privacy --port <n> --proxy-port <n>",
            problems.join("\n")
        )));
    }

    let mut parts = vec!["setup done".to_string()];
    if flag_of(&state, "needsWorkspace") {
        let name = args.flag("workspace").map(str::trim).filter(|name| !name.is_empty()).unwrap_or("Default");
        let added = ctx.request("workspace.add", json!({ "name": name }))?;
        parts.push(format!("workspace {}", added.get("name").and_then(Value::as_str).unwrap_or(name)));
    }
    checked(ctx.request("setup.complete", json!({}))?)?;
    if let Some(port) = servers["forward"]["port"].as_u64() {
        parts.push(format!("proxy :{port}"));
    }
    let done = json!({ "ok": true, "servers": servers });
    Ok(ctx.print(&done, || parts.join(" · ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{payloads, run};

    fn fresh() -> Value {
        json!({"termsAccepted": false, "privacyAccepted": false, "onboarded": false, "needsWorkspace": true,
               "termsOfServiceUrl": "https://bifurc.app/tos", "privacyPolicyUrl": "https://bifurc.app/privacy"})
    }

    fn walk(argv: &[&str], state: Value) -> (Result<String>, Vec<(String, Value)>) {
        let servers = json!({"ok": true, "reverse": {"port": 80}, "forward": {"port": 8080}});
        let answers = vec![
            ("setup.getState", Ok(state)),
            ("setup.acceptTerms", Ok(json!({"ok": true}))),
            ("setup.acceptPrivacy", Ok(json!({"ok": true}))),
            ("setup.startServers", Ok(servers)),
            ("workspace.add", Ok(json!({"id": "w", "name": "Default"}))),
            ("setup.complete", Ok(json!({"ok": true}))),
        ];
        run(argv, answers, setup)
    }

    fn names(calls: &[(String, Value)]) -> Vec<&str> {
        calls.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn without_flags_it_prints_the_state_and_needs_a_person() {
        let (result, calls) = walk(&["setup"], fresh());
        let err = result.unwrap_err();
        assert_eq!(err.code(), 6);
        assert_eq!(
            err.to_string().lines().take(5).collect::<Vec<_>>(),
            [
                "setup is not finished",
                "terms      not accepted  https://bifurc.app/tos",
                "privacy    not accepted  https://bifurc.app/privacy",
                "workspace  none",
                "finish it: bifurc setup --accept-terms --accept-privacy [--workspace <name>]",
            ]
        );
        assert_eq!(names(&calls), ["setup.getState"]);
    }

    #[test]
    fn both_flags_walk_the_apps_steps_in_the_apps_order() {
        let argv = [
            "setup",
            "--accept-terms",
            "--accept-privacy",
            "--workspace",
            "e2e",
            "--port",
            "18190",
            "--proxy-port",
            "18191",
        ];
        let servers = json!({"ok": true, "forward": {"port": 18191}, "reverse": {"port": 18190}});
        let answers = vec![
            ("setup.getState", Ok(fresh())),
            ("setup.acceptTerms", Ok(json!({"ok": true}))),
            ("setup.acceptPrivacy", Ok(json!({"ok": true}))),
            ("setup.startServers", Ok(servers)),
            ("workspace.add", Ok(json!({"id": "w", "name": "e2e"}))),
            ("setup.complete", Ok(json!({"ok": true}))),
        ];
        let (result, calls) = run(&argv, answers, setup);
        assert_eq!(result.unwrap(), "setup done · workspace e2e · proxy :18191");
        assert_eq!(
            names(&calls),
            [
                "setup.getState",
                "setup.acceptTerms",
                "setup.acceptPrivacy",
                "setup.startServers",
                "workspace.add",
                "setup.complete"
            ]
        );
        assert_eq!(payloads(&calls, "setup.startServers"), [json!({"port": 18190, "forwardProxyPort": 18191})]);
        assert_eq!(payloads(&calls, "workspace.add"), [json!({"name": "e2e"})]);
    }

    #[test]
    fn the_workspace_is_called_default_and_only_made_when_there_is_none() {
        let (result, calls) = walk(&["setup", "--accept-terms", "--accept-privacy"], fresh());
        assert_eq!(result.unwrap(), "setup done · workspace Default · proxy :8080");
        assert_eq!(payloads(&calls, "workspace.add"), [json!({"name": "Default"})]);
        assert_eq!(payloads(&calls, "setup.startServers"), [json!({})]);
        let mut state = fresh();
        state["needsWorkspace"] = json!(false);
        let (result, calls) = walk(&["setup", "--accept-terms", "--accept-privacy"], state);
        assert_eq!(result.unwrap(), "setup done · proxy :8080");
        assert!(payloads(&calls, "workspace.add").is_empty());
    }

    #[test]
    fn one_flag_records_that_one_and_names_the_other() {
        let (result, calls) = walk(&["setup", "--accept-terms"], fresh());
        let err = result.unwrap_err();
        assert_eq!(err.code(), 6);
        assert!(err.to_string().ends_with("still needed from you: --accept-privacy"), "{err}");
        assert!(err.to_string().starts_with("setup is not finished\nterms      accepted\nprivacy    not accepted"), "{err}");
        assert_eq!(names(&calls), ["setup.getState", "setup.acceptTerms"]);
    }

    #[test]
    fn a_finished_setup_only_prints_the_state() {
        let mut state = fresh();
        for key in ["onboarded", "termsAccepted", "privacyAccepted"] {
            state[key] = json!(true);
        }
        state["needsWorkspace"] = json!(false);
        let (result, calls) = walk(&["setup", "--accept-terms", "--accept-privacy"], state);
        assert_eq!(result.unwrap(), "terms      accepted\nprivacy    accepted\nworkspace  yes\nsetup done");
        assert_eq!(names(&calls), ["setup.getState"]);
    }

    #[test]
    fn a_bad_port_is_a_usage_error_before_anything_is_recorded() {
        let (result, calls) = walk(&["setup", "--accept-terms", "--accept-privacy", "--port", "x"], fresh());
        assert_eq!(result.unwrap_err().code(), 2);
        assert!(calls.is_empty());
    }

    #[test]
    fn a_proxy_that_cannot_start_says_which_and_leaves_the_setup_open() {
        let started = json!({"ok": true, "reverse": {"port": 80, "error": "in use"}});
        let answers = vec![
            ("setup.getState", Ok(fresh())),
            ("setup.acceptTerms", Ok(json!({"ok": true}))),
            ("setup.acceptPrivacy", Ok(json!({"ok": true}))),
            ("setup.startServers", Ok(started)),
        ];
        let (result, calls) = run(&["setup", "--accept-terms", "--accept-privacy"], answers, setup);
        let err = result.unwrap_err();
        assert!(err.to_string().starts_with("the reverse proxy could not start on :80: in use"), "{err}");
        assert!(payloads(&calls, "setup.complete").is_empty());
    }
}
