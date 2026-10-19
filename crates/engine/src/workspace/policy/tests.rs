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
    let _wait = short_wait(100);
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

// ---- approvals ----

/// For the length of a test, a held call waits this long (ms) instead of the setting's minutes.
fn short_wait(ms: u64) -> impl Drop {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            approvals::TEST_TIMEOUT_MS.store(0, Ordering::SeqCst);
        }
    }
    approvals::TEST_TIMEOUT_MS.store(ms, Ordering::SeqCst);
    approvals::reset();
    Reset
}

/// A server that answers 200 and counts the requests.
async fn ok_server() -> (String, Arc<AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(AtomicUsize::new(0));
    let count = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let count = count.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let _ = socket.read(&mut buf).await;
                count.fetch_add(1, Ordering::SeqCst);
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok").await;
            });
        }
    });
    (base, seen)
}

struct Harness {
    registry: Arc<CommandRegistry>,
    base: String,
    seen: Arc<AtomicUsize>,
}

impl Harness {
    async fn new(root: &crate::testing::ScratchRoot, policy: &str) -> Self {
        let (base, seen) = ok_server().await;
        let mut registry = CommandRegistry::new();
        crate::rest::register(&mut registry);
        crate::websocket::register(&mut registry);
        crate::workspace::policy::register(&mut registry);
        write_policy(root, "policy.yaml", policy);
        Harness { registry: Arc::new(registry), base, seen }
    }

    fn ctx(&self) -> CommandContext {
        CommandContext::internal(self.registry.clone())
    }

    /// A send as the command line; spawned, so several can wait at once.
    fn send(&self, method: &str, path: &str) -> tokio::task::JoinHandle<Result<Value>> {
        let request = json!({ "method": method, "url": format!("{}{path}", self.base), "headers": {}, "body": "", "settings": { "timeoutMs": 2000 } });
        let payload = json!({ "workspaceId": "ws1", "request": request, "scripts": {} });
        let (registry, ctx) = (self.registry.clone(), self.ctx());
        tokio::spawn(as_client(Some("cli"), async move { registry.invoke("request.send", payload, ctx).await }))
    }

    async fn run(&self, command: &str, payload: Value) -> Result<Value> {
        as_client(Some("cli"), self.registry.invoke(command, payload, self.ctx())).await
    }

    async fn pending(&self) -> Vec<Value> {
        let got = self.run("approvals.list", json!({})).await.unwrap();
        got["pending"].as_array().unwrap().clone()
    }

    /// Wait until `n` calls are queued.
    async fn queued(&self, n: usize) -> Vec<Value> {
        for _ in 0..200 {
            let pending = self.pending().await;
            if pending.len() == n {
                return pending;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("{n} approvals never queued");
    }

    async fn decide(&self, id: &str, decision: &str, reason: Option<&str>) -> Result<Value> {
        let mut payload = json!({ "id": id, "decision": decision, "interactive": true });
        if let Some(reason) = reason {
            payload["reason"] = json!(reason);
        }
        self.run("approvals.decide", payload).await
    }
}

const ASK: &str = "policies: [{ name: staging-deletes, require_human: { methods: [DELETE] } }]";

fn id_in(message: &str) -> String {
    message.split("pending as #").nth(1).and_then(|rest| rest.split(';').next()).unwrap().to_string()
}

#[tokio::test]
async fn a_waiting_send_goes_through_when_a_person_approves_and_every_step_is_logged() {
    let root = scratch_data_root();
    let _wait = short_wait(10_000);
    let h = Harness::new(&root, ASK).await;
    let events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sink = events.clone();
    let listener = crate::kernel::bus().on("approval.requested", move |payload| sink.lock().push(payload.clone()));
    let cursor = changes::cursor();

    let sending = h.send("DELETE", "/orders/1?token=s3cret");
    let queued = h.queued(1).await;
    let item = &queued[0];
    assert_eq!(
        (&item["policy"], &item["method"], &item["workspaceId"], &item["by"]),
        (&json!("staging-deletes"), &json!("DELETE"), &json!("ws1"), &json!("cli"))
    );
    assert_eq!(item["url"], format!("{}/orders/1", h.base), "no query in the queue");
    assert!(item["expiresAt"].as_i64() > item["createdAt"].as_i64());
    assert_eq!(h.seen.load(Ordering::SeqCst), 0, "nothing is sent while it waits");
    assert_eq!(events.lock().len(), 1);
    // The engine is not stuck while a command waits: another command answers.
    assert_eq!(h.run("policy.show", json!({ "workspaceId": "ws1" })).await.unwrap()["file"], "policy.yaml");

    let id = item["id"].as_str().unwrap();
    let decided = h.decide(id, "approve", None).await.unwrap();
    assert_eq!((&decided["decision"], &decided["decidedBy"]), (&json!("approved"), &json!("cli")));
    assert!(decided.get("passUntil").is_none(), "the waiting call took the approval itself");
    sending.await.unwrap().unwrap();
    assert_eq!(h.seen.load(Ordering::SeqCst), 1);
    assert!(h.pending().await.is_empty());
    let all = h.run("approvals.list", json!({ "all": true })).await.unwrap();
    assert_eq!(all["decided"][0]["id"], id);
    assert_eq!(all["decided"][0]["decision"], "approved");
    // No pass was left: the same call waits again.
    let again = h.send("DELETE", "/orders/1");
    h.queued(1).await;
    again.abort();

    let feed = changes::since("ws1", id, cursor);
    let actions: Vec<&str> = feed.iter().map(|c| c["action"].as_str().unwrap()).collect();
    assert_eq!(actions, ["requested", "approved"]);
    assert!(!format!("{feed:?}").contains("s3cret"));
    crate::kernel::bus().off(listener);
}

#[tokio::test]
async fn a_denied_send_fails_as_blocked_with_who_and_why_and_is_never_sent() {
    let root = scratch_data_root();
    let _wait = short_wait(10_000);
    let h = Harness::new(&root, ASK).await;
    let sending = h.send("DELETE", "/orders/2");
    let id = h.queued(1).await[0]["id"].as_str().unwrap().to_string();
    let decided = h.decide(&id, "deny", Some("not on friday")).await.unwrap();
    assert_eq!((&decided["decision"], &decided["reason"]), (&json!("denied"), &json!("not on friday")));
    let error = sending.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::PolicyBlocked);
    assert_eq!(
        error.message,
        format!("denied by cli: policy \"staging-deletes\" (DELETE {}/orders/2): not on friday", h.base)
    );
    assert_eq!(h.seen.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_send_nobody_answers_times_out_with_its_id_and_a_late_approval_lets_one_identical_retry_pass() {
    let root = scratch_data_root();
    let _wait = short_wait(150);
    let h = Harness::new(&root, ASK).await;
    let timed_out = h.send("DELETE", "/orders/3").await.unwrap().unwrap_err();
    assert_eq!(timed_out.code, ErrorCode::PolicyApproval);
    let id = id_in(&timed_out.message);
    assert!(
        timed_out.message.ends_with(&format!("approve with: bifurc approvals approve {id}")),
        "{}",
        timed_out.message
    );
    assert!(timed_out.message.starts_with("needs approval: policy \"staging-deletes\" (DELETE "));
    assert_eq!(h.pending().await.len(), 1, "it stays pending for a late approval");

    let decided = h.decide(&id, "approve", None).await.unwrap();
    assert!(decided["passUntil"].as_i64() > decided["decidedAt"].as_i64(), "a pass was left");
    h.send("DELETE", "/orders/3").await.unwrap().unwrap();
    assert_eq!(h.seen.load(Ordering::SeqCst), 1);
    // The pass is used up; a different call never had one.
    assert_eq!(h.send("DELETE", "/orders/3").await.unwrap().unwrap_err().code, ErrorCode::PolicyApproval);
    assert_eq!(h.send("DELETE", "/orders/4").await.unwrap().unwrap_err().code, ErrorCode::PolicyApproval);
    assert_eq!(h.seen.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn two_sends_wait_at_once_without_blocking_each_other_or_the_queue() {
    let root = scratch_data_root();
    let _wait = short_wait(10_000);
    let h = Harness::new(&root, ASK).await;
    let (a, b) = (h.send("DELETE", "/a"), h.send("DELETE", "/b"));
    let queued = h.queued(2).await;
    // Decide the second first: each waiter is woken by its own item.
    for item in queued.iter().rev() {
        h.decide(item["id"].as_str().unwrap(), "approve", None).await.unwrap();
    }
    a.await.unwrap().unwrap();
    b.await.unwrap().unwrap();
    assert_eq!(h.seen.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_connect_cannot_wait_so_it_queues_once_and_the_retry_after_approval_passes() {
    let root = scratch_data_root();
    let _wait = short_wait(10_000);
    let h = Harness::new(&root, "require_human: {}").await;
    let url = h.base.replacen("http://", "ws://", 1);
    let connect =
        || h.run("ws.connect", json!({ "workspaceId": "ws1", "socketId": "s1", "url": format!("{url}/live") }));
    let first = connect().await.unwrap_err();
    assert_eq!(first.code, ErrorCode::PolicyApproval);
    let id = id_in(&first.message);
    let second = connect().await.unwrap_err();
    assert_eq!(id_in(&second.message), id, "an identical pending item is reused");
    assert_eq!(h.pending().await.len(), 1);

    h.decide(&id, "approve", None).await.unwrap();
    // The pass lets the connect go on: it dials and fails there (the server is not a WebSocket), not at the policy.
    let retried = connect().await;
    assert!(retried.is_ok() || retried.is_err_and(|e| e.code != ErrorCode::PolicyApproval));
    assert_eq!(connect().await.unwrap_err().code, ErrorCode::PolicyApproval, "one pass, one connect");
}

#[tokio::test]
async fn only_a_terminal_decides_and_an_unknown_or_settled_id_says_so() {
    let root = scratch_data_root();
    let _wait = short_wait(10_000);
    let h = Harness::new(&root, ASK).await;
    let sending = h.send("DELETE", "/orders/5");
    let id = h.queued(1).await[0]["id"].as_str().unwrap().to_string();
    for payload in
        [json!({ "id": id, "decision": "approve", "interactive": false }), json!({ "id": id, "decision": "approve" })]
    {
        let error = h.run("approvals.decide", payload).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::BadRequest, "{}", error.message);
    }
    assert_eq!(h.pending().await.len(), 1, "nothing was decided");
    assert_eq!(h.decide("nope", "deny", None).await.unwrap_err().code, ErrorCode::NotFound);
    h.decide(&id, "deny", None).await.unwrap();
    assert_eq!(h.decide(&id, "approve", None).await.unwrap_err().code, ErrorCode::Conflict);
    sending.await.unwrap().unwrap_err();
}

#[tokio::test]
async fn the_decided_ring_keeps_100_pending_items_expire_and_list_filters_by_workspace() {
    let root = scratch_data_root();
    let _wait = short_wait(10_000);
    let h = Harness::new(&root, ASK).await;
    for n in 0..105 {
        let id = id_in(&approvals::hold_now("ws1", "p", "DELETE", &format!("http://h/{n}")).unwrap_err().message);
        h.decide(&id, "deny", None).await.unwrap();
    }
    let all = h.run("approvals.list", json!({ "all": true })).await.unwrap();
    assert_eq!(all["decided"].as_array().unwrap().len(), 100);
    assert_eq!(all["decided"][0]["url"], "http://h/104", "newest first");

    let id = id_in(&approvals::hold_now("ws1", "p", "GET", "http://h/old").unwrap_err().message);
    approvals::hold_now("other", "p", "GET", "http://h/x").unwrap_err();
    assert_eq!(h.run("approvals.list", json!({})).await.unwrap()["pending"].as_array().unwrap().len(), 2);
    let only = h.run("approvals.list", json!({ "workspaceId": "other", "all": true })).await.unwrap();
    assert_eq!((only["pending"].as_array().unwrap().len(), only["decided"].as_array().unwrap().len()), (1, 0));
    approvals::expire_now(&id);
    let after = h.run("approvals.list", json!({ "all": true })).await.unwrap();
    assert_eq!(after["pending"].as_array().unwrap().len(), 1);
    assert_eq!((&after["decided"][0]["id"], &after["decided"][0]["decision"]), (&json!(id), &json!("expired")));
    assert_eq!(h.decide(&id, "approve", None).await.unwrap_err().code, ErrorCode::Conflict);
}

#[tokio::test]
async fn a_block_is_written_to_the_audit_trail_without_the_secret_in_the_url() {
    let root = scratch_data_root();
    let _wait = short_wait(10_000);
    let h = Harness::new(&root, "policies: [{ name: no-deletes, deny: { methods: [DELETE] } }]").await;
    let cursor = changes::cursor();
    let error = h.send("DELETE", "/orders/6?token=s3cret").await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::PolicyBlocked);
    let feed = changes::since("ws1", "", cursor);
    assert_eq!(feed.len(), 1, "{feed:?}");
    assert_eq!((feed[0]["kind"].as_str(), feed[0]["action"].as_str()), (Some("approval"), Some("blocked")));
    assert_eq!(feed[0]["name"], format!("DELETE {}/orders/6", h.base));
    assert!(!feed[0].to_string().contains("s3cret"));
    // The file survives a restart, whoever launched the engine.
    let file = std::fs::read_to_string(root.dir.path().join("audit").join("policy.jsonl")).unwrap();
    let line: Value = serde_json::from_str(file.lines().last().unwrap()).unwrap();
    assert_eq!((line["action"].as_str(), line["policy"].as_str()), (Some("blocked"), Some("no-deletes")));
    assert!(!file.contains("s3cret"));
}
