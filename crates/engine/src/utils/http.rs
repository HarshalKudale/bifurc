//! HTTP facts shared by the proxies and the request sender.

/// Headers that describe one connection and are never forwarded.
pub const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "proxy-connection",
];

/// Whether `name` (any case) is a hop-by-hop header.
pub fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP.contains(&name.to_ascii_lowercase().as_str())
}

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Duration,
};

use reqwest::{Client, redirect::Policy};
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// The options that need a different reqwest `Client`. Everything else about a request (timeout, headers)
/// is set on the request, so a handful of clients serve every send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientOptions {
    /// Check the server's certificate.
    pub verify_tls: bool,
    /// How many redirects are followed; 0 hands the 3xx back as the answer.
    pub max_redirects: usize,
    /// Keep idle connections for the next request to the same origin.
    pub pooled: bool,
    /// Ignore the system proxy settings.
    pub no_proxy: bool,
}

/// How many hops a followed redirect chain may take (reqwest's own default).
const MAX_REDIRECTS: usize = 10;

impl ClientOptions {
    /// A client for the requests the user sends: pooled, system proxy honoured.
    pub fn sending(verify_tls: bool, follow_redirects: bool) -> Self {
        Self {
            verify_tls,
            max_redirects: if follow_redirects { MAX_REDIRECTS } else { 0 },
            pooled: true,
            no_proxy: false,
        }
    }

    /// The proxies' upstream: it shows what servers send, so no redirects, no pooling, no system proxy and no
    /// certificate checks.
    pub const PROXY_UPSTREAM: Self = Self { verify_tls: false, max_redirects: 0, pooled: false, no_proxy: true };
}

static CLIENTS: LazyLock<Mutex<HashMap<ClientOptions, Client>>> = LazyLock::new(Default::default);

/// The client for `options`, built on first use. HTTP/1 only (the response handling expects it). A `Client`
/// is a handle to shared state, so the clone shares the pool.
pub fn client(options: ClientOptions) -> Result<Client, String> {
    let mut clients = CLIENTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(client) = clients.get(&options) {
        return Ok(client.clone());
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = Client::builder()
        .http1_only()
        .redirect(if options.max_redirects == 0 { Policy::none() } else { Policy::limited(options.max_redirects) })
        .danger_accept_invalid_certs(!options.verify_tls);
    if !options.pooled {
        builder = builder.pool_max_idle_per_host(0);
    }
    if options.no_proxy {
        builder = builder.no_proxy();
    }
    let built = builder.build().map_err(|err| err.to_string())?;
    clients.insert(options, built.clone());
    Ok(built)
}

/// The innermost message of an error chain, which is the one that names the cause.
pub fn root_message(err: &dyn std::error::Error) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(inner) = source {
        message = inner.to_string();
        source = inner.source();
    }
    message
}

pub const REFUSED_NO_ADDRESS: &str = "Nothing is listening at this address. Is the server running?";
pub const NAME_NOT_FOUND_NO_HOST: &str = "The host name could not be found.";

/// A failed send in plain words: nothing listening, name not found, connection closed early; otherwise the
/// innermost message of the chain. `host` and `port` name the target in the message when known.
pub fn describe_send_error(err: &(dyn std::error::Error + 'static), host: Option<&str>, port: Option<u16>) -> String {
    use std::io::ErrorKind;
    let mut message = err.to_string();
    let mut source = Some(err);
    while let Some(inner) = source {
        if let Some(io) = inner.downcast_ref::<std::io::Error>() {
            match (io.kind(), host, port) {
                (ErrorKind::ConnectionRefused, Some(host), Some(port)) => {
                    return format!("Nothing is listening at {host}:{port}. Is the server running?");
                }
                (ErrorKind::ConnectionRefused, ..) => return REFUSED_NO_ADDRESS.to_string(),
                (ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe, ..) => {
                    return "The connection was closed before a response came.".to_string();
                }
                _ => {}
            }
        }
        message = inner.to_string();
        if message.contains("dns error") {
            return match host {
                Some(host) => format!("The name {host} could not be found."),
                None => NAME_NOT_FOUND_NO_HOST.to_string(),
            };
        }
        if message.contains("connection closed before message completed") {
            return "The connection was closed before a response came.".to_string();
        }
        source = inner.source();
    }
    message
}

/// What a request does when nothing says otherwise: certificates are verified, redirects are handed back and
/// there is no time limit.
const DEFAULT_VERIFY_TLS: bool = true;
const DEFAULT_FOLLOW_REDIRECTS: bool = false;
const DEFAULT_TIMEOUT_MS: u64 = 0;

/// The settings one send runs with, after resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestSettings {
    pub verify_tls: bool,
    pub follow_redirects: bool,
    /// 0 is no limit.
    pub timeout_ms: u64,
}

impl Default for RequestSettings {
    fn default() -> Self {
        Self {
            verify_tls: DEFAULT_VERIFY_TLS,
            follow_redirects: DEFAULT_FOLLOW_REDIRECTS,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

impl RequestSettings {
    /// `request` settings (`{verifyTls?, followRedirects?, timeoutMs?}`) over the app's `requestDefaults`
    /// over the built-in defaults. A field that is missing or of the wrong type falls through.
    pub fn resolve(request: Option<&Value>) -> Self {
        let stored = crate::workspace::settings::load_settings();
        Self::resolve_with(request, stored.as_value().get("requestDefaults"))
    }

    pub fn resolve_with(request: Option<&Value>, defaults: Option<&Value>) -> Self {
        // The first layer holding the field with the right type wins.
        let layers = [request, defaults];
        let flag = |key: &str, built_in: bool| {
            layers.iter().flatten().find_map(|layer| layer.get(key).and_then(Value::as_bool)).unwrap_or(built_in)
        };
        Self {
            verify_tls: flag("verifyTls", DEFAULT_VERIFY_TLS),
            follow_redirects: flag("followRedirects", DEFAULT_FOLLOW_REDIRECTS),
            timeout_ms: layers
                .iter()
                .flatten()
                .find_map(|layer| layer.get("timeoutMs").and_then(Value::as_u64))
                .unwrap_or(DEFAULT_TIMEOUT_MS),
        }
    }

    /// The client these settings send with.
    pub fn client(&self) -> Result<Client, String> {
        client(ClientOptions::sending(self.verify_tls, self.follow_redirects))
    }

    pub fn deadline(&self) -> Deadline {
        Deadline::after_ms(self.timeout_ms)
    }
}

/// The wording of a send that ran out of time: "Timed out after 30 s".
pub fn timeout_message(timeout_ms: u64) -> String {
    if timeout_ms % 1000 == 0 {
        format!("Timed out after {} s", timeout_ms / 1000)
    } else {
        format!("Timed out after {timeout_ms} ms")
    }
}

/// Whether `message` is [`timeout_message`]'s.
pub fn is_timeout(message: &str) -> bool {
    message.starts_with("Timed out after ")
}

/// The message of a send that was stopped with `request.cancel`.
pub const CANCELLED: &str = "Cancelled";

/// A point in time a send must be done by, or no limit.
#[derive(Debug, Clone, Copy)]
pub struct Deadline(Option<(Instant, u64)>);

impl Deadline {
    pub fn after_ms(timeout_ms: u64) -> Self {
        Self((timeout_ms > 0).then(|| (Instant::now() + Duration::from_millis(timeout_ms), timeout_ms)))
    }

    /// `work`, or the timeout message if the deadline comes first.
    pub async fn run<T>(&self, work: impl Future<Output = T>) -> Result<T, String> {
        match self.0 {
            Some((at, ms)) => tokio::time::timeout_at(at, work).await.map_err(|_| timeout_message(ms)),
            None => Ok(work.await),
        }
    }
}

/// The sends that can be cancelled, by the `streamId` the caller gave them.
static IN_FLIGHT: LazyLock<Mutex<HashMap<String, CancellationToken>>> = LazyLock::new(Default::default);

/// A send that `request.cancel` can reach, for as long as this is held.
pub struct Cancellable {
    id: String,
    token: CancellationToken,
}

impl Cancellable {
    pub fn register(id: &str) -> Self {
        let token = CancellationToken::new();
        IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner()).insert(id.to_string(), token.clone());
        Self { id: id.to_string(), token }
    }

    /// `work`, or [`CANCELLED`] if the send is cancelled first (which drops `work` and with it the connection).
    pub async fn run<T>(&self, work: impl Future<Output = Result<T, String>>) -> Result<T, String> {
        tokio::select! {
            result = work => result,
            _ = self.token.cancelled() => Err(CANCELLED.to_string()),
        }
    }
}

impl Drop for Cancellable {
    fn drop(&mut self) {
        IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner()).remove(&self.id);
    }
}

/// Cancel the send registered as `id`; whether there was one.
pub fn cancel(id: &str) -> bool {
    match IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner()).remove(id) {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_client_is_built_once_per_set_of_options() {
        let options = ClientOptions::sending(true, true);
        client(options).unwrap();
        let count = CLIENTS.lock().unwrap().len();
        client(options).unwrap();
        assert_eq!(CLIENTS.lock().unwrap().len(), count);
        client(ClientOptions::PROXY_UPSTREAM).unwrap();
        assert!(CLIENTS.lock().unwrap().contains_key(&ClientOptions::PROXY_UPSTREAM));
    }

    #[test]
    fn options_follow_the_settings() {
        assert_eq!(ClientOptions::sending(false, false).max_redirects, 0);
        assert_eq!(ClientOptions::sending(false, true).max_redirects, MAX_REDIRECTS);
        assert!(ClientOptions::sending(true, false).pooled);
        assert!(!ClientOptions::PROXY_UPSTREAM.pooled && !ClientOptions::PROXY_UPSTREAM.verify_tls);
    }

    #[test]
    fn settings_resolve_request_then_defaults_then_built_in() {
        let built_in = RequestSettings { verify_tls: true, follow_redirects: false, timeout_ms: 0 };
        // A request stored before settings existed.
        assert_eq!(RequestSettings::resolve_with(None, None), built_in);
        assert_eq!(RequestSettings::resolve_with(Some(&json!({})), Some(&json!({}))), built_in);
        let defaults = json!({ "verifyTls": false, "followRedirects": true, "timeoutMs": 5000 });
        let from_defaults = RequestSettings::resolve_with(None, Some(&defaults));
        assert_eq!(from_defaults, RequestSettings { verify_tls: false, follow_redirects: true, timeout_ms: 5000 });
        let request = json!({ "verifyTls": true, "timeoutMs": null, "followRedirects": "yes" });
        let mixed = RequestSettings::resolve_with(Some(&request), Some(&defaults));
        assert_eq!(mixed, RequestSettings { verify_tls: true, follow_redirects: true, timeout_ms: 5000 });
    }

    #[test]
    fn send_failures_are_worded_plainly() {
        use std::io::{Error, ErrorKind};
        let refused = Error::from(ErrorKind::ConnectionRefused);
        assert_eq!(
            describe_send_error(&refused, Some("127.0.0.1"), Some(9000)),
            "Nothing is listening at 127.0.0.1:9000. Is the server running?"
        );
        assert_eq!(describe_send_error(&refused, None, None), REFUSED_NO_ADDRESS);
        let reset = Error::from(ErrorKind::ConnectionReset);
        assert_eq!(describe_send_error(&reset, None, None), "The connection was closed before a response came.");
        let dns = Error::other("dns error: failed to lookup address information");
        assert_eq!(
            describe_send_error(&dns, Some("api.example.test"), None),
            "The name api.example.test could not be found."
        );
    }

    #[test]
    fn the_timeout_is_worded_in_seconds_when_whole() {
        assert_eq!(timeout_message(30_000), "Timed out after 30 s");
        assert_eq!(timeout_message(1500), "Timed out after 1500 ms");
    }
}
