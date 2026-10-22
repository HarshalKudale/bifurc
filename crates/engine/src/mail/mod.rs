//! The local mail sink (`mail.*`): an SMTP server on 127.0.0.1 that keeps the last mails it receives, so a test
//! can read the verification code or link an app "sent". A mail goes to the inbox of the workspace that is active
//! when it arrives; each inbox is written beside `app.json` (never in the workspace's repository).
//!
//! The same port also answers `GET /mail/wait` over HTTP (see [`http`]): a flow reaches the sink with an
//! ordinary request step, which is how the `mail:` step of a flow works without a node of its own.

mod extract;
mod mime;
mod smtp;

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{io::AsyncReadExt, io::AsyncWriteExt, net::TcpStream, time::timeout};

use crate::{
    env::session::workspace_of,
    kernel::{CommandRegistry, bus},
    protocol::EngineError,
    proxy::listener::{ConnectionHandler, ListenerCore},
    utils::{
        id::{mk_id, now_ms},
        json::{opt_str, str_of},
    },
    workspace::{paths::data_dir, settings::AppSettings},
};

/// How many mails are kept; the oldest go first.
const MAX_MAILS: usize = 200;

/// The largest message accepted (SMTP code 552 above it).
const MAX_MESSAGE: usize = 10 * 1024 * 1024;

/// The longest a `mail.wait` or `/mail/wait` may be asked to wait.
const MAX_WAIT_MS: u64 = 600_000;

/// How often a waiter looks for a new mail. shortcut: polling, 100ms late at worst; upgrade to a Notify if
/// mail volume ever makes it matter.
const POLL: Duration = Duration::from_millis(100);

/// How long a new connection is given to speak first. SMTP clients wait for our greeting, HTTP ones do not.
const SNIFF: Duration = Duration::from_millis(100);

#[derive(Clone, Serialize, Deserialize)]
struct Mail {
    id: String,
    timestamp: i64,
    from: String,
    to: Vec<String>,
    subject: String,
    text: String,
    html: String,
}

impl Mail {
    fn summary(&self) -> Value {
        json!({ "id": self.id, "timestamp": self.timestamp, "from": self.from, "to": self.to, "subject": self.subject })
    }

    /// Everything, with the code and the link found in it (`link_match`: the first link containing it).
    fn full(&self, link_match: Option<&str>) -> Value {
        let mut value = self.summary();
        value["text"] = json!(self.text);
        value["html"] = json!(self.html);
        value["otp"] = json!(extract::otp(&self.subject, &self.text, &self.html));
        value["link"] = json!(extract::link(&self.text, &self.html, link_match));
        value
    }

    /// Whether the mail went to `to`: the whole address when it has an `@`, else any part of one.
    fn addressed(&self, to: &str) -> bool {
        let to = to.trim().to_lowercase();
        self.to.iter().any(|address| if to.contains('@') { *address == to } else { address.contains(&to) })
    }
}

/// By workspace id; an inbox is read from its file the first time it is asked for.
static INBOXES: LazyLock<Mutex<HashMap<String, VecDeque<Mail>>>> = LazyLock::new(Default::default);
static SINK: LazyLock<ListenerCore> = LazyLock::new(ListenerCore::default);

/// Add `mail`, dropping the oldest beyond `limit`.
fn keep(inbox: &mut VecDeque<Mail>, mail: Mail, limit: usize) {
    inbox.push_back(mail);
    while inbox.len() > limit {
        inbox.pop_front();
    }
}

fn inbox_file(ws_id: &str) -> PathBuf {
    data_dir().join("mail").join(format!("{ws_id}.json"))
}

/// The workspace a mail arriving now belongs to: the active one.
fn active_workspace() -> String {
    workspace_of(&Value::Null)
}

fn with_inbox<T>(ws_id: &str, work: impl FnOnce(&mut VecDeque<Mail>) -> T) -> T {
    let mut inboxes = INBOXES.lock();
    let inbox = inboxes.entry(ws_id.to_string()).or_insert_with(|| {
        std::fs::read(inbox_file(ws_id)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    });
    work(inbox)
}

/// Write `ws_id`'s inbox; before onboarding there is no workspace, and its mails stay in memory.
fn save(ws_id: &str, inbox: &VecDeque<Mail>) {
    if ws_id.is_empty() {
        return;
    }
    let file = inbox_file(ws_id);
    let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
    if let Ok(bytes) = serde_json::to_vec(inbox) {
        let _ = std::fs::write(file, bytes);
    }
}

/// Forget every inbox held in memory, so a new data root (a test's scratch store) starts without mails.
pub fn forget_all() {
    INBOXES.lock().clear();
}

/// Drop a deleted workspace's inbox and its file.
pub fn forget(ws_id: &str) {
    INBOXES.lock().remove(ws_id);
    let _ = std::fs::remove_file(inbox_file(ws_id));
}

/// A message the SMTP session received; returns its id. Recipients are the envelope's and the `To` header's.
pub(crate) fn receive(envelope_from: &str, envelope_to: &[String], raw: &[u8]) -> String {
    let parsed = mime::parse(raw);
    let mut to: Vec<String> = Vec::new();
    for address in envelope_to.iter().chain(&parsed.to) {
        if !address.is_empty() && !to.contains(address) {
            to.push(address.clone());
        }
    }
    let from = if parsed.from.is_empty() { envelope_from.to_string() } else { parsed.from };
    let id = mk_id();
    let mail = Mail {
        id: id.clone(),
        timestamp: now_ms(),
        from,
        to,
        subject: parsed.subject,
        text: parsed.text,
        html: parsed.html,
    };
    let summary = mail.summary();
    let ws_id = active_workspace();
    with_inbox(&ws_id, |inbox| {
        keep(inbox, mail, MAX_MAILS);
        save(&ws_id, inbox);
    });
    bus().emit("mail.changed", json!({ "workspaceId": ws_id, "received": summary }));
    id
}

/// Which mail is wanted. `to` is matched as [`Mail::addressed`] does; the rest are a part of the sender, the
/// subject, or the text or HTML body, in any case. What is not given matches every mail.
#[derive(Default)]
pub struct Filter {
    pub to: Option<String>,
    pub from: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
}

impl Filter {
    fn matches(&self, mail: &Mail) -> bool {
        let has = |text: &str, part: &Option<String>| {
            part.as_ref().is_none_or(|part| text.to_lowercase().contains(&part.trim().to_lowercase()))
        };
        self.to.as_deref().is_none_or(|to| mail.addressed(to))
            && has(&mail.from, &self.from)
            && has(&mail.subject, &self.subject)
            && (has(&mail.text, &self.body) || has(&mail.html, &self.body))
    }
}

/// The newest mail of `ws_id` not older than `since`, sent to `to` when that is given.
#[cfg(test)]
fn newest(ws_id: &str, to: Option<&str>, since: i64) -> Option<Mail> {
    newest_matching(ws_id, &Filter { to: to.map(str::to_string), ..Filter::default() }, since)
}

fn newest_matching(ws_id: &str, filter: &Filter, since: i64) -> Option<Mail> {
    with_inbox(ws_id, |inbox| inbox.iter().rev().find(|m| m.timestamp >= since && filter.matches(m)).cloned())
}

/// The newest mail that `filter` matches, waiting up to `timeout_ms` for one to arrive.
async fn wait_for(ws_id: &str, filter: &Filter, since: i64, timeout_ms: u64) -> Option<Mail> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(MAX_WAIT_MS));
    loop {
        if let Some(mail) = newest_matching(ws_id, filter, since) {
            return Some(mail);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// For a flow's mail node: the newest mail of `ws_id` that `filter` matches and that arrived at `since` or
/// later, with its code and link, waiting up to `timeout_ms` for one.
pub async fn wait(ws_id: &str, filter: &Filter, since: i64, timeout_ms: u64) -> Option<Value> {
    wait_for(ws_id, filter, since, timeout_ms).await.map(|mail| mail.full(None))
}

async fn reply(socket: &mut TcpStream, status: &str, body: &Value) {
    let body = body.to_string();
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(body.as_bytes()).await;
    let _ = socket.shutdown().await;
}

/// `GET /mail/wait?to=&timeoutMs=&linkMatch=`: the newest mail of the active workspace for `to` as `{id, subject, from, otp, link}`,
/// waiting up to `timeoutMs` (default 30s) for one; 408 `{error}` when none came.
async fn http(mut socket: TcpStream) {
    let mut buf = vec![0u8; 8192];
    let Ok(Ok(read)) = timeout(Duration::from_secs(5), socket.read(&mut buf)).await else { return };
    let request = String::from_utf8_lossy(&buf[..read]).into_owned();
    let target = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    // A loopback Host only: a web page that rebinds its own name to 127.0.0.1 must not read the codes.
    let host = request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("host").then(|| value.trim().to_ascii_lowercase())
    });
    let host = host.as_deref().map(|h| h.rsplit_once(':').filter(|(_, p)| !p.contains(']')).map_or(h, |(h, _)| h));
    if !matches!(host, Some("127.0.0.1" | "localhost" | "[::1]")) {
        return reply(&mut socket, "403 Forbidden", &json!({ "error": "only a loopback Host is served" })).await;
    }
    if path != "/mail/wait" {
        return reply(&mut socket, "404 Not Found", &json!({ "error": "only GET /mail/wait is served here" })).await;
    }
    // `+` stays a plus: an address may contain one.
    let param = |key: &str| {
        query.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            (name == key).then(|| percent_decode_str(value).decode_utf8_lossy().into_owned())
        })
    };
    let (to, link_match) = (param("to").filter(|t| !t.is_empty()), param("linkMatch"));
    let wait_ms = param("timeoutMs").and_then(|ms| ms.parse().ok()).unwrap_or(30_000);
    let filter = Filter { to: to.clone(), ..Filter::default() };
    match wait_for(&active_workspace(), &filter, 0, wait_ms).await {
        Some(mail) => {
            let full = mail.full(link_match.as_deref());
            let body = json!({ "id": full["id"], "subject": full["subject"], "from": full["from"], "otp": full["otp"], "link": full["link"] });
            reply(&mut socket, "200 OK", &body).await
        }
        None => {
            let who = to.map_or(String::new(), |to| format!(" for {to}"));
            reply(&mut socket, "408 Request Timeout", &json!({ "error": format!("no mail{who} within {wait_ms}ms") }))
                .await
        }
    }
}

/// A connection to the sink: SMTP unless the client opens with an HTTP `GET`.
fn handler(max: usize) -> ConnectionHandler {
    Arc::new(move |socket| {
        Box::pin(async move {
            let mut head = [0u8; 4];
            let spoke =
                matches!(timeout(SNIFF, socket.peek(&mut head)).await, Ok(Ok(n)) if n >= 3 && head.starts_with(b"GET"));
            if spoke { http(socket).await } else { smtp::session(socket, max).await }
        })
    })
}

/// Serve the configured port unless the sink already does. A port that cannot be bound leaves the sink off,
/// with the reason in [`status`]; the engine goes on.
pub fn ensure(settings: &AppSettings) {
    if !SINK.is_running() || SINK.port() != settings.mail_port() {
        SINK.start(settings.mail_port(), handler(MAX_MESSAGE));
    }
}

/// After a settings change: move the sink to the new port, once it has been started (before the first-run
/// setup confirms the servers, nothing is).
pub fn rebind(next: &AppSettings, prev: &AppSettings) {
    if next.mail_port() != prev.mail_port() && SINK.port() != 0 {
        ensure(next);
    }
}

fn status_of(sink: &ListenerCore, ws_id: &str) -> Value {
    let error = sink.error().map(|e| e.message);
    let count = with_inbox(ws_id, |inbox| inbox.len());
    json!({ "port": sink.port(), "running": sink.is_running(), "error": error, "count": count })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("mail.status", |params, _| Ok(status_of(&SINK, &workspace_of(&params))));

    registry.register_sync("mail.list", |params, _| {
        let to = opt_str(&params, "to");
        let last = params["last"].as_u64().map_or(20, |n| n as usize);
        with_inbox(&workspace_of(&params), |inbox| {
            let matched: Vec<&Mail> = inbox.iter().rev().filter(|m| to.is_none_or(|to| m.addressed(to))).collect();
            let shown: Vec<Value> = matched.iter().take(last).map(|m| m.summary()).collect();
            Ok(json!({ "mails": shown, "matched": matched.len() }))
        })
    });

    registry.register_sync("mail.get", |params, _| {
        let id = str_of(&params, "id");
        let found = with_inbox(&workspace_of(&params), |inbox| inbox.iter().find(|m| m.id == id).cloned());
        let mail = found.ok_or_else(|| {
            EngineError::not_found(format!("No mail \"{id}\" (it may have been dropped: the sink keeps {MAX_MAILS})."))
        })?;
        Ok(mail.full(opt_str(&params, "linkMatch")))
    });

    registry.register("mail.wait", |params, _| async move {
        let filter = Filter { to: opt_str(&params, "to").map(str::to_string), ..Filter::default() };
        let found = wait_for(
            &workspace_of(&params),
            &filter,
            params["sinceTs"].as_i64().unwrap_or(0),
            params["timeoutMs"].as_u64().unwrap_or(0),
        )
        .await;
        Ok(json!({ "mail": found.map(|m| m.full(opt_str(&params, "linkMatch"))) }))
    });

    registry.register_sync("mail.clear", |params, _| {
        let ws_id = workspace_of(&params);
        let cleared = with_inbox(&ws_id, |inbox| {
            let cleared = std::mem::take(inbox).len();
            save(&ws_id, inbox);
            cleared
        });
        bus().emit("mail.changed", json!({ "workspaceId": ws_id, "cleared": cleared }));
        Ok(json!({ "cleared": cleared }))
    });
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, BufReader};

    use super::*;

    async fn free_port() -> u16 {
        tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
    }

    /// A sink on a free port; the mails of every test share the one inbox, so each test uses its own recipient.
    async fn sink(max: usize) -> (ListenerCore, u16) {
        let (core, port) = (ListenerCore::default(), free_port().await);
        core.start(port, handler(max));
        assert!(core.is_running());
        (core, port)
    }

    struct Client(BufReader<TcpStream>);

    impl Client {
        async fn open(port: u16) -> Self {
            let mut client = Self(BufReader::new(TcpStream::connect(("127.0.0.1", port)).await.unwrap()));
            assert!(client.reply().await.starts_with("220 "));
            client
        }

        /// One reply, its continuation lines joined with `|`.
        async fn reply(&mut self) -> String {
            let mut out = Vec::new();
            loop {
                let mut line = String::new();
                self.0.read_line(&mut line).await.unwrap();
                let line = line.trim_end().to_string();
                let last = line.as_bytes().get(3) != Some(&b'-');
                out.push(line);
                if last {
                    return out.join("|");
                }
            }
        }

        async fn send(&mut self, text: &str) -> String {
            self.0.get_mut().write_all(format!("{text}\r\n").as_bytes()).await.unwrap();
            self.reply().await
        }
    }

    #[tokio::test]
    async fn a_mail_sent_over_smtp_is_kept_with_its_parts() {
        let _root = crate::testing::scratch_data_root();
        let (_sink, port) = sink(MAX_MESSAGE).await;
        let mut smtp = Client::open(port).await;
        assert_eq!(smtp.send("EHLO test").await, format!("250-bifurc|250 SIZE {MAX_MESSAGE}"));
        assert_eq!(smtp.send("DATA").await, "503 need RCPT TO first");
        assert_eq!(smtp.send("RCPT TO:<x@y>").await.get(..3), Some("503"));
        assert_eq!(smtp.send("MAIL FROM:<app@shop.test> SIZE=100").await, "250 ok");
        assert_eq!(smtp.send("RCPT TO:<Dana@Smtp-One.test>").await, "250 ok");
        assert_eq!(smtp.send("NOOP").await, "250 ok");
        assert_eq!(smtp.send("BOGUS").await, "500 unknown command");
        assert_eq!(smtp.send("DATA").await.get(..3), Some("354"));
        for line in [
            "From: Shop <app@shop.test>",
            "To: dana@smtp-one.test, other@smtp-one.test",
            "Subject: =?utf-8?q?Your_code?=",
            "",
            "Your code is 482913",
            "..a line that starts with a dot",
        ] {
            smtp.0.get_mut().write_all(format!("{line}\r\n").as_bytes()).await.unwrap();
        }
        let done = smtp.send(".").await;
        assert!(done.starts_with("250 ok queued as "), "{done}");
        assert_eq!(smtp.send("RSET").await, "250 ok");
        assert_eq!(smtp.send("QUIT").await, "221 bye");

        let mail = newest("ws1", Some("dana@smtp-one.test"), 0).expect("kept");
        assert_eq!(done.trim_start_matches("250 ok queued as "), mail.id);
        assert_eq!((mail.from.as_str(), mail.subject.as_str()), ("app@shop.test", "Your code"));
        assert_eq!(mail.to, ["dana@smtp-one.test", "other@smtp-one.test"]);
        assert_eq!(mail.text, "Your code is 482913\n.a line that starts with a dot");
        assert_eq!(mail.full(None)["otp"], "482913");
        assert!(
            mail.addressed("SMTP-ONE") && mail.addressed("other@smtp-one.test") && !mail.addressed("dana@smtp-one")
        );
    }

    #[tokio::test]
    async fn a_message_over_the_limit_is_refused_and_not_kept() {
        let _root = crate::testing::scratch_data_root();
        let (_sink, port) = sink(60).await;
        let mut smtp = Client::open(port).await;
        assert!(smtp.send("MAIL FROM:<a@b> SIZE=61").await.starts_with("552"));
        assert_eq!(smtp.send("MAIL FROM:<a@b>").await, "250 ok");
        assert_eq!(smtp.send("RCPT TO:<big@smtp-two.test>").await, "250 ok");
        smtp.send("DATA").await;
        smtp.0
            .get_mut()
            .write_all(
                b"Subject: big\r\n\r\n0123456789012345678901234567890123456789012345678901234567890123456789\r\n",
            )
            .await
            .unwrap();
        assert_eq!(smtp.send(".").await, "552 message too large");
        assert_eq!(smtp.send("NOOP").await, "250 ok");
        assert!(newest("ws1", Some("big@smtp-two.test"), 0).is_none());
    }

    async fn get(port: u16, target: &str) -> (String, Value) {
        get_as(port, target, &format!("127.0.0.1:{port}")).await
    }

    async fn get_as(port: u16, target: &str, host: &str) -> (String, Value) {
        let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        socket.write_all(format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes()).await.unwrap();
        let mut text = String::new();
        socket.read_to_string(&mut text).await.unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        (head.lines().next().unwrap().to_string(), serde_json::from_str(body).unwrap())
    }

    #[tokio::test]
    async fn the_same_port_answers_http_with_the_code_and_waits_for_a_late_mail() {
        let _root = crate::testing::scratch_data_root();
        let (_sink, port) = sink(MAX_MESSAGE).await;
        receive(
            "a@b",
            &["http+one@smtp-three.test".into()],
            b"Subject: Hi\r\n\r\nUse 123456 or https://app.test/verify?x=1 now\r\n",
        );
        let (status, body) = get(port, "/mail/wait?to=http%2Bone@smtp-three.test&timeoutMs=100").await;
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!((&body["otp"], &body["link"]), (&json!("123456"), &json!("https://app.test/verify?x=1")));

        let (status, body) = get(port, "/mail/wait?to=nobody@smtp-three.test&timeoutMs=100").await;
        assert_eq!(status, "HTTP/1.1 408 Request Timeout");
        assert_eq!(body["error"], "no mail for nobody@smtp-three.test within 100ms");
        assert!(get(port, "/other").await.0.contains("404"));
        let target = "/mail/wait?to=http%2Bone@smtp-three.test&timeoutMs=100";
        assert!(get_as(port, target, "localhost").await.0.contains("200"));
        assert!(get_as(port, target, "[::1]:9").await.0.contains("200"));
        assert!(get_as(port, target, "rebound.example:1025").await.0.contains("403"));

        let later = tokio::spawn(async move { get(port, "/mail/wait?to=late@smtp-three.test&timeoutMs=5000").await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        receive("a@b", &["late@smtp-three.test".into()], b"Subject: Late 7777\r\n\r\nx");
        assert_eq!(later.await.unwrap().1["otp"], "7777");
    }

    #[test]
    fn a_received_mail_is_announced() {
        let _root = crate::testing::scratch_data_root();
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = seen.clone();
        let listener = bus().on("mail.changed", move |change| sink.lock().push(change.clone()));
        let id = receive("a@b", &["announce@smtp-three.test".into()], b"Subject: Hello\r\n\r\nx");
        let received = seen.lock().iter().find(|c| c["received"]["id"] == id.as_str()).cloned().unwrap();
        assert_eq!(received["received"]["subject"], "Hello");
        assert_eq!(received["received"]["to"], json!(["announce@smtp-three.test"]));
        assert_eq!(received["workspaceId"], "ws1");
        bus().off(listener);
    }

    #[test]
    fn the_oldest_mails_go_first_beyond_the_limit() {
        let mail = |n: usize| Mail {
            id: n.to_string(),
            timestamp: n as i64,
            from: String::new(),
            to: vec![],
            subject: String::new(),
            text: String::new(),
            html: String::new(),
        };
        let mut inbox = VecDeque::new();
        for n in 0..7 {
            keep(&mut inbox, mail(n), 5);
        }
        assert_eq!(inbox.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["2", "3", "4", "5", "6"]);
    }

    #[test]
    fn a_filter_picks_by_any_part_of_sender_subject_or_body_in_any_case() {
        let mail = Mail {
            id: "1".into(),
            timestamp: 0,
            from: "Shop <no-reply@shop.test>".into(),
            to: vec!["dana@x.test".into()],
            subject: "Your sign-in code".into(),
            text: String::new(),
            html: "<p>Code: 123456</p>".into(),
        };
        let filter = |from: &str, subject: &str, body: &str| Filter {
            to: Some("dana@x.test".into()),
            from: Some(from.into()),
            subject: Some(subject.into()),
            body: Some(body.into()),
        };
        assert!(filter("NO-REPLY@shop", "sign-in", "code:").matches(&mail));
        assert!(Filter::default().matches(&mail));
        assert!(!filter("billing@", "sign-in", "code:").matches(&mail));
        assert!(!filter("shop", "reset", "code:").matches(&mail));
        assert!(!filter("shop", "sign-in", "reset").matches(&mail));
        assert!(!Filter { to: Some("other@x.test".into()), ..Filter::default() }.matches(&mail));
    }

    #[test]
    fn a_mail_goes_to_the_active_workspace_and_outlives_the_engine() {
        let root = crate::testing::scratch_data_root();
        receive("a@b", &["kept@smtp-four.test".into()], b"Subject: Kept 4242

x");
        let mut settings = crate::workspace::settings::load_settings();
        settings.set("activeWorkspaceId", json!("ws2"));
        crate::workspace::settings::save_settings(&settings).unwrap();
        assert!(newest(&active_workspace(), Some("kept@smtp-four.test"), 0).is_none());
        forget_all();
        assert_eq!(newest("ws1", Some("kept@smtp-four.test"), 0).unwrap().full(None)["otp"], "4242");
        forget("ws1");
        assert!(!root.dir.path().join("mail").join("ws1.json").exists());
        assert!(newest("ws1", None, 0).is_none());
    }

    #[tokio::test]
    async fn a_port_in_use_leaves_the_sink_off_with_the_reason_in_the_status() {
        let _root = crate::testing::scratch_data_root();
        let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = taken.local_addr().unwrap().port();
        let sink = ListenerCore::default();
        sink.start(port, handler(MAX_MESSAGE));
        let status = status_of(&sink, "ws1");
        assert_eq!((&status["running"], &status["port"]), (&json!(false), &json!(port)));
        assert!(status["error"].as_str().unwrap().contains("already in use"), "{status}");
    }
}
