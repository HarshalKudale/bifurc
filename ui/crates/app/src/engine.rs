//! The UI's handle on the engine: one session, shared by every view, and the way views ask it things.
//!
//! The engine is a separate process and every call is a round trip, so a view never waits on one: it hands a
//! payload and a continuation to [`call`], which runs the request off the UI thread and runs the
//! continuation back on it, on the view that asked, if that view is still alive.

use std::sync::{Arc, Mutex};

use bifurc_engine_client::{ClientError, EngineClient, Hello, RuntimeInfo, Sidecar};
use gpui_kit::{AppContext as _, AsyncApp, Context, Global, Window};
use serde_json::Value;

/// A connected engine and, if this process started it, the means to stop it.
#[derive(Clone)]
pub struct EngineSession {
    pub client: EngineClient,
    pub hello: Hello,
    pub info: RuntimeInfo,
    sidecar: Arc<Mutex<Sidecar>>,
}

impl Global for EngineSession {}

impl EngineSession {
    pub fn new(client: EngineClient, hello: Hello, sidecar: Sidecar) -> Self {
        let info = sidecar.info.clone();
        Self { client, hello, info, sidecar: Arc::new(Mutex::new(sidecar)) }
    }

    /// `true` when this UI started the engine (and so stops it on exit) rather than joining one that was
    /// already running, for example the one `bifurc engine start` launched for the CLI.
    pub fn is_owned(&self) -> bool {
        self.sidecar.lock().unwrap().is_owned()
    }

    /// Stops the engine if this process started it. Blocks for up to a few seconds; for app shutdown only.
    pub fn release(&self) {
        self.sidecar.lock().unwrap().release(&self.client);
    }

    /// Stops the engine whoever started it and waits for it to be gone. Blocks; call it off the UI thread.
    pub fn stop(&self) -> bool {
        self.sidecar.lock().unwrap().stop(&self.client)
    }
}

/// Short text for a failed call, as the UI shows it.
pub fn describe(error: &ClientError) -> String {
    match error {
        ClientError::Engine(e) => e.message.clone(),
        ClientError::Closed => "The connection to the engine was lost.".to_owned(),
        other => other.to_string(),
    }
}

/// Sends `action` and runs `then` on the asking view with the outcome.
pub fn call<V: 'static>(
    client: &EngineClient,
    cx: &mut Context<V>,
    action: &'static str,
    payload: Value,
    then: impl FnOnce(&mut V, Result<Value, ClientError>, &mut Context<V>) + 'static,
) {
    let client = client.clone();
    let request = cx.background_spawn(async move { client.call(action, payload).await });
    cx.spawn(async move |this, cx: &mut AsyncApp| {
        let outcome = request.await;
        this.update(cx, |this, cx| then(this, outcome, cx)).ok();
    })
    .detach();
}

/// The failure of a command, whether the call failed or the engine answered `{ ok: false, error }`.
pub fn failure(outcome: &Result<Value, ClientError>) -> Option<String> {
    match outcome {
        Ok(result) if result.get("ok").and_then(Value::as_bool) == Some(false) => {
            Some(result.get("error").and_then(Value::as_str).unwrap_or("The engine refused.").to_owned())
        }
        Ok(_) => None,
        Err(error) => Some(describe(error)),
    }
}

/// Stages `bytes` with the engine (`blob.put`). `then` gets the staged file's id, or why it was refused. This is
/// how a file the user picked reaches a command that imports it.
pub fn put_blob<V: 'static>(
    client: &EngineClient,
    cx: &mut Context<V>,
    workspace_id: Option<&str>,
    filename: &str,
    bytes: &[u8],
    then: impl FnOnce(&mut V, Result<String, String>, &mut Context<V>) + 'static,
) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let mut put = serde_json::json!({
        "filename": filename,
        "mimeType": "application/octet-stream",
        "size": bytes.len(),
        "data": STANDARD.encode(bytes),
    });
    if let Some(id) = workspace_id {
        put["workspaceId"] = Value::String(id.to_owned());
    }
    call(client, cx, "blob.put", put, move |this, outcome, cx| {
        let blob_id = match outcome {
            Ok(staged) => staged.get("blobId").and_then(Value::as_str).map(str::to_owned),
            Err(error) => return then(this, Err(describe(&error)), cx),
        };
        then(this, blob_id.ok_or_else(|| "The engine did not accept the file.".to_owned()), cx)
    });
}

/// [`put_blob`], then `action` with the staged file's id as `blobId` next to the fields of `extra`. `then`
/// hears the command's result, or the first thing that went wrong, as text.
#[allow(clippy::too_many_arguments)]
pub fn upload_and_call<V: 'static>(
    client: &EngineClient,
    cx: &mut Context<V>,
    workspace_id: Option<&str>,
    filename: &str,
    bytes: &[u8],
    action: &'static str,
    extra: serde_json::Map<String, Value>,
    then: impl FnOnce(&mut V, Result<Value, String>, &mut Context<V>) + 'static,
) {
    let next_client = client.clone();
    put_blob(client, cx, workspace_id, filename, bytes, move |this, staged, cx| {
        let blob_id = match staged {
            Ok(blob_id) => blob_id,
            Err(message) => return then(this, Err(message), cx),
        };
        let mut payload = extra;
        payload.insert("blobId".to_owned(), Value::String(blob_id));
        call(&next_client, cx, action, Value::Object(payload), move |this, outcome, cx| {
            then(this, failure(&outcome).map_or_else(|| Ok(outcome.unwrap_or(Value::Null)), Err), cx)
        });
    });
}

type Fetched<V> = Box<dyn FnOnce(&mut V, Result<(Vec<u8>, String), String>, &mut Context<V>)>;

/// The bytes of an export (`{ ok, suggestedName, inline | blobId }`) and the name it suggests. Small ones
/// arrive inline; large ones are read from the engine's staging area a chunk at a time and released after.
/// `this` is the view the continuation `then` runs on.
pub fn fetch_artifact<V: 'static>(
    this: &mut V,
    client: &EngineClient,
    cx: &mut Context<V>,
    workspace_id: &str,
    artifact: &Value,
    then: impl FnOnce(&mut V, Result<(Vec<u8>, String), String>, &mut Context<V>) + 'static,
) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    if artifact.get("ok").and_then(Value::as_bool) == Some(false) {
        let message = artifact.get("error").and_then(Value::as_str).unwrap_or("The engine refused.").to_owned();
        return then(this, Err(message), cx);
    }
    let name = artifact.get("suggestedName").and_then(Value::as_str).unwrap_or("export").to_owned();
    if let Some(inline) = artifact.get("inline").and_then(Value::as_str) {
        let bytes = STANDARD.decode(inline).map_err(|error| error.to_string());
        return then(this, bytes.map(|bytes| (bytes, name)), cx);
    }
    match artifact.get("blobId").and_then(Value::as_str) {
        Some(blob) => {
            let read = ChunkRead { workspace_id: workspace_id.to_owned(), blob: blob.to_owned(), name };
            read_chunks(client.clone(), cx, read, 0, Vec::new(), Box::new(then));
        }
        None => then(this, Err("The engine returned no data.".to_owned()), cx),
    }
}

/// What is being read back from the staging area.
struct ChunkRead {
    workspace_id: String,
    blob: String,
    name: String,
}

const CHUNK_BYTES: usize = 512 * 1024;

fn read_chunks<V: 'static>(
    client: EngineClient,
    cx: &mut Context<V>,
    read: ChunkRead,
    offset: usize,
    mut bytes: Vec<u8>,
    then: Fetched<V>,
) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let payload = serde_json::json!({
        "blobId": read.blob,
        "workspaceId": read.workspace_id,
        "offset": offset,
        "length": CHUNK_BYTES,
    });
    let next_client = client.clone();
    call(&client, cx, "blob.read", payload, move |this, outcome, cx| {
        let release = |cx: &mut Context<V>| {
            let payload = serde_json::json!({ "blobId": read.blob, "workspaceId": read.workspace_id });
            call(&next_client, cx, "blob.release", payload, |_, _, _| {});
        };
        let chunk = match outcome {
            Ok(chunk) => chunk,
            Err(error) => {
                release(cx);
                return then(this, Err(describe(&error)), cx);
            }
        };
        let decoded = match chunk.get("data").and_then(Value::as_str).map(|data| STANDARD.decode(data)) {
            Some(Ok(decoded)) => decoded,
            _ => {
                release(cx);
                return then(this, Err("The engine returned no data.".to_owned()), cx);
            }
        };
        let eof = chunk.get("eof").and_then(Value::as_bool).unwrap_or(false);
        if decoded.is_empty() && !eof {
            release(cx);
            return then(this, Err("The engine stopped sending data.".to_owned()), cx);
        }
        bytes.extend_from_slice(&decoded);
        if eof {
            release(cx);
            then(this, Ok((bytes, read.name)), cx);
        } else {
            let next = offset + decoded.len();
            read_chunks(next_client.clone(), cx, read, next, bytes, then);
        }
    });
}

/// Changes some fields of a saved entity, addressed by id: the engine's copy is read, `patch` is written
/// over it, and the result is written back. Fields the caller does not name are left as the engine has them.
pub fn patch_entity<V: 'static>(
    client: &EngineClient,
    cx: &mut Context<V>,
    workspace_id: &str,
    kind: &'static str,
    id: &str,
    patch: serde_json::Map<String, Value>,
    then: impl FnOnce(&mut V, Result<(), String>, &mut Context<V>) + 'static,
) {
    let load = serde_json::json!({ "workspaceId": workspace_id, "kind": kind, "id": id });
    let update_client = client.clone();
    call(client, cx, "entity.load", load, move |this, outcome, cx| {
        let entity = match outcome {
            Ok(result) => match result.get("entity").cloned() {
                Some(Value::Object(mut entity)) => {
                    entity.extend(patch);
                    entity
                }
                _ => return then(this, Err("It no longer exists.".to_owned()), cx),
            },
            Err(error) => return then(this, Err(describe(&error)), cx),
        };
        let update = serde_json::json!({ "kind": kind, "entity": entity });
        call(&update_client, cx, "entity.update", update, move |this, outcome, cx| {
            then(this, failure(&outcome).map_or(Ok(()), Err), cx)
        });
    });
}

/// [`call`] for continuations that need the window, such as one that fills an input.
pub fn call_in<V: 'static>(
    client: &EngineClient,
    window: &Window,
    cx: &mut Context<V>,
    action: &'static str,
    payload: Value,
    then: impl FnOnce(&mut V, Result<Value, ClientError>, &mut Window, &mut Context<V>) + 'static,
) {
    let client = client.clone();
    let request = cx.background_spawn(async move { client.call(action, payload).await });
    cx.spawn_in(window, async move |this, cx| {
        let outcome = request.await;
        this.update_in(cx, |this, window, cx| then(this, outcome, window, cx)).ok();
    })
    .detach();
}
