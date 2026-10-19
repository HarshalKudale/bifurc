//! `approvals list|approve|deny`: the calls a `require_human` policy holds for a person. The agent's call waits in
//! the engine; the person decides here, in their own terminal. `approve` and `deny` refuse unless stdin and
//! stdout are both terminals, so an agent's non-interactive shell cannot approve its own call. Not exposed
//! through MCP (which has no terminal).

use std::io::IsTerminal;

use serde_json::{Value, json};

use crate::{
    capture::{now_ms, tails},
    changes::ago,
    commands::{Context, need, unknown_verb},
    error::{CliError, Result},
    format::aligned,
};

fn text<'a>(item: &'a Value, key: &str) -> &'a str {
    item.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `DELETE https://… (policy staging-deletes)`
fn what(item: &Value) -> String {
    format!("{} {} (policy {})", text(item, "method"), text(item, "url"), text(item, "policy"))
}

fn items(got: &Value, key: &str) -> Vec<Value> {
    got.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn list(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["all"], 0)?;
    let mut payload = json!({ "all": ctx.args.has("all") });
    if let Some(workspace) = ctx.args.flag("workspace") {
        payload["workspaceId"] = json!(workspace);
    }
    let got = ctx.request("approvals.list", payload)?;
    Ok(ctx.print(&got, || {
        let (pending, decided) = (items(&got, "pending"), items(&got, "decided"));
        let ids: Vec<&str> = pending.iter().chain(&decided).map(|i| text(i, "id")).collect();
        let tail = tails(&ids);
        let now = now_ms();
        let rows = |items: &[Value], first: usize, decisions: bool| -> Vec<Vec<String>> {
            items
                .iter()
                .enumerate()
                .map(|(n, item)| {
                    let mut row = vec![format!("#{}", tail[first + n])];
                    if decisions {
                        row.push(text(item, "decision").to_string());
                    }
                    let at = item[if decisions { "decidedAt" } else { "createdAt" }].as_i64().unwrap_or(now);
                    row.push(ago(now - at));
                    row.push(format!("{} {}", text(item, "method"), text(item, "url")));
                    row.push(format!("policy {}", text(item, "policy")));
                    row.push(format!("by {}", text(item, "by")));
                    if decisions {
                        let reason = text(item, "reason");
                        let why = if reason.is_empty() { String::new() } else { format!(": {reason}") };
                        row.push(format!("decided by {}{why}", text(item, "decidedBy")));
                    }
                    row
                })
                .collect()
        };
        let mut out =
            if pending.is_empty() { "no approvals pending".to_string() } else { aligned(&rows(&pending, 0, false)) };
        if !decided.is_empty() {
            out = format!("{out}\n\ndecided\n{}", aligned(&rows(&decided, pending.len(), true)));
        }
        out
    }))
}

/// The pending item `wanted` names (a full id, `#tail` or `tail`).
fn resolve(pending: &[Value], wanted: &str) -> Result<Value> {
    let tail = wanted.strip_prefix('#').unwrap_or(wanted);
    if let Some(exact) = pending.iter().find(|i| text(i, "id") == tail) {
        return Ok(exact.clone());
    }
    let hits: Vec<&Value> = pending.iter().filter(|i| !tail.is_empty() && text(i, "id").ends_with(tail)).collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => {
            let ids: Vec<&str> = pending.iter().map(|i| text(i, "id")).collect();
            let hint = if ids.is_empty() {
                "nothing is pending".to_string()
            } else {
                format!("did you mean {}?", tails(&ids).iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(", "))
            };
            Err(CliError::not_found(format!("no pending approval \"{wanted}\" — {hint} (see: bifurc approvals list)")))
        }
        many => {
            let listed: Vec<&str> = many.iter().map(|i| text(i, "id")).collect();
            Err(CliError::usage(format!(
                "{} approvals end with \"{tail}\": {} — use more of the id",
                many.len(),
                listed.join(", ")
            )))
        }
    }
}

/// Decide the pending item named by the operand. `interactive` is whether a person is at a terminal; it is what
/// lets the engine accept the decision, so the caller sets it only after checking.
fn decide(ctx: &mut Context, decision: &str, interactive: bool) -> Result<String> {
    ctx.accept(&["reason"], 1)?;
    if !interactive {
        return Err(CliError::usage("approvals are decided by a person in a terminal"));
    }
    let missing = format!("the approval: bifurc approvals {decision} <id> (see: bifurc approvals list)");
    let wanted = need(ctx.args.positional(2), &missing)?;
    let pending = items(&ctx.request("approvals.list", json!({}))?, "pending");
    let item = resolve(&pending, &wanted)?;
    let mut payload = json!({ "id": text(&item, "id"), "decision": decision, "interactive": true });
    if let Some(reason) = ctx.args.flag("reason") {
        payload["reason"] = json!(reason);
    }
    let done = ctx.request("approvals.decide", payload)?;
    Ok(ctx.print(&done, || {
        let ids: Vec<&str> = pending.iter().map(|i| text(i, "id")).collect();
        let at = ids.iter().position(|id| *id == text(&done, "id")).unwrap_or(0);
        let tail = tails(&ids).get(at).cloned().unwrap_or_default();
        let verb = if decision == "approve" { "approved" } else { "denied" };
        let late = if done.get("passUntil").is_some() {
            "\nthe call had stopped waiting: the same call, run again within 10 minutes, goes through once"
        } else {
            ""
        };
        format!("{verb} #{tail}: {}{late}", what(&done))
    }))
}

pub fn approvals(ctx: &mut Context, sub: Option<&str>) -> Result<String> {
    let person = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    match sub {
        Some("list") => list(ctx),
        Some("approve") => decide(ctx, "approve", person),
        Some("deny") => decide(ctx, "deny", person),
        Some(other) => Err(unknown_verb("approvals", other)),
        None => Err(CliError::usage("missing the verb: list | approve | deny (see: bifurc approvals --help)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, payloads, run as with_engine};

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        with_engine(argv, answers, |ctx| approvals(ctx, argv.get(1).copied()))
    }

    /// `approve`/`deny` as a person at a terminal would run them.
    fn go_at_terminal(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        let decision = if argv[1] == "approve" { "approve" } else { "deny" };
        with_engine(argv, answers, |ctx| decide(ctx, decision, true))
    }

    fn item(id: &str, method: &str, by: &str) -> Value {
        json!({"id": id, "policy": "staging-deletes", "method": method, "url": "https://staging.x/orders/1",
            "workspaceId": "w1", "by": by, "createdAt": now_ms() - 125_000, "expiresAt": now_ms() + 1_000_000})
    }

    fn pending() -> Value {
        json!({"pending": [item("m1aaaa1k3f", "DELETE", "session:agent-1"), item("m1aaaa2x1b", "DELETE", "cli")]})
    }

    #[test]
    fn list_is_a_row_per_pending_call_with_the_shortest_tail_and_says_when_none() {
        let (out, calls) = go(&["approvals", "list"], vec![("approvals.list", Ok(pending()))]);
        assert_eq!(
            out.unwrap(),
            "#1k3f  2m ago  DELETE https://staging.x/orders/1  policy staging-deletes  by session:agent-1\n\
             #2x1b  2m ago  DELETE https://staging.x/orders/1  policy staging-deletes  by cli"
        );
        assert_eq!(payloads(&calls, "approvals.list"), [json!({"all": false})]);
        let (out, _) = go(&["approvals", "list"], vec![("approvals.list", Ok(json!({"pending": []})))]);
        assert_eq!(out.unwrap(), "no approvals pending");
    }

    #[test]
    fn list_all_adds_the_decided_ones_with_who_and_why() {
        let mut done = item("m1aaaa3zzz", "POST", "cli");
        done["decision"] = json!("denied");
        done["decidedAt"] = json!(now_ms() - 61_000);
        done["decidedBy"] = json!("cli");
        done["reason"] = json!("not now");
        let got = json!({"pending": [], "decided": [done]});
        let (out, calls) = go(&["approvals", "list", "--all", "-w", "w2"], vec![("approvals.list", Ok(got))]);
        assert_eq!(
            out.unwrap(),
            "no approvals pending\n\ndecided\n\
             #3zzz  denied  1m ago  POST https://staging.x/orders/1  policy staging-deletes  by cli  decided by cli: not now"
        );
        assert_eq!(payloads(&calls, "approvals.list"), [json!({"all": true, "workspaceId": "w2"})]);
    }

    #[test]
    fn approve_and_deny_refuse_without_a_terminal_before_asking_the_engine_anything() {
        // The test harness has no terminal on stdout.
        for verb in ["approve", "deny"] {
            let (out, calls) = go(&["approvals", verb, "1k3f"], vec![("approvals.list", Ok(pending()))]);
            let error = out.unwrap_err();
            assert_eq!(
                (error.code(), error.to_string()),
                (2, "approvals are decided by a person in a terminal".into())
            );
            assert!(calls.is_empty());
        }
    }

    #[test]
    fn a_decision_names_the_item_by_its_tail_and_sends_the_interactive_flag() {
        let decided = json!({"id": "m1aaaa1k3f", "policy": "staging-deletes", "method": "DELETE",
            "url": "https://staging.x/orders/1", "decision": "approved"});
        let answers = vec![("approvals.list", Ok(pending())), ("approvals.decide", Ok(decided))];
        let (out, calls) = go_at_terminal(&["approvals", "approve", "#1k3f", "--reason", "checked"], answers.clone());
        assert_eq!(out.unwrap(), "approved #1k3f: DELETE https://staging.x/orders/1 (policy staging-deletes)");
        assert_eq!(
            payloads(&calls, "approvals.decide"),
            [json!({"id": "m1aaaa1k3f", "decision": "approve", "interactive": true, "reason": "checked"})]
        );
        let mut late = answers.clone();
        late[1].1 = Ok(json!({"id": "m1aaaa1k3f", "method": "DELETE", "url": "u", "policy": "p", "passUntil": 5}));
        let (out, _) = go_at_terminal(&["approvals", "deny", "1k3f"], late);
        assert!(out.unwrap().ends_with("goes through once"));
        let (_, calls) = go_at_terminal(&["approvals", "deny", "1k3f"], answers);
        assert_eq!(payloads(&calls, "approvals.decide")[0]["decision"], "deny");
    }

    #[test]
    fn an_unknown_or_ambiguous_id_is_exit_3_with_a_hint_or_exit_2() {
        let answers = vec![("approvals.list", Ok(pending()))];
        let (out, _) = go_at_terminal(&["approvals", "approve", "1k3x"], answers);
        let error = out.unwrap_err();
        assert_eq!(error.code(), 3);
        assert!(error.to_string().contains("did you mean #1k3f, #2x1b?"), "{error}");
        let both = json!({"pending": [item("m1aaaa1k3f", "GET", "cli"), item("m1aaab1k3f", "GET", "cli")]});
        let (out, _) = go_at_terminal(&["approvals", "approve", "k3f"], vec![("approvals.list", Ok(both))]);
        assert_eq!(out.unwrap_err().code(), 2);
        let (out, _) =
            go_at_terminal(&["approvals", "approve", "x"], vec![("approvals.list", Ok(json!({"pending": []})))]);
        assert!(out.unwrap_err().to_string().contains("nothing is pending"));
    }
}
