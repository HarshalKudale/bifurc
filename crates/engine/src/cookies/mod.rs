//! The cookie jar of the request tools: what a browser does with `Set-Cookie`, kept per workspace.
//!
//! A send command names its workspace with [`scoped`]; the HTTP functions under it call [`attach`] before a
//! request and [`collect`] on its response. Outside a scope both do nothing, so traffic that only passes
//! through the engine (the proxies, health checks) never meets the jar.
//!
//! A jar lives as long as the engine. A workspace whose `clearCookiesOnRestart` is off also has its jar
//! written beside `app.json` (never in the workspace's repository) and read back after a restart.

use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    path::PathBuf,
    sync::LazyLock,
};

use cookie_store::{CookieExpiration, CookieStore, RawCookie};
use http::{
    HeaderMap, HeaderValue,
    header::{COOKIE, SET_COOKIE},
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use url::Url;

use crate::{
    kernel::{CommandRegistry, bus::bus},
    utils::json::{opt_str, str_of},
    workspace::{
        paths::data_dir,
        settings::{load_settings, save_settings},
    },
};

tokio::task_local! {
    static WORKSPACE: String;
}

tokio::task_local! {
    static PRIVATE: String;
}

static JARS: LazyLock<Mutex<HashMap<String, CookieStore>>> = LazyLock::new(Default::default);

/// Run `work` with `ws_id`'s jar in reach of every request it sends on this task.
pub async fn scoped<F: Future>(ws_id: &str, work: F) -> F::Output {
    WORKSPACE.scope(ws_id.to_string(), work).await
}

/// Run `work` with jars of its own, named `owner`, in place of the workspaces': what one user among many at
/// once has. Such a jar starts empty, is never written or announced, and is gone when `work` is done.
pub async fn private<F: Future>(owner: &str, work: F) -> F::Output {
    let out = PRIVATE.scope(owner.to_string(), work).await;
    JARS.lock().retain(|key, _| key.split_once('#').is_none_or(|(_, of)| of != owner));
    out
}

/// The jar a request of `ws_id` sent from this task uses: the workspace's, or the task's own under
/// [`private`]. A task spawned to send takes it along, as it takes the workspace: `scoped(&jar_of(ws_id), …)`.
pub fn jar_of(ws_id: &str) -> String {
    match PRIVATE.try_with(String::clone) {
        Ok(owner) if !is_private(ws_id) => format!("{ws_id}#{owner}"),
        _ => ws_id.to_string(),
    }
}

fn is_private(jar: &str) -> bool {
    jar.contains('#')
}

fn jar_file(ws_id: &str) -> PathBuf {
    data_dir().join("cookies").join(format!("{ws_id}.json"))
}

fn clears_on_restart(ws_id: &str) -> bool {
    load_settings()
        .workspace(ws_id)
        .and_then(|ws| ws.get("clearCookiesOnRestart"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// The workspace's jar, read from its file the first time it is asked for when the workspace keeps cookies.
fn with_jar<T>(ws_id: &str, work: impl FnOnce(&mut CookieStore) -> T) -> T {
    let key = jar_of(ws_id);
    let mut jars = JARS.lock();
    let jar = jars.entry(key.clone()).or_insert_with(|| {
        if is_private(&key) || clears_on_restart(ws_id) {
            return CookieStore::default();
        }
        std::fs::File::open(jar_file(ws_id))
            .ok()
            .and_then(|file| cookie_store::serde::json::load_all(std::io::BufReader::new(file)).ok())
            .unwrap_or_default()
    });
    work(jar)
}

/// Write the jar when the workspace keeps cookies. Session cookies are kept too: the engine has no "browser
/// closed" moment other than the restart the user chose to keep them through.
fn save(ws_id: &str, jar: &CookieStore) {
    if is_private(&jar_of(ws_id)) || clears_on_restart(ws_id) {
        return;
    }
    let mut out = Vec::new();
    if cookie_store::serde::json::save_incl_expired_and_nonpersistent(jar, &mut out).is_ok() {
        let file = jar_file(ws_id);
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
        let _ = std::fs::write(file, out);
    }
}

fn changed(ws_id: &str) {
    if is_private(&jar_of(ws_id)) {
        return;
    }
    bus().emit("cookies.changed", json!({ "workspaceId": ws_id }));
}

/// The jar matches `http` and `https` addresses; a WebSocket handshake is an HTTP request.
fn request_url(url: &str) -> Option<Url> {
    let mut url = Url::parse(url).ok()?;
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => return Some(url),
    };
    url.set_scheme(scheme).ok()?;
    Some(url)
}

/// Add the jar's cookies for `url` to the request's `headers`. A cookie the request names itself wins.
pub fn attach(url: &str, headers: &mut HeaderMap) {
    let Ok(ws_id) = WORKSPACE.try_with(String::clone) else { return };
    let Some(url) = request_url(url) else { return };
    let own = headers.get(COOKIE).and_then(|value| value.to_str().ok()).unwrap_or("").trim().to_string();
    let own_names: Vec<&str> = own.split(';').filter_map(|pair| pair.split('=').next()).map(str::trim).collect();
    let mut pairs: Vec<String> = with_jar(&ws_id, |jar| {
        jar.get_request_values(&url)
            .filter(|(name, _)| !own_names.contains(name))
            .map(|(name, value)| format!("{name}={value}"))
            .collect()
    });
    if pairs.is_empty() {
        return;
    }
    pairs.sort();
    if !own.is_empty() {
        pairs.insert(0, own.clone());
    }
    if let Ok(value) = HeaderValue::from_str(&pairs.join("; ")) {
        headers.insert(COOKIE, value);
    }
}

/// Take the `Set-Cookie` headers of the response to `url` into the jar. One that is already expired removes
/// its cookie, which is how a server logs a client out.
pub fn collect(url: &str, headers: &HeaderMap) {
    if !headers.contains_key(SET_COOKIE) {
        return;
    }
    let Ok(ws_id) = WORKSPACE.try_with(String::clone) else { return };
    let Some(url) = request_url(url) else { return };
    with_jar(&ws_id, |jar| {
        for value in headers.get_all(SET_COOKIE) {
            // A cookie the jar refuses (another site's domain, a malformed one) is skipped, as a browser skips it.
            let _ = jar.parse(&String::from_utf8_lossy(value.as_bytes()), &url);
        }
        save(&ws_id, jar);
    });
    changed(&ws_id);
}

/// The jar's cookies for `url` by name: what a request's script reads as `bc.cookies`.
pub fn for_url(ws_id: &str, url: &str) -> Value {
    let Some(url) = request_url(url) else { return json!({}) };
    with_jar(ws_id, |jar| {
        Value::Object(jar.get_request_values(&url).map(|(name, value)| (name.to_string(), json!(value))).collect())
    })
}

/// Carry out what a script did through `bc.cookies`: a list of `{ op: set|unset|clear, name?, value?, url? }`,
/// each on the site of its own `url`, or else of `url`, the request's.
pub fn apply(ws_id: &str, url: &str, ops: &Value) {
    let Some(ops) = ops.as_array().filter(|ops| !ops.is_empty()) else { return };
    with_jar(ws_id, |jar| {
        for op in ops {
            let Some(url) = request_url(opt_str(op, "url").unwrap_or(url)) else { continue };
            let name = str_of(op, "name");
            match str_of(op, "op") {
                // `Path=/`, so the cookie reaches the whole site and not only the addresses beside the request.
                "set" => {
                    let _ = jar.insert_raw(&RawCookie::build((name, str_of(op, "value"))).path("/").build(), &url);
                }
                kind => {
                    let doomed: Vec<(String, String, String)> = jar
                        .matches(&url)
                        .into_iter()
                        .filter(|cookie| kind == "clear" || cookie.name() == name)
                        .map(|cookie| (domain_of(cookie), cookie.path.to_string(), cookie.name().to_string()))
                        .collect();
                    for (domain, path, name) in doomed {
                        jar.remove(&domain, &path, &name);
                    }
                }
            }
        }
        save(ws_id, jar);
    });
    changed(ws_id);
}

/// Drop a deleted workspace's jar and its file.
pub fn forget(ws_id: &str) {
    JARS.lock().remove(ws_id);
    let _ = std::fs::remove_file(jar_file(ws_id));
}

fn domain_of(cookie: &cookie_store::Cookie<'_>) -> String {
    cookie.domain.as_cow().map(|domain| domain.into_owned()).unwrap_or_default()
}

/// The live cookies grouped by domain, both in alphabetical order.
fn list(ws_id: &str) -> Value {
    let mut domains: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    with_jar(ws_id, |jar| {
        for cookie in jar.iter_unexpired() {
            let expires = match cookie.expires {
                CookieExpiration::AtUtc(at) => json!(at.unix_timestamp() * 1000),
                CookieExpiration::SessionEnd => Value::Null,
            };
            domains.entry(domain_of(cookie)).or_default().push(json!({
                "name": cookie.name(),
                "value": cookie.value(),
                "path": &*cookie.path,
                "expires": expires,
                "secure": cookie.secure().unwrap_or(false),
                "httpOnly": cookie.http_only().unwrap_or(false),
            }));
        }
    });
    let domains: Vec<Value> = domains
        .into_iter()
        .map(|(domain, mut cookies)| {
            cookies.sort_by(|a, b| str_of(a, "name").cmp(str_of(b, "name")));
            json!({ "domain": domain, "cookies": cookies })
        })
        .collect();
    json!({ "clearOnRestart": clears_on_restart(ws_id), "domains": domains })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("cookies.list", |params, _| Ok(list(str_of(&params, "workspaceId"))));

    // Without a `domain` the whole jar is emptied.
    registry.register_sync("cookies.clear", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        with_jar(ws_id, |jar| {
            match opt_str(&params, "domain") {
                Some(domain) => {
                    let doomed: Vec<(String, String)> = jar
                        .iter_any()
                        .filter(|cookie| domain_of(cookie) == domain)
                        .map(|cookie| (cookie.path.to_string(), cookie.name().to_string()))
                        .collect();
                    for (path, name) in doomed {
                        jar.remove(domain, &path, &name);
                    }
                }
                None => jar.clear(),
            }
            save(ws_id, jar);
        });
        changed(ws_id);
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("cookies.configure", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        let clear = params.get("clearOnRestart").and_then(Value::as_bool).unwrap_or(true);
        let mut settings = load_settings();
        if let Some(ws) = settings.workspace_mut(ws_id) {
            ws["clearCookiesOnRestart"] = json!(clear);
        }
        save_settings(&settings)?;
        if clear {
            let _ = std::fs::remove_file(jar_file(ws_id));
        } else {
            with_jar(ws_id, |jar| save(ws_id, jar));
        }
        changed(ws_id);
        Ok(json!({ "ok": true }))
    });
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use super::*;
    use crate::{kernel::CommandContext, rest::test_server, testing::scratch_data_root};

    async fn call(action: &str, payload: Value) -> Value {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        crate::rest::register(&mut registry);
        let registry = Arc::new(registry);
        registry.invoke(action, payload, CommandContext::internal(registry.clone())).await.unwrap()
    }

    fn response(set_cookies: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for cookie in set_cookies {
            headers.append(SET_COOKIE, HeaderValue::from_str(cookie).unwrap());
        }
        headers
    }

    fn sent(url: &str, own: Option<&str>) -> Option<String> {
        let mut headers = HeaderMap::new();
        if let Some(own) = own {
            headers.insert(COOKIE, HeaderValue::from_str(own).unwrap());
        }
        attach(url, &mut headers);
        headers.get(COOKIE).map(|value| value.to_str().unwrap().to_string())
    }

    #[tokio::test]
    async fn a_jar_follows_the_browser_rules_and_a_logout_empties_it() {
        let _root = scratch_data_root();
        forget("ws1");
        scoped("ws1", async {
            collect(
                "https://app.example.test/login",
                &response(&[
                    "sid=abc; Path=/; HttpOnly; Secure",
                    "theme=dark; Domain=example.test; Expires=Wed, 21 Oct 2099 07:28:00 GMT",
                    "stolen=1; Domain=other.test",
                ]),
            );
            assert_eq!(sent("https://app.example.test/api", None).as_deref(), Some("sid=abc; theme=dark"));
            // The handshake of a WebSocket to the same site carries them too.
            assert_eq!(sent("wss://app.example.test/live", None).as_deref(), Some("sid=abc; theme=dark"));
            // A host-only cookie stays on its host, a secure one off plain HTTP, and none reach another site.
            assert_eq!(sent("https://api.example.test/", None).as_deref(), Some("theme=dark"));
            assert_eq!(sent("http://app.example.test/", None).as_deref(), Some("theme=dark"));
            assert_eq!(sent("https://other.test/", None), None);
            // What the request says itself wins over the jar.
            assert_eq!(sent("https://app.example.test/", Some("sid=mine")).as_deref(), Some("sid=mine; theme=dark"));

            collect("https://app.example.test/logout", &response(&["sid=; Path=/; Max-Age=0"]));
            assert_eq!(sent("https://app.example.test/api", None).as_deref(), Some("theme=dark"));
        })
        .await;
        // Another workspace has its own jar, and outside a scope there is none.
        assert_eq!(scoped("ws2", async { sent("https://app.example.test/", None) }).await, None);
        assert_eq!(sent("https://app.example.test/", None), None);
    }

    #[tokio::test]
    async fn a_sent_request_stores_cookies_and_the_next_one_carries_them() {
        let _root = scratch_data_root();
        forget("ws1");
        let base = test_server::start(HashMap::new()).await;
        let send = |path: &str| {
            let request = json!({ "method": "GET", "url": format!("{base}{path}"), "headers": {}, "body": "" });
            call("request.send", json!({ "workspaceId": "ws1", "request": request, "scripts": {} }))
        };
        send("/set-cookie/sid=s1/csrf=c1").await;
        let echoed = send("/me").await;
        let body = crate::utils::replay::decode_base64_lenient(echoed["body"].as_str().unwrap());
        let echoed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(echoed["headers"]["cookie"], "csrf=c1; sid=s1");

        let listed = call("cookies.list", json!({ "workspaceId": "ws1" })).await;
        assert_eq!(listed["clearOnRestart"], true);
        assert_eq!(listed["domains"][0]["domain"], "127.0.0.1");
        assert_eq!(listed["domains"][0]["cookies"][1]["name"], "sid");
        assert_eq!(listed["domains"][0]["cookies"][1]["expires"], Value::Null);

        call("cookies.clear", json!({ "workspaceId": "ws1", "domain": "127.0.0.1" })).await;
        assert_eq!(call("cookies.list", json!({ "workspaceId": "ws1" })).await["domains"], json!([]));
    }

    #[tokio::test]
    async fn a_script_puts_a_token_of_the_body_in_the_jar_and_takes_it_out() {
        let _root = scratch_data_root();
        forget("ws1");
        let canned =
            HashMap::from([("/login".to_string(), ("application/json".to_string(), r#"{"token":"t-9"}"#.to_string()))]);
        let base = test_server::start(canned).await;
        let send = |path: &str, scripts: Value| {
            let request = json!({ "method": "GET", "url": format!("{base}{path}"), "headers": {}, "body": "" });
            call("request.send", json!({ "workspaceId": "ws1", "request": request, "scripts": scripts }))
        };
        let cookie_sent = |answer: Value| {
            let body = crate::utils::replay::decode_base64_lenient(answer["body"].as_str().unwrap());
            serde_json::from_slice::<Value>(&body).unwrap()["headers"]["cookie"].clone()
        };

        let script =
            "bc.cookies.set('token', bc.response.json().token); bc.cookies.set('far', 1, 'http://other.test/');";
        send("/login", json!({ "post": script })).await;
        // A request deep in the site carries it, and its own scripts read it.
        let answer = send(
            "/api/v1/me",
            json!({ "test": "test('seen', () => expect(bc.cookies.get('token')).to.equal('t-9'));" }),
        )
        .await;
        assert_eq!(answer["tests"][0]["passed"], true, "{answer}");
        assert_eq!(cookie_sent(answer), "token=t-9");
        assert_eq!(for_url("ws1", "http://other.test/x"), json!({ "far": "1" }));

        // A pre-script's change is in time for its own request.
        let answer = send(
            "/me",
            json!({ "pre": "bc.cookies.unset('token'); bc.cookies.set('fresh', bc.cookies.has('token'));" }),
        )
        .await;
        assert_eq!(cookie_sent(answer), "fresh=false");
        send("/me", json!({ "post": "bc.cookies.clear();" })).await;
        assert_eq!(for_url("ws1", &base), json!({}));
        assert_eq!(for_url("ws1", "http://other.test/"), json!({ "far": "1" }));
    }

    #[tokio::test]
    async fn cookies_outlive_the_engine_only_when_the_workspace_keeps_them() {
        let _root = scratch_data_root();
        forget("ws1");
        let login = || scoped("ws1", async { collect("http://h.test/", &response(&["sid=abc"])) });
        let after_restart = || {
            JARS.lock().clear();
            scoped("ws1", async { sent("http://h.test/", None) })
        };

        login().await;
        assert!(!jar_file("ws1").exists());
        assert_eq!(after_restart().await, None);

        call("cookies.configure", json!({ "workspaceId": "ws1", "clearOnRestart": false })).await;
        login().await;
        assert_eq!(after_restart().await.as_deref(), Some("sid=abc"));

        call("cookies.configure", json!({ "workspaceId": "ws1", "clearOnRestart": true })).await;
        assert!(!jar_file("ws1").exists());
        assert_eq!(after_restart().await, None);
    }
}
