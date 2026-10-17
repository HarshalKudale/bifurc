//! Named sessions end to end: connections that say who they are, commands through the transport session.

use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::{
    kernel::{CommandRegistry, EngineEventBus, bus::bus},
    rest::test_server,
    testing::scratch_data_root,
    transport::{
        event_log::EventLog,
        session::{Outbound, Session, SessionOptions},
    },
    utils::json::{array_of, str_of},
    workspace::{
        fs::{create_folder, read_entity, write_entity, write_flat_entity},
        settings::load_settings,
    },
};

const WS: &str = "ws1";

struct Connection {
    session: Session,
    rx: UnboundedReceiver<Outbound>,
    next: u32,
}

impl Connection {
    /// A command line connection; `name` is its `--session`.
    async fn open(registry: &Arc<CommandRegistry>, events: &'static EngineEventBus, name: Option<&str>) -> Self {
        let (tx, rx) = unbounded_channel();
        let session = Session::new(
            SessionOptions {
                registry: registry.clone(),
                log: EventLog::new(events),
                engine_version: "0.0.0".into(),
                host_domain: None,
                capabilities: vec![],
                scopes_for: None,
                on_auth_failure: None,
                on_session_start: None,
                on_session_end: None,
                refuse_command: None,
                peer: None,
                allowed_events: None,
            },
            tx,
        );
        let mut hello = json!({ "protocolVersion": "1.0.0", "clientName": "cli", "clientVersion": "1" });
        if let Some(name) = name {
            hello["session"] = json!(name);
        }
        let mut connection = Self { session, rx, next: 0 };
        assert_eq!(connection.reply("hello", hello).await["ok"], true);
        connection
    }

    async fn reply(&mut self, action: &str, payload: Value) -> Value {
        self.next += 1;
        let id = self.next.to_string();
        self.session.handle_frame(json!({ "id": id, "action": action, "payload": payload }));
        match self.rx.recv().await.expect("a reply") {
            Outbound::Frame(frame) | Outbound::FrameThenClose(frame) => frame,
            Outbound::Close => panic!("closed"),
        }
    }

    async fn ask(&mut self, action: &str, payload: Value) -> Value {
        let reply = self.reply(action, payload).await;
        assert_eq!(reply["ok"], true, "{action}: {reply}");
        reply["data"].clone()
    }

    /// Send a GET and return what the echo server received.
    async fn get(&mut self, url: &str, headers: Value, scripts: Value) -> Value {
        let request = json!({ "method": "GET", "url": url, "headers": headers, "body": "" });
        let sent = self.ask("request.send", json!({ "workspaceId": WS, "request": request, "scripts": scripts })).await;
        let body = crate::utils::replay::decode_base64_lenient(sent["body"].as_str().unwrap());
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    }
}

fn registry() -> Arc<CommandRegistry> {
    let mut registry = CommandRegistry::new();
    super::super::register(&mut registry);
    crate::cookies::register(&mut registry);
    crate::rest::register(&mut registry);
    crate::scripting::register(&mut registry);
    crate::flows::register(&mut registry);
    Arc::new(registry)
}

fn environment(id: &str, variables: &[(&str, &str)]) {
    let variables: Vec<Value> = variables.iter().map(|(k, v)| json!({ "id": k, "key": k, "value": v })).collect();
    let env = json!({ "id": id, "name": id, "createdAt": 0, "workspaceId": WS, "variables": variables });
    write_flat_entity(WS, "environments", id, &env).unwrap();
}

fn stored_has(key: &str) -> bool {
    ["__global__", "dev", "other"].iter().any(|id| {
        let env = read_entity(WS, "environments", id).unwrap();
        array_of(&env, "variables").iter().any(|variable| variable["key"] == key)
    })
}

#[tokio::test]
async fn sessions_keep_their_cookies_variables_and_environment_apart() {
    let _root = scratch_data_root();
    crate::cookies::forget(WS);
    let events: &'static EngineEventBus = Box::leak(Box::new(EngineEventBus::new()));
    let base = test_server::start(HashMap::new()).await;
    environment("__global__", &[("base", &base)]);
    environment("dev", &[("where", "dev")]);
    environment("other", &[]); // with two, none is the lone environment the stored selection falls back to
    let registry = registry();
    let mut a = Connection::open(&registry, events, Some("a")).await;
    let mut b = Connection::open(&registry, events, Some("b")).await;
    let mut app = Connection::open(&registry, events, None).await;
    let me = format!("{base}/me");
    let seen = |echo: &Value, header: &str| echo["headers"][header].clone();

    // Cookies: what a send in A is given is not sent in B, nor by the default session, which keeps the workspace's jar.
    a.get(&format!("{base}/set-cookie/sid=a"), json!({}), json!({})).await;
    assert_eq!(seen(&a.get(&me, json!({}), json!({})).await, "cookie"), "sid=a");
    assert_eq!(seen(&b.get(&me, json!({}), json!({})).await, "cookie"), Value::Null);
    assert_eq!(seen(&app.get(&me, json!({}), json!({})).await, "cookie"), Value::Null);
    assert_eq!(crate::cookies::for_url(WS, &base), json!({}), "the workspace jar never saw them");
    let listed = a.ask("cookies.list", json!({ "workspaceId": WS })).await;
    assert_eq!(listed["domains"][0]["cookies"][0]["name"], "sid");
    assert_eq!(b.ask("cookies.list", json!({ "workspaceId": WS })).await["domains"], json!([]));
    b.ask("cookies.set", json!({ "workspaceId": WS, "domain": "127.0.0.1", "name": "flag", "value": "on" })).await;
    assert_eq!(seen(&b.get(&me, json!({}), json!({})).await, "cookie"), "flag=on");
    assert_eq!(seen(&a.get(&me, json!({}), json!({})).await, "cookie"), "sid=a");

    // Runtime variables resolve in their session only.
    a.ask("session.setVariables", json!({ "variables": { "x": "from-a" } })).await;
    let header = json!({ "X-V": "{{x}}" });
    assert_eq!(seen(&a.get(&me, header.clone(), json!({})).await, "x-v"), "from-a");
    assert_eq!(seen(&b.get(&me, header.clone(), json!({})).await, "x-v"), "{{x}}");
    assert_eq!(seen(&app.get(&me, header, json!({})).await, "x-v"), "{{x}}");

    // A script's write lands in the session's layer and not in the environment's file.
    let write = json!({ "post": "bc.environment.set('tok', 't1');" });
    a.get(&me, json!({}), write.clone()).await;
    let got = a.ask("session.get", json!({})).await;
    assert_eq!(got["name"], "a");
    assert_eq!(got["variables"], json!([{ "key": "x", "value": "from-a" }, { "key": "tok", "value": "t1" }]));
    assert_eq!(got["cookieDomains"], 1);
    assert!(!stored_has("tok"));
    assert_eq!(seen(&a.get(&me, json!({ "T": "{{tok}}" }), json!({})).await, "t"), "t1");
    // In the default session a script still writes the file.
    app.get(&me, json!({}), write).await;
    assert!(stored_has("tok"));

    // `env use` in A is A's.
    a.ask("env.setActive", json!({ "id": "dev" })).await;
    assert_eq!(load_settings().stored_environment_id(WS), None, "the stored selection is the app's");
    let wants = json!({ "W": "{{where}}" });
    assert_eq!(seen(&a.get(&me, wants.clone(), json!({})).await, "w"), "dev");
    assert_eq!(seen(&b.get(&me, wants, json!({})).await, "w"), "{{where}}");
    assert_eq!(a.ask("session.get", json!({})).await["environmentId"], "dev");

    // A flow run in A has A's variables, environment and jar.
    let folder = create_folder(WS, "requests", "F", None, false).unwrap().0;
    let saved = json!({ "id": "r1", "name": "r1", "method": "GET", "url": me, "workspaceId": WS,
        "headers": { "X-V": "{{x}}", "X-W": "{{where}}" }, "body": "", "createdAt": 0 });
    write_entity(WS, "requests", "r1", &saved, Some(folder.as_str())).unwrap();
    let node = json!({ "id": "me", "type": "request", "requestKind": "requests", "requestId": "r1" });
    let edge = json!({ "id": "e", "from": "start", "port": "out", "to": "me" });
    let flow = json!({ "id": "f", "name": "f", "nodes": [{ "id": "start", "type": "start" }, node], "edges": [edge] });
    write_entity(WS, "flows", "f", &flow, None).unwrap();
    let finished = Arc::new(Mutex::new(None::<Value>));
    let sink = finished.clone();
    let listener = bus().on("flow.finished", move |payload| *sink.lock() = Some(payload["report"].clone()));
    a.ask("flow.run", json!({ "workspaceId": WS, "id": "f" })).await;
    for _ in 0..400 {
        if finished.lock().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bus().off(listener);
    let report = finished.lock().take().expect("the run finishes");
    let reached = array_of(&report, "nodes").iter().find(|n| n["id"] == "me").unwrap().clone();
    let echo: Value = serde_json::from_str(str_of(&reached["output"], "body")).unwrap();
    assert_eq!((&echo["headers"]["x-v"], &echo["headers"]["x-w"]), (&json!("from-a"), &json!("dev")), "{report}");
    assert_eq!(echo["headers"]["cookie"], "sid=a");

    // `--var` builds on the session's variables and wins over them.
    let request = json!({ "method": "GET", "url": me, "headers": { "X-V": "{{x}}", "X-Y": "{{y}}" }, "body": "" });
    let sent = a
        .ask(
            "request.send",
            json!({ "workspaceId": WS, "request": request, "scripts": {},
        "variables": { "y": "1" } }),
        )
        .await;
    let echo: Value =
        serde_json::from_slice(&crate::utils::replay::decode_base64_lenient(sent["body"].as_str().unwrap())).unwrap();
    assert_eq!((&echo["headers"]["x-v"], &echo["headers"]["x-y"]), (&json!("from-a"), &json!("1")));

    // Reset empties the session and nothing else.
    a.ask("session.reset", json!({})).await;
    let got = a.ask("session.get", json!({})).await;
    assert_eq!(got["variables"], json!([]));
    assert_eq!((got["environmentId"].clone(), got["cookieDomains"].clone()), (Value::Null, json!(0)));
    assert_eq!(seen(&b.get(&me, json!({}), json!({})).await, "cookie"), "flag=on");

    // `freshJar` sends with a jar of its own: it neither carries the workspace's cookies nor leaves any.
    app.get(&format!("{base}/set-cookie/w=1"), json!({}), json!({})).await;
    let request = json!({ "method": "GET", "url": format!("{base}/set-cookie/gone=1"), "headers": {}, "body": "" });
    let fresh = json!({ "workspaceId": WS, "request": request, "scripts": {}, "freshJar": true });
    app.ask("request.send", fresh).await;
    assert_eq!(crate::cookies::for_url(WS, &base), json!({ "w": "1" }));
    let echoed = app
        .ask(
            "request.send",
            json!({ "workspaceId": WS, "freshJar": true, "scripts": {},
        "request": { "method": "GET", "url": me, "headers": {}, "body": "" } }),
        )
        .await;
    let body = crate::utils::replay::decode_base64_lenient(echoed["body"].as_str().unwrap());
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["headers"]["cookie"], Value::Null);
    // Jars are process-wide: leave none behind for the tests that look at the workspace's.
    crate::cookies::forget(WS);
    crate::cookies::reset_session(WS, "b");
}
