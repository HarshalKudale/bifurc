//! The GitHub calls the engine makes: the device sign-in, who a token belongs to, and the repositories that
//! person owns.
//!
//! Sign-in is the OAuth device flow: it needs a client id and no secret, so it works from a desktop app. The
//! id comes from `BIFURC_GITHUB_CLIENT_ID` (when the engine runs, else when it was built) or from
//! `githubClientId` in `app.json`.

use std::time::Duration;

use serde_json::{Value, json};

use crate::{
    utils::json::{array_of, bool_of, str_of},
    workspace::settings::load_settings,
};

const TIMEOUT: Duration = Duration::from_secs(20);
const USER_AGENT: &str = concat!("bifurc-engine/", env!("CARGO_PKG_VERSION"));
/// What marks a repository as a Bifurc workspace in a listing.
pub const WORKSPACE_TOPIC: &str = "bifurc-workspace";
/// `repo` is what pushing to and creating private repositories needs.
const SCOPE: &str = "repo";

#[derive(Debug, Clone)]
pub struct Endpoints {
    /// Where sign-in happens: `https://github.com`.
    pub web: String,
    /// `https://api.github.com`.
    pub api: String,
}

#[cfg(test)]
static TEST_ENDPOINTS: parking_lot::Mutex<Option<Endpoints>> = parking_lot::Mutex::new(None);

/// Point the engine at a stand-in for GitHub. Tests take turns through the scratch data root.
#[cfg(test)]
pub fn set_test_endpoints(endpoints: Option<Endpoints>) {
    *TEST_ENDPOINTS.lock() = endpoints;
}

pub fn endpoints() -> Endpoints {
    #[cfg(test)]
    if let Some(endpoints) = TEST_ENDPOINTS.lock().clone() {
        return endpoints;
    }
    let from_env = |name: &str, default: &str| {
        std::env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
    };
    Endpoints {
        web: from_env("BIFURC_GITHUB_URL", "https://github.com").trim_end_matches('/').to_string(),
        api: from_env("BIFURC_GITHUB_API_URL", "https://api.github.com").trim_end_matches('/').to_string(),
    }
}

/// The host accounts are filed under: `github.com`.
pub fn host() -> String {
    let web = endpoints().web;
    url::Url::parse(&web).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or(web)
}

/// The OAuth app sign-in runs as, if one is configured.
pub fn client_id() -> Option<String> {
    #[cfg(test)]
    if TEST_ENDPOINTS.lock().is_some() {
        return Some("test-client".to_string());
    }
    std::env::var("BIFURC_GITHUB_CLIENT_ID")
        .ok()
        .or_else(|| load_settings().as_value().get("githubClientId").and_then(Value::as_str).map(str::to_string))
        .or_else(|| option_env!("BIFURC_GITHUB_CLIENT_ID").map(str::to_string))
        .filter(|id| !id.trim().is_empty())
}

fn client() -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder().timeout(TIMEOUT).user_agent(USER_AGENT).build().map_err(|e| e.to_string())
}

/// A response's status and JSON body (`null` when it has none).
async fn read(response: reqwest::Response) -> Result<(u16, Value), String> {
    let status = response.status().as_u16();
    let text = response.text().await.map_err(|e| e.to_string())?;
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn unreachable(error: reqwest::Error) -> String {
    format!("GitHub could not be reached: {error}")
}

async fn api(method: reqwest::Method, token: &str, path: &str, body: Option<Value>) -> Result<(u16, Value), String> {
    let mut request = client()?
        .request(method, format!("{}{path}", endpoints().api))
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.header("content-type", "application/json").body(body.to_string());
    }
    read(request.send().await.map_err(unreachable)?).await
}

/// What an answer of 401 is reported as.
const REJECTED: &str = "GitHub no longer accepts this sign-in. Sign in again.";

/// Whether an error says GitHub refused the token itself (revoked, expired), as opposed to anything else.
pub fn is_rejected(error: &str) -> bool {
    error == REJECTED
}

fn api_error(status: u16, body: &Value) -> String {
    match status {
        401 => REJECTED.to_string(),
        _ => match str_of(body, "message") {
            "" => format!("GitHub answered {status}"),
            message => format!("GitHub: {message}"),
        },
    }
}

/// What the user is shown to sign in: a code, and the page to type it on.
#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

pub async fn start_device_flow() -> Result<DeviceCode, String> {
    let client_id = client_id().ok_or("GitHub sign-in is not set up in this build (no OAuth client id).")?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &client_id)
        .append_pair("scope", SCOPE)
        .finish();
    let response = client()?
        .post(format!("{}/login/device/code", endpoints().web))
        .header("accept", "application/json")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(unreachable)?;
    let (status, body) = read(response).await?;
    let device_code = str_of(&body, "device_code");
    if status != 200 || device_code.is_empty() {
        let reason = [str_of(&body, "error_description"), str_of(&body, "error")]
            .into_iter()
            .find(|text| !text.is_empty())
            .map_or_else(|| format!("GitHub answered {status}"), str::to_string);
        return Err(format!("GitHub refused to start a sign-in: {reason}"));
    }
    Ok(DeviceCode {
        device_code: device_code.to_string(),
        user_code: str_of(&body, "user_code").to_string(),
        verification_uri: str_of(&body, "verification_uri").to_string(),
        expires_in: body.get("expires_in").and_then(Value::as_u64).unwrap_or(900),
        interval: body.get("interval").and_then(Value::as_u64).unwrap_or(5),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevicePoll {
    Pending,
    /// Ask less often: wait this many seconds between tries from now on.
    SlowDown(u64),
    Token(String),
    Denied,
    Expired,
}

pub async fn poll_device_flow(device_code: &str) -> Result<DevicePoll, String> {
    let client_id = client_id().ok_or("GitHub sign-in is not set up in this build (no OAuth client id).")?;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &client_id)
        .append_pair("device_code", device_code)
        .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:device_code")
        .finish();
    let response = client()?
        .post(format!("{}/login/oauth/access_token", endpoints().web))
        .header("accept", "application/json")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(unreachable)?;
    let (_, body) = read(response).await?;
    if let Some(token) = body.get("access_token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        return Ok(DevicePoll::Token(token.to_string()));
    }
    match str_of(&body, "error") {
        "authorization_pending" => Ok(DevicePoll::Pending),
        "slow_down" => Ok(DevicePoll::SlowDown(body.get("interval").and_then(Value::as_u64).unwrap_or(10))),
        "access_denied" => Ok(DevicePoll::Denied),
        "expired_token" => Ok(DevicePoll::Expired),
        other => Err(match str_of(&body, "error_description") {
            "" => format!("GitHub refused the sign-in: {other}"),
            description => format!("GitHub refused the sign-in: {description}"),
        }),
    }
}

#[derive(Debug, Clone)]
pub struct GithubUser {
    pub id: u64,
    pub login: String,
    pub name: String,
    pub avatar_url: String,
}

pub async fn fetch_user(token: &str) -> Result<GithubUser, String> {
    let (status, body) = api(reqwest::Method::GET, token, "/user", None).await?;
    let id = body.get("id").and_then(Value::as_u64);
    match (status, id) {
        (200, Some(id)) => Ok(GithubUser {
            id,
            login: str_of(&body, "login").to_string(),
            name: str_of(&body, "name").to_string(),
            avatar_url: str_of(&body, "avatar_url").to_string(),
        }),
        _ => Err(api_error(status, &body)),
    }
}

/// A repository as the engine uses it.
#[derive(Debug, Clone)]
pub struct Repo {
    pub full_name: String,
    pub name: String,
    pub owner_id: u64,
    pub owner_login: String,
    pub private: bool,
    pub description: String,
    pub clone_url: String,
    pub html_url: String,
    pub default_branch: String,
    pub updated_at: String,
    pub is_workspace: bool,
}

impl Repo {
    fn from_value(value: &Value) -> Option<Repo> {
        let full_name = str_of(value, "full_name");
        if full_name.is_empty() {
            return None;
        }
        Some(Repo {
            full_name: full_name.to_string(),
            name: str_of(value, "name").to_string(),
            owner_id: value.pointer("/owner/id").and_then(Value::as_u64)?,
            owner_login: value.pointer("/owner/login").and_then(Value::as_str).unwrap_or_default().to_string(),
            private: bool_of(value, "private", false),
            description: str_of(value, "description").to_string(),
            clone_url: str_of(value, "clone_url").to_string(),
            html_url: str_of(value, "html_url").to_string(),
            default_branch: match str_of(value, "default_branch") {
                "" => "main".to_string(),
                branch => branch.to_string(),
            },
            updated_at: str_of(value, "updated_at").to_string(),
            is_workspace: array_of(value, "topics").iter().any(|topic| topic.as_str() == Some(WORKSPACE_TOPIC)),
        })
    }

    pub fn public(&self) -> Value {
        json!({
            "fullName": self.full_name, "name": self.name, "private": self.private,
            "description": self.description, "url": self.html_url, "updatedAt": self.updated_at,
            "isWorkspace": self.is_workspace,
        })
    }
}

/// The repositories the signed-in person owns themselves, most recently updated first. Ones they were only
/// given access to are not listed: a workspace is used by its owner alone.
pub async fn list_owned_repos(token: &str) -> Result<Vec<Repo>, String> {
    const PER_PAGE: usize = 100;
    const MAX_PAGES: usize = 10;
    let mut repos = Vec::new();
    for page in 1..=MAX_PAGES {
        let path = format!("/user/repos?affiliation=owner&sort=updated&per_page={PER_PAGE}&page={page}");
        let (status, body) = api(reqwest::Method::GET, token, &path, None).await?;
        let Some(items) = body.as_array().filter(|_| status == 200) else { return Err(api_error(status, &body)) };
        repos.extend(items.iter().filter_map(Repo::from_value));
        if items.len() < PER_PAGE {
            break;
        }
    }
    Ok(repos)
}

/// One repository by `owner/name`.
pub async fn get_repo(token: &str, full_name: &str) -> Result<Repo, String> {
    let (status, body) = api(reqwest::Method::GET, token, &format!("/repos/{full_name}"), None).await?;
    match (status, Repo::from_value(&body)) {
        (200, Some(repo)) => Ok(repo),
        (404, _) => Err(format!("There is no repository {full_name} this account can see.")),
        _ => Err(api_error(status, &body)),
    }
}

/// A JSON file at the root of a repository's default branch, read without cloning it. `None` when the
/// repository has no such file (or is empty), or the file is not JSON.
pub async fn read_json_file(token: &str, full_name: &str, path: &str) -> Result<Option<Value>, String> {
    let response = client()?
        .get(format!("{}/repos/{full_name}/contents/{path}", endpoints().api))
        .header("accept", "application/vnd.github.raw+json")
        .header("x-github-api-version", "2022-11-28")
        .bearer_auth(token)
        .send()
        .await
        .map_err(unreachable)?;
    let (status, body) = read(response).await?;
    match status {
        200 => Ok(Some(body).filter(|body| !body.is_null())),
        404 => Ok(None),
        _ => Err(api_error(status, &body)),
    }
}

/// Create an empty repository on the signed-in person's own account. A name that is taken is an error: a
/// workspace is only ever pushed to a repository made for it.
pub async fn create_repo(token: &str, name: &str, private: bool, description: &str) -> Result<Repo, String> {
    let body = json!({ "name": name, "private": private, "description": description, "auto_init": false });
    let (status, body) = api(reqwest::Method::POST, token, "/user/repos", Some(body)).await?;
    match (status, Repo::from_value(&body)) {
        (201, Some(repo)) => Ok(repo),
        (422, _) => Err(format!(
            "A repository named \"{name}\" already exists on this account, or the name is not allowed. Choose another name."
        )),
        _ => Err(api_error(status, &body)),
    }
}

/// Mark a repository as a Bifurc workspace, so listings can say which ones are. Best effort.
pub async fn mark_as_workspace(token: &str, full_name: &str) {
    let body = json!({ "names": [WORKSPACE_TOPIC] });
    let _ = api(reqwest::Method::PUT, token, &format!("/repos/{full_name}/topics"), Some(body)).await;
}
