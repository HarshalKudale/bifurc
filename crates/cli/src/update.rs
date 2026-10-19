//! `update`: whether a newer Bifurc exists, and how this install gets it. The engine knows both (`update.status`);
//! nothing is installed from here.

use serde_json::{Value, json};

use crate::{
    commands::{Context, unknown_verb},
    error::Result,
    format::truthy,
};

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The engine's answer as a person reads it.
fn describe(status: &Value) -> String {
    let installed = format!("Bifurc {} ({})", text(status, "currentVersion"), text(status, "channel"));
    let mut lines = Vec::new();
    let available = status.get("available").filter(|available| !available.is_null());
    match available {
        Some(available) => {
            lines.push(format!("{installed}: version {} is available.", text(available, "version")));
            if !text(available, "summary").is_empty() {
                lines.push(format!("  {}", text(available, "summary")));
            }
            let path: Vec<&str> = available
                .get("path")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if path.len() > 1 {
                lines.push(format!(
                    "  This version cannot go there in one step. Update to {} first, then {}.",
                    path[0],
                    path[1..].join(", then ")
                ));
            }
            if available.get("critical").is_some_and(truthy) {
                lines.push("  It fixes a security problem.".to_string());
            }
            let action = status.get("action").unwrap_or(&Value::Null);
            match text(action, "kind") {
                "store" => lines.push(
                    "  The Microsoft Store installs it: open the Store's Downloads page to get it now.".to_string(),
                ),
                "command" => lines.push(format!("  Run: {}", text(action, "command"))),
                "updater" => lines
                    .push("  Run: bifurc update apply (closes Bifurc, installs it and opens it again).".to_string()),
                _ => lines.push(format!("  Download: {}", text(action, "url"))),
            }
            if !text(available, "notesUrl").is_empty() {
                lines.push(format!("  What changed: {}", text(available, "notesUrl")));
            }
        }
        None if status.get("checkedAt").is_none_or(Value::is_null) => {
            lines.push(format!("{installed}: not checked yet (bifurc update check)."));
        }
        None => lines.push(format!("{installed} is up to date.")),
    }
    if status.get("unsupported").is_some_and(truthy) {
        lines.push("  This version is no longer supported.".to_string());
    }
    if !text(status, "error").is_empty() {
        lines.push(format!("  The last check failed: {}", text(status, "error")));
    }
    lines.join("\n")
}

/// `update [status|check|apply]`. `apply` hands the update to the updater, which closes Bifurc, installs it and
/// opens it again.
pub fn update(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let command = match sub.unwrap_or("status") {
        "status" => "update.status",
        "check" => "update.check",
        "apply" => {
            ctx.accept(&[], 0)?;
            let started = ctx.request("update.apply", json!({}))?;
            return Ok(
                ctx.print(&started, || "The updater is running. Bifurc closes, updates and opens again.".to_string())
            );
        }
        other => return Err(unknown_verb("update", other)),
    };
    ctx.accept(&[], 0)?;
    let status = ctx.request(command, json!({}))?;
    Ok(ctx.print(&status, || describe(&status)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{run, sub};

    fn available() -> Value {
        json!({
            "currentVersion": "0.4.0", "channel": "installer", "checkedAt": 5, "error": null, "unsupported": false,
            "action": { "kind": "download", "url": "https://bifurc.app/download" },
            "available": { "version": "0.6.0", "summary": "Faster.", "path": ["0.5.0", "0.6.0"], "critical": true,
                           "notesUrl": "https://bifurc.app/changelog" },
        })
    }

    #[test]
    fn an_update_is_described_with_its_steps_and_how_to_get_it() {
        let argv = ["update"];
        let (out, calls) = run(&argv, vec![("update.status", Ok(available()))], |c| update(c, sub(&argv)));
        assert_eq!(
            out.unwrap(),
            "Bifurc 0.4.0 (installer): version 0.6.0 is available.\n  Faster.\n  This version cannot go there in one \
             step. Update to 0.5.0 first, then 0.6.0.\n  It fixes a security problem.\n  Download: \
             https://bifurc.app/download\n  What changed: https://bifurc.app/changelog"
        );
        assert_eq!(calls[0].0, "update.status");
    }

    #[test]
    fn check_asks_the_engine_to_look_now() {
        let argv = ["update", "check"];
        let current = json!({ "currentVersion": "0.6.0", "channel": "scoop", "checkedAt": 5, "available": null });
        let (out, calls) = run(&argv, vec![("update.check", Ok(current))], |c| update(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Bifurc 0.6.0 (scoop) is up to date.");
        assert_eq!(calls[0].0, "update.check");
    }

    #[test]
    fn apply_hands_the_update_to_the_updater() {
        let argv = ["update", "apply"];
        let (out, calls) =
            run(&argv, vec![("update.apply", Ok(json!({ "started": true })))], |c| update(c, sub(&argv)));
        assert!(out.unwrap().starts_with("The updater is running."));
        assert_eq!(calls[0].0, "update.apply");
    }

    #[test]
    fn a_package_managers_install_is_given_its_command() {
        let mut status = available();
        status["action"] = json!({ "kind": "command", "command": "scoop update bifurc" });
        assert!(describe(&status).contains("\n  Run: scoop update bifurc\n"));
    }

    #[test]
    fn an_engine_that_never_looked_says_so() {
        let status = json!({ "currentVersion": "0.5.0", "channel": "unknown", "checkedAt": null, "available": null });
        assert_eq!(describe(&status), "Bifurc 0.5.0 (unknown): not checked yet (bifurc update check).");
    }

    #[test]
    fn other_verbs_are_refused() {
        let argv = ["update", "install"];
        let (out, calls) = run(&argv, vec![], |c| update(c, sub(&argv)));
        assert!(out.unwrap_err().to_string().starts_with("unknown: update install"));
        assert!(calls.is_empty());
    }
}
