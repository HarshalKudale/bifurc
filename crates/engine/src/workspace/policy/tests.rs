use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::*;
use crate::{kernel::CommandContext, testing::scratch_data_root, workspace::changes::as_client};

fn call(env: &str, method: &str, url: &str) -> Call {
    call_of(env.into(), method, url).0
}

fn judge(text: &str, env: &str, method: &str, url: &str) -> Verdict {
    evaluate(&parse("policy.yaml", text).unwrap(), &call(env, method, url))
}

fn deny(policy: &str) -> Verdict {
    Verdict::Deny { policy: policy.into() }
}

const LISTED: &str = "
policies:
  - name: no-prod-writes
    deny: { env: prod, methods: [DELETE, PUT] }
  - name: local-only
    allow_hosts: ['*.localhost', localhost, 127.0.0.1]
  - name: staging-deletes
    require_human: { env: staging, methods: [delete] }
";

#[test]
fn a_policy_is_read_from_the_list_form_the_flat_form_and_json() {
    let listed = parse("policy.yaml", LISTED).unwrap();
    assert_eq!(listed.apply_to, ["cli", "mcp"]);
    assert_eq!(listed.clauses.len(), 3);
    assert_eq!(listed.clauses[0].methods, ["DELETE", "PUT"]);
    assert_eq!(listed.clauses[2].methods, ["DELETE"], "methods are upper-cased");

    let flat = parse("policy.yaml", "apply_to: [app]\ndeny: { methods: DELETE }\nallow_hosts: localhost\n").unwrap();
    assert_eq!(flat.apply_to, ["app"]);
    let names: Vec<(&str, Kind)> = flat.clauses.iter().map(|c| (c.policy.as_str(), c.kind)).collect();
    assert_eq!(names, [("deny", Kind::Deny), ("allow_hosts", Kind::AllowHosts)]);

    let json = parse("policy.json", r#"{"policies":[{"name":"p","deny":{"hosts":["*.prod.x"]}}]}"#).unwrap();
    assert_eq!(json.clauses[0].hosts, ["*.prod.x"]);
    assert!(parse("policy.yaml", "").unwrap().clauses.is_empty(), "an empty file has no rules");
}

#[test]
fn a_policy_file_that_means_nothing_is_refused_with_a_reason() {
    for (text, reason) in [
        ("policies: [{deny: {}}, {name: x}]", "none of deny"),
        ("deny: { method: [GET] }", "unknown key \"method\""),
        ("apply_to: [web]", "use cli, mcp or app"),
        ("polices: []", "unknown key \"polices\""),
        ("policies: 3", "must be a list"),
        ("allow_hosts: []", "is empty"),
        ("deny: [", ""),
    ] {
        let error = parse("policy.yaml", text).unwrap_err();
        assert!(error.contains(reason), "{text}: {error}");
    }
}

#[test]
fn environments_methods_hosts_and_paths_match_as_documented() {
    let prod_writes = "deny: { env: prod, methods: [DELETE, PUT] }";
    assert_eq!(judge(prod_writes, "prod", "delete", "http://h/x"), deny("deny"));
    assert_eq!(judge(prod_writes, "prod", "GET", "http://h/x"), Verdict::Allow);
    assert_eq!(judge(prod_writes, "staging", "DELETE", "http://h/x"), Verdict::Allow);
    assert_eq!(judge(prod_writes, "", "DELETE", "http://h/x"), Verdict::Allow, "no environment is not prod");
    assert_eq!(judge("deny: { env: 'prod*' }", "prod-eu", "GET", "http://h/"), deny("deny"));
    assert_eq!(judge("deny: { env: [qa, Prod] }", "prod", "GET", "http://h/"), deny("deny"));
    assert_eq!(judge("deny: { methods: ['*'] }", "", "PATCH", "http://h/"), deny("deny"));

    let hosts = "deny: { hosts: ['*.prod.com'] }";
    assert_eq!(judge(hosts, "", "GET", "https://api.prod.com:8443/x"), deny("deny"), "the port is ignored");
    assert_eq!(judge(hosts, "", "GET", "https://a.b.prod.com/x"), deny("deny"));
    assert_eq!(judge(hosts, "", "GET", "https://prod.com/x"), Verdict::Allow, "*. is subdomains only");
    assert_eq!(judge("deny: { hosts: [prod.com] }", "", "GET", "https://PROD.com:1/x"), deny("deny"));

    let paths = "deny: { paths: ['/orders/*', '/admin/**'] }";
    assert_eq!(judge(paths, "", "GET", "http://h/orders/1?x=1"), deny("deny"));
    assert_eq!(judge(paths, "", "GET", "http://h/orders/1/items"), Verdict::Allow, "* stays in a segment");
    assert_eq!(judge(paths, "", "GET", "http://h/admin/a/b/c"), deny("deny"), "** crosses segments");

    assert_eq!(judge("deny:", "", "GET", "http://h/"), deny("deny"), "a rule with no match keys matches everything");
    assert_eq!(judge("deny: {}", "x", "TRACE", "http://h/"), deny("deny"));
}

#[test]
fn allow_hosts_blocks_a_host_it_does_not_list() {
    let local = "allow_hosts: ['*.localhost', localhost, 127.0.0.1]";
    for url in ["http://localhost:3000/", "http://api.localhost/", "http://127.0.0.1:18596/x"] {
        assert_eq!(judge(local, "", "GET", url), Verdict::Allow, "{url}");
    }
    for url in ["http://example.com/", "http://localhost.evil.com/", "nonsense"] {
        assert_eq!(judge(local, "", "GET", url), deny("allow_hosts"), "{url}");
    }
}

#[test]
fn deny_and_allow_hosts_come_first_in_file_order_then_require_human() {
    // The human rule is first in the file and also matches, yet the deny below it wins.
    let text = "
policies:
  - { name: ask, require_human: { methods: [DELETE] } }
  - { name: first, deny: { methods: [DELETE] } }
  - { name: second, deny: {} }
";
    assert_eq!(judge(text, "", "DELETE", "http://h/"), deny("first"));
    assert_eq!(judge(text, "", "GET", "http://h/"), deny("second"));
    let only_ask = "policies: [{ name: ask, require_human: { env: staging, methods: [DELETE] } }]";
    assert_eq!(judge(only_ask, "staging", "DELETE", "http://h/"), Verdict::RequireHuman { policy: "ask".into() });
    assert_eq!(judge(only_ask, "prod", "DELETE", "http://h/"), Verdict::Allow);
}

fn write_policy(root: &crate::testing::ScratchRoot, file: &str, text: &str) {
    std::fs::write(ws_dir(root.ws_id).join(file), text).unwrap();
}

#[test]
fn a_policy_applies_to_the_command_line_by_default_and_to_the_app_only_when_listed() {
    let root = scratch_data_root();
    let go = |client: &str| verdict("ws1", client, Some("prod"), "DELETE", "http://h/x").0;
    assert_eq!(go("cli"), Verdict::Allow, "no policy file, no policy");
    write_policy(&root, "policy.yaml", "deny: { env: prod }");
    assert_eq!(go("cli"), deny("deny"));
    assert_eq!(go("desktop-ui"), Verdict::Allow);
    assert_eq!(go("bifurc-extension"), Verdict::Allow);
    assert_eq!(go(""), Verdict::Allow, "engine-internal work is never held");
    write_policy(&root, "policy.yaml", "apply_to: [cli, app]\ndeny: { env: prod }");
    assert_eq!(go("desktop-ui"), deny("deny"));
    write_policy(&root, "policy.yaml", "apply_to: [mcp]\ndeny: { env: prod }");
    assert_eq!(go("cli"), deny("deny"), "mcp and cli are one switch: both connect as cli");
    assert_eq!(go("desktop-ui"), Verdict::Allow);
}

#[test]
fn a_policy_file_that_cannot_be_read_blocks_the_agent_and_not_the_app() {
    let root = scratch_data_root();
    write_policy(&root, "policy.yaml", "deny: [");
    let (cli, _) = verdict("ws1", "cli", Some(""), "GET", "http://h/");
    assert!(matches!(cli, Verdict::Invalid(_)), "{cli:?}");
    assert_eq!(verdict("ws1", "desktop-ui", Some(""), "GET", "http://h/").0, Verdict::Allow);
    let error = invalid("boom");
    assert_eq!((error.code, error.message.as_str()), (ErrorCode::PolicyBlocked, "policy file invalid: boom"));
}

#[test]
fn an_edit_of_the_file_is_picked_up_and_yaml_wins_over_json() {
    let root = scratch_data_root();
    let go = || verdict("ws1", "cli", Some(""), "GET", "http://h/x").0;
    write_policy(&root, "policy.yaml", "deny: { methods: [POST] }");
    assert_eq!(go(), Verdict::Allow);
    write_policy(&root, "policy.yaml", "deny: { methods: [POST, GET] }");
    assert_eq!(go(), deny("deny"));
    write_policy(&root, "policy.json", "{}");
    assert_eq!(go(), deny("deny"), "policy.yaml wins");
    std::fs::remove_file(ws_dir("ws1").join("policy.yaml")).unwrap();
    assert_eq!(go(), Verdict::Allow, "the json file now rules, and it is empty");
    std::fs::remove_file(ws_dir("ws1").join("policy.json")).unwrap();
    assert!(load("ws1").is_none());
}

/// A listener that counts connections and never answers.
async fn silent_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(AtomicUsize::new(0));
    let count = seen.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            held.push(socket);
        }
    });
    (base, seen)
}

#[tokio::test]
async fn a_send_by_the_command_line_is_stopped_before_the_network_and_the_apps_goes_through() {
    let root = scratch_data_root();
    let (base, seen) = silent_server().await;
    let mut registry = CommandRegistry::new();
    crate::rest::register(&mut registry);
    crate::workspace::policy::register(&mut registry);
    let registry = Arc::new(registry);
    let ctx = CommandContext::internal(registry.clone());
    write_policy(&root, "policy.yaml", "policies:\n  - { name: no-deletes, deny: { methods: [DELETE] } }\n");
    let send = |client: &'static str, method: &str, query: &str| {
        let request = json!({ "method": method, "url": format!("{base}/orders/1{query}"), "headers": {}, "body": "", "settings": { "timeoutMs": 300 } });
        let payload = json!({ "workspaceId": "ws1", "request": request, "scripts": {} });
        let (registry, ctx) = (registry.clone(), ctx.clone());
        async move { as_client(Some(client), registry.invoke("request.send", payload, ctx)).await }
    };

    let blocked = send("cli", "DELETE", "?token=s3cret").await.unwrap_err();
    assert_eq!(blocked.code, ErrorCode::PolicyBlocked, "{}", blocked.message);
    assert_eq!(blocked.message, format!("blocked by policy \"no-deletes\": DELETE {base}/orders/1"));
    assert_eq!(blocked.key(), Some("policy.blocked"));
    assert_eq!(blocked.params().unwrap()["policy"], "no-deletes");
    assert_eq!(seen.load(Ordering::SeqCst), 0, "nothing reached the server");

    // The same send as the desktop app is not held, so it dials the server (which never answers).
    let app = send("desktop-ui", "DELETE", "").await;
    assert!(app.is_err_and(|e| e.code != ErrorCode::PolicyBlocked));
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    // So is a command line GET.
    let get = send("cli", "GET", "").await;
    assert!(get.is_err_and(|e| e.code != ErrorCode::PolicyBlocked));
    assert_eq!(seen.load(Ordering::SeqCst), 2);

    let checked = |method: &str, url: &str, env: Value| {
        let mut payload = json!({ "workspaceId": "ws1", "method": method, "url": url });
        if !env.is_null() {
            payload["env"] = env;
        }
        let (registry, ctx) = (registry.clone(), ctx.clone());
        async move { as_client(Some("cli"), registry.invoke("policy.check", payload, ctx)).await.unwrap() }
    };
    let denied = checked("DELETE", "http://h/x", json!("prod")).await;
    assert_eq!((&denied["decision"], &denied["policy"]), (&json!("deny"), &json!("no-deletes")));
    assert_eq!(checked("GET", "http://h/x", Value::Null).await["decision"], "allow");
    write_policy(&root, "policy.yaml", "require_human: { methods: [DELETE] }");
    let asked = checked("DELETE", "http://h/x", Value::Null).await;
    assert_eq!((&asked["decision"], &asked["policy"]), (&json!("require_human"), &json!("require_human")));
    let held = send("cli", "DELETE", "").await.unwrap_err();
    assert_eq!(held.code, ErrorCode::PolicyApproval);
    assert!(held.message.starts_with("needs approval: policy \"require_human\" (DELETE "), "{}", held.message);
    assert_eq!(seen.load(Ordering::SeqCst), 2);

    let shown = registry.invoke("policy.show", json!({ "workspaceId": "ws1" }), ctx).await.unwrap();
    assert_eq!(shown["file"], "policy.yaml");
    assert_eq!(shown["policies"][0]["kind"], "require_human");
    assert_eq!(shown["policies"][0]["methods"], json!(["DELETE"]));
}

#[tokio::test]
async fn a_websocket_or_event_stream_the_command_line_opens_is_held_too() {
    let root = scratch_data_root();
    let (base, seen) = silent_server().await;
    let mut registry = CommandRegistry::new();
    crate::websocket::register(&mut registry);
    crate::sse::register(&mut registry);
    let registry = Arc::new(registry);
    let ctx = CommandContext::internal(registry.clone());
    write_policy(&root, "policy.yaml", "policies:\n  - { name: local-names, allow_hosts: [\"*.localhost\"] }\n");
    let ws_url = base.replacen("http://", "ws://", 1);
    for (command, payload) in [
        ("ws.connect", json!({ "workspaceId": "ws1", "socketId": "s1", "url": format!("{ws_url}/live") })),
        ("sse.connect", json!({ "workspaceId": "ws1", "streamId": "e1", "url": format!("{base}/events") })),
    ] {
        let blocked = as_client(Some("cli"), registry.invoke(command, payload, ctx.clone())).await.unwrap_err();
        assert_eq!(blocked.code, ErrorCode::PolicyBlocked, "{command}: {}", blocked.message);
        assert!(blocked.message.starts_with("blocked by policy \"local-names\""), "{}", blocked.message);
    }
    assert_eq!(seen.load(Ordering::SeqCst), 0, "nothing was dialled");
}
