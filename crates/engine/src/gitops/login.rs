//! Signing in to GitHub from a device: the engine asks for a code, the user types it on GitHub's page, and
//! the engine waits for the answer in the background. Clients watch a sign-in through `account.login.status`
//! or the `account.changed` event; the token never leaves the engine.

use std::{collections::HashMap, time::Duration};

use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use tokio::task::JoinHandle;

use super::github::{self, DevicePoll};
use crate::{
    kernel::bus,
    protocol::EngineError,
    utils::id::{generate_id, now_ms},
    workspace::accounts::{Account, account_id, save_account},
};

struct Login {
    /// `pending` | `done` | `failed` | `denied` | `expired` | `cancelled`
    status: &'static str,
    user_code: String,
    verification_uri: String,
    expires_at: i64,
    error: Option<EngineError>,
    account: Option<Value>,
    task: Option<JoinHandle<()>>,
}

impl Login {
    fn to_value(&self, login_id: &str) -> Value {
        let mut out = Map::new();
        out.insert("loginId".into(), json!(login_id));
        out.insert("status".into(), json!(self.status));
        out.insert("userCode".into(), json!(self.user_code));
        out.insert("verificationUri".into(), json!(self.verification_uri));
        out.insert("expiresAt".into(), json!(self.expires_at));
        if let Some(error) = &self.error {
            crate::kernel::streams::put_error(&mut out, Some(error));
        }
        if let Some(account) = &self.account {
            out.insert("account".into(), account.clone());
        }
        Value::Object(out)
    }
}

static LOGINS: Mutex<Option<HashMap<String, Login>>> = Mutex::new(None);

fn finish(login_id: &str, status: &'static str, error: Option<EngineError>, account: Option<Value>) {
    let payload = {
        let mut logins = LOGINS.lock();
        let Some(login) = logins.as_mut().and_then(|logins| logins.get_mut(login_id)) else { return };
        if login.status != "pending" {
            return;
        }
        login.status = status;
        login.error = error;
        login.account = account;
        login.task = None;
        login.to_value(login_id)
    };
    bus().emit("account.changed", payload);
}

/// Exchange the granted token for the person it belongs to and store both.
async fn complete(token: String) -> Result<Value, EngineError> {
    let user = github::fetch_user(&token).await.map_err(github::failure)?;
    let host = github::host();
    let account = Account {
        id: account_id(&host, user.id),
        host,
        user_id: user.id,
        login: user.login,
        name: user.name,
        avatar_url: user.avatar_url,
        token: Some(token),
    };
    save_account(&account).map_err(|e| {
        EngineError::engine(format!("The sign-in could not be saved: {e}"))
            .keyed("github.sign_in_not_saved", json!({ "detail": e.to_string() }))
    })?;
    Ok(account.public())
}

async fn wait_for_grant(login_id: String, device_code: String, mut interval: u64, expires_at: i64) {
    loop {
        tokio::time::sleep(Duration::from_secs(interval.max(1))).await;
        if now_ms() >= expires_at {
            return finish(&login_id, "expired", Some(code_expired()), None);
        }
        match github::poll_device_flow(&device_code).await {
            Ok(DevicePoll::Pending) => {}
            Ok(DevicePoll::SlowDown(seconds)) => interval = seconds,
            Ok(DevicePoll::Token(token)) => {
                return match complete(token).await {
                    Ok(account) => finish(&login_id, "done", None, Some(account)),
                    Err(message) => finish(&login_id, "failed", Some(message), None),
                };
            }
            Ok(DevicePoll::Denied) => {
                let declined = EngineError::engine("The sign-in was declined on GitHub.")
                    .keyed("github.sign_in_declined", json!({}));
                return finish(&login_id, "denied", Some(declined), None);
            }
            Ok(DevicePoll::Expired) => {
                return finish(&login_id, "expired", Some(code_expired()), None);
            }
            // One failed poll (the network dropped) is not the end of the sign-in; the code's expiry is.
            Err(_) => {}
        }
    }
}

fn code_expired() -> EngineError {
    EngineError::engine("The code expired before it was used.").keyed("github.code_expired", json!({}))
}

/// Begin a sign-in. Returns what the user needs: `{ loginId, status, userCode, verificationUri, expiresAt }`.
pub async fn start_login() -> Result<Value, EngineError> {
    let code = github::start_device_flow().await.map_err(github::failure)?;
    let login_id = generate_id();
    let expires_at = now_ms() + (code.expires_in as i64) * 1000;
    let task = tokio::spawn(wait_for_grant(login_id.clone(), code.device_code, code.interval, expires_at));
    let login = Login {
        status: "pending",
        user_code: code.user_code,
        verification_uri: code.verification_uri,
        expires_at,
        error: None,
        account: None,
        task: Some(task),
    };
    let value = login.to_value(&login_id);
    LOGINS.lock().get_or_insert_with(HashMap::new).insert(login_id, login);
    Ok(value)
}

/// Where a sign-in stands, or `{ status: "unknown" }`.
pub fn login_status(login_id: &str) -> Value {
    LOGINS
        .lock()
        .as_ref()
        .and_then(|logins| logins.get(login_id))
        .map_or_else(|| json!({ "loginId": login_id, "status": "unknown" }), |login| login.to_value(login_id))
}

/// Stop waiting for a sign-in the user gave up on.
pub fn cancel_login(login_id: &str) {
    let task = LOGINS.lock().as_mut().and_then(|logins| logins.get_mut(login_id)).and_then(|login| login.task.take());
    if let Some(task) = task {
        task.abort();
    }
    finish(login_id, "cancelled", None, None);
}
