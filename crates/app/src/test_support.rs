//! The harness UI tests drive screens with: a fake engine behind the real [`EngineClient`], and a headless
//! window holding the real [`Shell`].
//!
//! The client is built over in-memory byte streams ([`EngineClient::from_halves`]), so everything between a
//! screen and the wire is production code. The fake answers each request frame the moment it is written,
//! from canned replies, and records it: tests assert on the commands sent (action and key parameters) and on
//! what the window shows. No engine process and no sockets; time is the test executor's clock.

use std::{
    collections::HashMap,
    io::{self, Read, Write},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use bifurc_engine_client::EngineClient;
use gpui_base::test_support::{ElementSnapshot, find, snapshots};
use gpui_kit::component::Root;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ElementId, Entity, InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollDelta, ScrollWheelEvent, TestAppContext, Window,
    px, size,
};
use serde_json::{Value, json};

use crate::{app_model::AppModel, commands, engine::EngineSession, preferences::Preferences, shell::Shell, theme};

/// One request the UI sent.
#[derive(Debug, Clone)]
pub struct Call {
    pub action: String,
    pub payload: Value,
}

type Reply = Arc<dyn Fn(&Value) -> Result<Value, String> + Send + Sync>;

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    replies: HashMap<String, Reply>,
    /// Entities `entity.load` can return, by `(kind, id)`.
    entities: HashMap<(String, String), Value>,
    /// Bytes of a frame still being written.
    pending: Vec<u8>,
    /// Actions whose answer is withheld until [`FakeEngine::release`], and the requests waiting for it.
    held: Vec<String>,
    waiting: Vec<(String, Value)>,
}

struct Inner {
    state: Mutex<State>,
    to_client: Mutex<mpsc::Sender<Vec<u8>>>,
}

/// The engine, as far as the UI can tell. Cheap to clone; clones share their state.
#[derive(Clone)]
pub struct FakeEngine(Arc<Inner>);

struct FromEngine(mpsc::Receiver<Vec<u8>>, io::Cursor<Vec<u8>>);

impl Read for FromEngine {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let n = self.1.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            match self.0.recv() {
                Ok(chunk) => self.1 = io::Cursor::new(chunk),
                Err(_) => return Ok(0),
            }
        }
    }
}

struct ToEngine(FakeEngine);

impl Write for ToEngine {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.receive(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn frame(message: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(message).expect("a frame");
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

impl FakeEngine {
    /// An engine with a one-workspace, onboarded setup and nothing in the workspace.
    pub fn new() -> (Self, EngineClient) {
        let (to_client, from_engine) = mpsc::channel();
        let fake = Self(Arc::new(Inner { state: Mutex::default(), to_client: Mutex::new(to_client) }));
        let client =
            EngineClient::from_halves(FromEngine(from_engine, io::Cursor::new(Vec::new())), ToEngine(fake.clone()));
        fake.config(json!({}));
        fake.reply("setup.getState", json!({ "gitInstalled": true, "onboarded": true, "serversConfirmed": true }));
        (fake, client)
    }

    /// What `config.get` answers: the defaults (workspace `w1`) with `extra`'s keys laid over them.
    pub fn config(&self, extra: Value) {
        let mut config = json!({
            "port": 80, "forwardProxyPort": 8080, "activeWorkspaceId": "w1",
            "workspaces": [{ "id": "w1", "name": "Work" }],
            "environments": [{ "id": "__global__", "name": "Global", "variables": [] }],
        });
        config.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        self.reply("config.get", config);
    }

    /// Answers `action` with `data` from now on.
    pub fn reply(&self, action: &str, data: Value) {
        self.reply_with(action, move |_| Ok(data.clone()));
    }

    pub fn reply_with(&self, action: &str, f: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static) {
        self.0.state.lock().unwrap().replies.insert(action.to_owned(), Arc::new(f));
    }

    /// Makes `entity` loadable by `entity.load` as `kind`.
    pub fn entity(&self, kind: &str, entity: Value) {
        let id = entity["id"].as_str().expect("an entity with an id").to_owned();
        self.0.state.lock().unwrap().entities.insert((kind.to_owned(), id), entity);
    }

    /// Withholds the answer to `action` (a send that is still in flight) until [`release`](Self::release).
    pub fn hold(&self, action: &str) {
        self.0.state.lock().unwrap().held.push(action.to_owned());
    }

    /// Answers every request of `action` that was held, with `data`.
    pub fn release(&self, action: &str, data: Value) {
        let waiting: Vec<_> = {
            let mut state = self.0.state.lock().unwrap();
            let (these, rest) = std::mem::take(&mut state.waiting).into_iter().partition(|(a, _)| a == action);
            state.waiting = rest;
            these
        };
        for (_, id) in waiting {
            self.0.to_client.lock().unwrap().send(frame(&json!({ "id": id, "ok": true, "data": data }))).ok();
        }
    }

    /// Sends a broadcast to the UI.
    pub fn emit(&self, event: &str, payload: Value) {
        let bytes = frame(&json!({ "event": event, "seq": 1, "payload": payload }));
        self.0.to_client.lock().unwrap().send(bytes).ok();
    }

    /// Every request so far, oldest first.
    pub fn calls(&self) -> Vec<Call> {
        self.0.state.lock().unwrap().calls.clone()
    }

    /// The payloads of the requests that ran `action`.
    pub fn sent(&self, action: &str) -> Vec<Value> {
        self.calls().into_iter().filter(|c| c.action == action).map(|c| c.payload).collect()
    }

    pub fn forget_calls(&self) {
        self.0.state.lock().unwrap().calls.clear();
    }

    fn receive(&self, bytes: &[u8]) {
        let mut requests = Vec::new();
        {
            let mut state = self.0.state.lock().unwrap();
            state.pending.extend_from_slice(bytes);
            while state.pending.len() >= 4 {
                let len = u32::from_be_bytes(state.pending[..4].try_into().unwrap()) as usize;
                if state.pending.len() < 4 + len {
                    break;
                }
                let body: Vec<u8> = state.pending.drain(..4 + len).skip(4).collect();
                requests.push(serde_json::from_slice::<Value>(&body).expect("a request frame"));
            }
        }
        for request in requests {
            let id = request["id"].clone();
            let action = request["action"].as_str().unwrap_or_default().to_owned();
            let payload = request["payload"].clone();
            let reply = {
                let mut state = self.0.state.lock().unwrap();
                state.calls.push(Call { action: action.clone(), payload: payload.clone() });
                if state.held.contains(&action) {
                    state.waiting.push((action.clone(), id));
                    continue;
                }
                match state.replies.get(&action) {
                    Some(reply) => Some(reply.clone()),
                    None if action == "entity.load" => {
                        let key = (
                            payload["kind"].as_str().unwrap_or_default().to_owned(),
                            payload["id"].as_str().unwrap_or_default().to_owned(),
                        );
                        let found = state.entities.get(&key).cloned();
                        Some(Arc::new(move |_: &Value| match &found {
                            Some(entity) => Ok(json!({ "entity": entity })),
                            None => Err("not found".to_owned()),
                        }) as Reply)
                    }
                    None => None,
                }
            };
            let answer = match reply.map(|reply| reply(&payload)) {
                Some(Ok(data)) => json!({ "id": id, "ok": true, "data": data }),
                Some(Err(message)) => {
                    json!({ "id": id, "ok": false, "error": { "code": "ENGINE_ERROR", "message": message } })
                }
                // Anything not canned succeeds with the shape most commands answer with.
                None => json!({ "id": id, "ok": true, "data": { "ok": true } }),
            };
            self.0.to_client.lock().unwrap().send(frame(&answer)).ok();
        }
    }
}

/// A headless window holding the real shell, over a [`FakeEngine`].
pub struct Ui {
    pub cx: TestAppContext,
    pub window: AnyWindowHandle,
    pub fake: FakeEngine,
    pub shell: Entity<Shell>,
}

impl Ui {
    /// Opens the window and lets the first reads finish. `prepare` shapes the fake before anything is asked.
    pub fn open(cx: &mut TestAppContext, prepare: impl FnOnce(&FakeEngine)) -> Self {
        Self::open_sized(cx, size(px(1440.), px(900.)), prepare)
    }

    /// [`open`](Self::open) with a window of another size.
    pub fn open_sized(
        cx: &mut TestAppContext,
        window_size: gpui_kit::Size<gpui_kit::Pixels>,
        prepare: impl FnOnce(&FakeEngine),
    ) -> Self {
        // Replies are handed over by the engine reader thread, which the scheduler must not mind.
        cx.executor().allow_parking();
        let (fake, client) = FakeEngine::new();
        prepare(&fake);
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            cx.set_global(Preferences::default());
            commands::bind_keys(cx);
            commands::register_actions(cx);
        });
        let mut built = None;
        let handle = cx.open_window(window_size, |window, cx| {
            let model = cx.new(|cx| AppModel::new(EngineSession::for_test(client), cx));
            let shell = cx.new(|cx| Shell::new(model.clone(), window, cx));
            built = Some(shell.clone());
            Root::new(shell, window, cx)
        });
        // The app-level handlers reach the shell through the active window.
        cx.update_window(handle.into(), |_, window, _| window.activate_window()).expect("the window is open");
        let shell = built.expect("the window built the shell");
        let mut ui = Self { cx: cx.clone(), window: handle.into(), fake, shell };
        ui.settle();
        ui
    }

    /// Runs everything queued, and waits out the engine reader thread's hand-over of replies: a request is
    /// answered on a real thread, so the executor is polled until the traffic has been quiet for a few rounds.
    pub fn settle(&mut self) {
        let mut quiet = 0;
        let mut seen = self.fake.calls().len();
        while quiet < 3 {
            self.cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(2));
            self.cx.run_until_parked();
            let now = self.fake.calls().len();
            quiet = if now == seen { quiet + 1 } else { 0 };
            seen = now;
        }
        self.frame();
    }

    /// Completes a frame, so queries see the latest state.
    pub fn frame(&mut self) {
        self.cx.update_window(self.window, |_, window, cx| draw(window, cx)).expect("the window is open");
    }

    /// Moves the test clock on, running whatever timers fall due (an autosave debounce, a refresh delay).
    pub fn advance(&mut self, by: Duration) {
        self.cx.executor().advance_clock(by);
        self.settle();
    }

    /// Brings a screen to the front, as the sidebar does.
    pub fn show(&mut self, screen: crate::shell::Screen) {
        let shell = self.shell.clone();
        self.cx
            .update_window(self.window, |_, window, cx| {
                shell.update(cx, |shell, cx| shell.show_screen(screen, window, cx))
            })
            .expect("the window is open");
        self.settle();
    }

    /// Clicks the centre of the element with this id, with a real pointer event.
    pub fn click(&mut self, id: impl Into<ElementId>) {
        self.cx.update_window(self.window, |_, window, cx| click(window, id, cx)).expect("the window is open");
        self.settle();
    }

    /// Turns the mouse wheel by `lines` (positive scrolls the content up) with the pointer at (`x`, `y`).
    pub fn scroll_at(&mut self, x: f32, y: f32, lines: f32) {
        self.cx
            .update_window(self.window, |_, window, cx| {
                draw(window, cx);
                let position = gpui_kit::point(px(x), px(y));
                window.dispatch_event(
                    MouseMoveEvent { position, pressed_button: None, modifiers: Default::default() }
                        .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    ScrollWheelEvent {
                        position,
                        delta: ScrollDelta::Lines(gpui_kit::point(0., -lines)),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                draw(window, cx);
            })
            .expect("the window is open");
        self.settle();
    }

    /// Presses a key or chord (`ctrl-s`, `escape`, `secondary-1`) on whatever has focus.
    pub fn press(&mut self, keys: &str) {
        self.cx.update_window(self.window, |_, window, cx| press(window, keys, cx)).expect("the window is open");
        self.settle();
    }

    /// Types text at the current focus.
    pub fn input(&mut self, text: &str) {
        self.cx.update_window(self.window, |_, window, cx| input(window, text, cx)).expect("the window is open");
        self.settle();
    }

    /// Fills the open prompt dialog, field by field (Tab moves on, and each field's text is replaced), and
    /// confirms it with its button.
    pub fn answer_prompt(&mut self, values: &[&str]) {
        for (ix, value) in values.iter().enumerate() {
            if ix > 0 {
                self.press("tab");
            }
            self.press("ctrl-a");
            self.input(value);
        }
        self.click("prompt-confirm");
    }

    /// Whether anything on screen has this id; unlike [`find`](Self::find) it does not mind there being several.
    pub fn has(&mut self, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.frame();
        self.cx
            .update_window(self.window, |_, window, _| snapshots(window).iter().any(|s| s.path().last() == Some(&id)))
            .expect("the window is open")
    }

    /// How many elements on screen have this id.
    pub fn count(&mut self, id: impl Into<ElementId>) -> usize {
        let id = id.into();
        self.frame();
        self.cx
            .update_window(self.window, |_, window, _| {
                snapshots(window).iter().filter(|s| s.path().last() == Some(&id)).count()
            })
            .expect("the window is open")
    }
}

fn draw(window: &mut Window, cx: &mut App) {
    window.refresh();
    window.draw(cx).clear(cx);
}

fn click(window: &mut Window, id: impl Into<ElementId>, cx: &mut App) {
    let id = id.into();
    // An overlay that has just opened slides into place on the wall clock, so the target is aimed at once it
    // has stopped moving.
    let mut target = None;
    for _ in 0..40 {
        draw(window, cx);
        let now = find(window, &[], &id).unwrap_or_else(|| panic!("no element {id:?} on screen"));
        if target.as_ref().is_some_and(|before: &ElementSnapshot| before.bounds() == now.bounds()) {
            break;
        }
        target = Some(now);
        std::thread::sleep(Duration::from_millis(10));
    }
    let target = target.expect("looked at least once");
    assert!(target.visible(), "{id:?} is not visible");
    let position = target.bounds().center();
    for event in [
        MouseMoveEvent { position, pressed_button: None, modifiers: Default::default() }.to_platform_input(),
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        MouseUpEvent { button: MouseButton::Left, position, modifiers: Default::default(), click_count: 1 }
            .to_platform_input(),
    ] {
        window.dispatch_event(event, cx);
        draw(window, cx);
    }
}

fn press(window: &mut Window, keys: &str, cx: &mut App) {
    let key = Keystroke::parse(keys).unwrap_or_else(|error| panic!("invalid keystroke {keys:?}: {error}"));
    draw(window, cx);
    // Enter and Tab run their bindings without the simulated input method also typing them.
    let key = if matches!(key.key.as_str(), "enter" | "tab") {
        window.dispatch_event(
            KeyDownEvent { keystroke: key.clone(), is_held: false, prefer_character_input: false }.to_platform_input(),
            cx,
        );
        key
    } else {
        let key = key.with_simulated_ime();
        window.dispatch_keystroke(key.clone(), cx);
        key
    };
    window.dispatch_event(KeyUpEvent { keystroke: key }.to_platform_input(), cx);
    draw(window, cx);
}

fn input(window: &mut Window, text: &str, cx: &mut App) {
    draw(window, cx);
    for character in text.chars() {
        let text = character.to_string();
        let mut key = Keystroke::parse(&text).expect("a character is a keystroke");
        key.key_char = Some(text);
        window.dispatch_keystroke(key, cx);
        draw(window, cx);
    }
}
