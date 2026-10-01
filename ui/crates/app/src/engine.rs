//! The UI's handle on the engine: one session, shared by every view, and the way views ask it things.
//!
//! The engine is a separate process and every call is a round trip, so a view never waits on one: it hands a
//! payload and a continuation to [`call`], which runs the request off the UI thread and runs the
//! continuation back on it, on the view that asked, if that view is still alive.

use std::{cell::RefCell, rc::Rc};

use bifurc_engine_client::{ClientError, EngineClient, Hello, RuntimeInfo, Sidecar};
use gpui_kit::{AppContext as _, AsyncApp, Context, Global, Window};
use serde_json::Value;

/// A connected engine and, if this process started it, the means to stop it.
#[derive(Clone)]
pub struct EngineSession {
    pub client: EngineClient,
    pub hello: Hello,
    pub info: RuntimeInfo,
    sidecar: Rc<RefCell<Sidecar>>,
}

impl Global for EngineSession {}

impl EngineSession {
    pub fn new(client: EngineClient, hello: Hello, sidecar: Sidecar) -> Self {
        let info = sidecar.info.clone();
        Self { client, hello, info, sidecar: Rc::new(RefCell::new(sidecar)) }
    }

    /// `true` when this UI started the engine (and so stops it on exit) rather than joining one that was
    /// already running, for example the one `bifurc engine start` launched for the CLI.
    pub fn is_owned(&self) -> bool {
        self.sidecar.borrow().is_owned()
    }

    /// Stops the engine if this process started it. Blocks for up to a few seconds; for app shutdown only.
    pub fn release(&self) {
        self.sidecar.borrow_mut().release(&self.client);
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
