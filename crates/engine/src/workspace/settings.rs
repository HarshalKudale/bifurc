//! `app.json`: the app-level settings — ports, TLS, and the list of workspaces.
//!
//! Kept as a JSON object rather than a struct: the file is merged over the defaults key by key, so a field
//! this engine does not know survives a load and save.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use super::paths::data_dir;
use crate::utils::json::{read_json, write_json_pretty};

pub const DEFAULT_REVERSE_PROXY_PORT: u16 = 80;
pub const DEFAULT_FORWARD_PROXY_PORT: u16 = 8080;
pub const DEFAULT_COMPANION_PORT: u16 = 4747;
/// How many unpinned captured exchanges a workspace keeps, unless `captureLimit` says otherwise.
pub const DEFAULT_CAPTURE_LIMIT: usize = 200;
/// The header that carries the capture id to the upstream, unless `traceHeader` says otherwise.
pub const DEFAULT_TRACE_HEADER: &str = "x-bifurc-trace";
/// Where the terms of service a first run asks the user to accept are published.
pub const TERMS_OF_SERVICE_URL: &str = "https://bifurc.app/tos";
/// Where the privacy policy a first run asks the user to accept is published.
pub const PRIVACY_POLICY_URL: &str = "https://bifurc.app/privacy";

/// The settings object. Read fields through the accessors; mutate through [`AppSettings::set`] or
/// [`AppSettings::as_value_mut`].
#[derive(Debug, Clone, PartialEq)]
pub struct AppSettings(pub Value);

impl AppSettings {
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub fn as_value_mut(&mut self) -> &mut Value {
        &mut self.0
    }

    pub fn set(&mut self, key: &str, value: Value) {
        if let Some(object) = self.0.as_object_mut() {
            object.insert(key.to_string(), value);
        }
    }

    fn port_field(&self, key: &str, default: u16) -> u16 {
        self.0.get(key).and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).unwrap_or(default)
    }

    /// The reverse proxy (edge) port: what `*.localhost` mappings are served on, over HTTPS while HTTPS is on.
    pub fn port(&self) -> u16 {
        self.port_field("port", DEFAULT_REVERSE_PROXY_PORT)
    }

    pub fn forward_proxy_port(&self) -> u16 {
        self.port_field("forwardProxyPort", DEFAULT_FORWARD_PROXY_PORT)
    }

    pub fn companion_port(&self) -> u16 {
        self.port_field("companionPort", DEFAULT_COMPANION_PORT)
    }

    /// How many unpinned captured exchanges a workspace keeps (10 to 10000; anything else is the default).
    pub fn capture_limit(&self) -> usize {
        let limit = self.0.get("captureLimit").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
        limit.filter(|n| (10..=10_000).contains(n)).unwrap_or(DEFAULT_CAPTURE_LIMIT)
    }

    /// The header the proxies stamp on forwarded requests with the capture id (`traceHeader`; empty or not a
    /// header name = off, unset = [`DEFAULT_TRACE_HEADER`]).
    pub fn trace_header(&self) -> Option<String> {
        let raw = self.0.get("traceHeader").and_then(Value::as_str).unwrap_or(DEFAULT_TRACE_HEADER);
        let name = raw.trim().to_lowercase();
        let valid = name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        (!name.is_empty() && valid).then_some(name)
    }

    pub fn tls_enabled(&self) -> bool {
        self.0.get("tlsEnabled").and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn tls_ca_cert_path(&self) -> Option<String> {
        self.0.get("tlsCaCertPath").and_then(Value::as_str).map(str::to_string)
    }

    pub fn tls_ca_key_path(&self) -> Option<String> {
        self.0.get("tlsCaKeyPath").and_then(Value::as_str).map(str::to_string)
    }

    /// `false` only when the user turned anonymous usage analytics off.
    pub fn telemetry_enabled(&self) -> bool {
        self.0.get("telemetryEnabled").and_then(Value::as_bool).unwrap_or(true)
    }

    /// `false` only when the user turned the daily look for a newer version off.
    pub fn update_check_enabled(&self) -> bool {
        self.0.get("updateCheckEnabled").and_then(Value::as_bool).unwrap_or(true)
    }

    /// The terms of service were accepted on this data directory.
    pub fn terms_accepted(&self) -> bool {
        self.0.get("termsAcceptedAt").is_some_and(|at| !at.is_null())
    }

    /// The privacy policy was accepted on this data directory.
    pub fn privacy_accepted(&self) -> bool {
        self.0.get("privacyAcceptedAt").is_some_and(|at| !at.is_null())
    }

    /// The first-run setup was walked through to its end.
    pub fn onboarded(&self) -> bool {
        self.0.get("hasSeenWelcome").and_then(Value::as_bool).unwrap_or(false)
    }

    /// The two proxies may be started without being asked: the first run's servers step was passed, so their
    /// ports are ones the user chose or left as they are. An install that finished the first run before that
    /// step was recorded counts too.
    pub fn servers_confirmed(&self) -> bool {
        self.onboarded() || self.0.get("serversConfirmedAt").is_some_and(|at| !at.is_null())
    }

    pub fn active_workspace_id(&self) -> String {
        self.0.get("activeWorkspaceId").and_then(Value::as_str).unwrap_or("").to_string()
    }

    /// The workspace metas: `{id, name, activeEnvironmentId, accountId?, syncConfig?, syncMeta?}`.
    pub fn workspaces(&self) -> &[Value] {
        self.0.get("workspaces").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn workspaces_mut(&mut self) -> &mut Vec<Value> {
        crate::utils::json::array_mut(&mut self.0, "workspaces")
    }

    pub fn workspace(&self, ws_id: &str) -> Option<&Value> {
        self.workspaces().iter().find(|w| w.get("id").and_then(Value::as_str) == Some(ws_id))
    }

    pub fn workspace_mut(&mut self, ws_id: &str) -> Option<&mut Value> {
        self.workspaces_mut().iter_mut().find(|w| w.get("id").and_then(Value::as_str) == Some(ws_id))
    }

    /// The stored environment pointer of a workspace (not validated; see `env::effective_environment_id`).
    pub fn stored_environment_id(&self, ws_id: &str) -> Option<String> {
        self.workspace(ws_id)?.get("activeEnvironmentId")?.as_str().map(str::to_string)
    }
}

/// `BIFURC_PROXY_BASE_PORT` sets the forward proxy's port outright. An unusable value is ignored with a
/// warning rather than taking the engine down.
pub fn forward_proxy_port_from_env() -> Option<u16> {
    let raw = std::env::var("BIFURC_PROXY_BASE_PORT").ok().filter(|v| !v.is_empty())?;
    match raw.trim().parse::<u16>() {
        Ok(port) => Some(port),
        Err(_) => {
            eprintln!("[settings] BIFURC_PROXY_BASE_PORT must be a port number (0-65535), got {raw:?}; ignoring it");
            None
        }
    }
}

/// A new data directory holds no workspace: the first one is made by whoever sets the app up (new, or
/// imported from a remote), so that its first commit is theirs.
fn make_default_settings() -> Value {
    json!({
        "port": DEFAULT_REVERSE_PROXY_PORT,
        "forwardProxyPort": DEFAULT_FORWARD_PROXY_PORT,
        "companionPort": DEFAULT_COMPANION_PORT,
        "minimizeToTray": true,
        "tlsEnabled": false,
        "tlsCaCertPath": null,
        "tlsCaKeyPath": null,
        "tlsTrustedCa": null,
        "workspaces": [],
        "activeWorkspaceId": "",
        "hasSeenWelcome": false,
        "termsAcceptedAt": null,
        "privacyAcceptedAt": null,
        "serversConfirmedAt": null,
        "zoomLevel": 0,
        "zoomLevelSetByUser": false,
        "themeId": null,
    })
}

pub fn settings_path() -> PathBuf {
    data_dir().join("app.json")
}

/// The directory `app.json` is in; the interception CA is kept beside it.
pub fn app_data_dir() -> PathBuf {
    data_dir()
}

fn apply_env_overrides(mut settings: Value) -> AppSettings {
    if let Some(port) = forward_proxy_port_from_env() {
        settings["forwardProxyPort"] = json!(port);
    }
    AppSettings(settings)
}

/// Read `app.json` over the defaults. A missing file yields fresh defaults, which are saved.
pub fn load_settings() -> AppSettings {
    match read_json(&settings_path()).filter(Value::is_object) {
        Some(parsed) => {
            let mut merged: Map<String, Value> = make_default_settings().as_object().cloned().unwrap_or_default();
            for (key, value) in parsed.as_object().into_iter().flatten() {
                merged.insert(key.clone(), value.clone());
            }
            apply_env_overrides(Value::Object(merged))
        }
        None => {
            let fresh = apply_env_overrides(make_default_settings());
            // An unreadable file is left alone: overwriting it would lose the workspace list for good.
            if !settings_path().exists() {
                let _ = save_settings(&fresh);
            }
            fresh
        }
    }
}

pub fn save_settings(settings: &AppSettings) -> std::io::Result<()> {
    write_json_pretty(&settings_path(), &settings.0)
}
