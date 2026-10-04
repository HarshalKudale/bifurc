//! Open requests. One entity per open tab, one kind per request type.
//!
//! A session owns the *edit* of a request (or, for the kinds that cannot be edited here yet, just its
//! contents), and the engine owns the request. [`Session`] is the handle the shell holds: it hides which
//! kind it is behind the handful of things the shell does to every tab — title it, ask if it is dirty, save
//! it, run it, and find its response view.

pub mod connection;
pub mod exchange;
pub mod protocol;
pub mod rest;
pub mod websocket;

use gpui_kit::component::input::InputState;
use gpui_kit::{AnyView, App, Context, Entity, SharedString, Window};

use crate::{
    app_model::AppModel,
    components::{EntityBar, FolderPicker},
    model::{RequestKind, snippet::RequestParts, tree::folder_path},
};

use self::{protocol::ProtocolSession, rest::RestSession, websocket::WsSession};

#[derive(Debug, Clone, PartialEq)]
pub enum LoadState {
    Loading,
    Ready,
    Failed(String),
}

/// Who asked for a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMode {
    /// The user did, with the Save button or its key: what stops the save is said aloud.
    Explicit,
    /// The editor did, after a change: an edit that cannot be written yet is left for the next change.
    Auto,
}

/// What a draft keeps when its request type is changed: the fields every type has some form of.
#[derive(Debug, Clone, Default)]
pub struct Carry {
    /// `None` when the name was still the type's default one.
    pub name: Option<String>,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// The folder the engine put a saved entity in. The engine decides it: an entity saved with none goes into Default.
pub fn engine_folder(entity: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    entity.get("folderId").and_then(serde_json::Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// What a session tells the shell, which owns the tabs.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Something the tab strip shows may have changed: the title, the dirty mark, the state.
    Changed,
    /// A draft was saved and now has this id.
    Saved { id: String },
    /// Something the user should be told about.
    Notice(String),
}

#[derive(Clone)]
pub enum Session {
    Rest(Entity<RestSession>),
    WebSocket(Entity<WsSession>),
    Protocol(Entity<ProtocolSession>),
}

impl Session {
    pub fn kind(&self, cx: &App) -> RequestKind {
        match self {
            Session::Rest(_) => RequestKind::Rest,
            Session::WebSocket(_) => RequestKind::WebSocket,
            Session::Protocol(s) => s.read(cx).kind(),
        }
    }

    pub fn saved_id(&self, cx: &App) -> Option<String> {
        match self {
            Session::Rest(s) => s.read(cx).saved_id().map(str::to_owned),
            Session::WebSocket(s) => s.read(cx).saved_id().map(str::to_owned),
            Session::Protocol(s) => s.read(cx).saved_id().map(str::to_owned),
        }
    }

    pub fn title(&self, cx: &App) -> SharedString {
        match self {
            Session::Rest(s) => s.read(cx).title(cx),
            Session::WebSocket(s) => s.read(cx).title(cx),
            Session::Protocol(s) => s.read(cx).title(cx),
        }
    }

    /// What a tab leads with: the method for REST, the type's short badge for everything else.
    pub fn badge(&self, cx: &App) -> &'static str {
        match self {
            Session::Rest(s) => s.read(cx).method(cx),
            _ => self.kind(cx).badge(),
        }
    }

    /// The request as an HTTP call, for a code snippet. `None` for a type a snippet cannot express.
    pub fn request_parts(&self, cx: &App) -> Option<RequestParts> {
        match self {
            Session::Rest(s) => Some(s.read(cx).request_parts(cx)),
            Session::Protocol(s) => s.read(cx).request_parts(cx),
            Session::WebSocket(_) => None,
        }
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        match self {
            Session::Rest(s) => s.read(cx).is_dirty(cx),
            Session::WebSocket(s) => s.read(cx).is_dirty(cx),
            Session::Protocol(s) => s.read(cx).is_dirty(cx),
        }
    }

    /// The view that shows what the request produced: a response, or a message stream.
    pub fn response_view(&self, cx: &App) -> AnyView {
        match self {
            Session::Rest(s) => s.read(cx).exchange().clone().into(),
            Session::WebSocket(s) => s.read(cx).connection().clone().into(),
            Session::Protocol(s) => s.read(cx).exchange().clone().into(),
        }
    }

    pub fn editor_view(&self) -> AnyView {
        match self {
            Session::Rest(s) => s.clone().into(),
            Session::WebSocket(s) => s.clone().into(),
            Session::Protocol(s) => s.clone().into(),
        }
    }

    /// The field the request's name is typed in. The shell shows it in the breadcrumb.
    pub fn name_input(&self, cx: &App) -> Entity<InputState> {
        match self {
            Session::Rest(s) => s.read(cx).name_input().clone(),
            Session::WebSocket(s) => s.read(cx).name_input().clone(),
            Session::Protocol(s) => s.read(cx).name_input().clone(),
        }
    }

    /// The folder the request is in, or will be created in.
    pub fn folder_id(&self, cx: &App) -> Option<String> {
        match self {
            Session::Rest(s) => s.read(cx).folder_id().map(str::to_owned),
            Session::WebSocket(s) => s.read(cx).folder_id().map(str::to_owned),
            Session::Protocol(s) => s.read(cx).folder_id().map(str::to_owned),
        }
    }

    /// Puts the request in another folder of its type, or at the top level.
    pub fn set_folder(&self, folder: Option<String>, cx: &mut App) {
        match self {
            Session::Rest(s) => s.update(cx, |s, cx| s.set_folder(folder, cx)),
            Session::WebSocket(s) => s.update(cx, |s, cx| s.set_folder(folder, cx)),
            Session::Protocol(s) => s.update(cx, |s, cx| s.set_folder(folder, cx)),
        }
    }

    /// The names of the folders the request is in, outermost first.
    pub fn folder_trail(&self, model: &AppModel, cx: &App) -> Vec<String> {
        let folders = model.config().request_folder_rows(self.kind(cx));
        self.folder_id(cx).map(|id| folder_path(&folders, &id)).unwrap_or_default()
    }

    /// What the tab's footer says about the request and offers to do with it.
    pub fn bar(&self, model: &AppModel, cx: &App) -> EntityBar {
        let (saving, problem) = match self {
            Session::Rest(s) => (s.read(cx).is_saving(), s.read(cx).problem(cx)),
            Session::WebSocket(s) => (s.read(cx).is_saving(), s.read(cx).problem(cx)),
            Session::Protocol(s) => (s.read(cx).is_saving(), s.read(cx).problem(cx)),
        };
        let saved_id = self.saved_id(cx);
        EntityBar {
            saving,
            unsaved: saved_id.is_some() && self.is_dirty(cx),
            problem,
            closable: true,
            folder: Some(FolderPicker::new(self.folder_id(cx), &model.config().request_folder_rows(self.kind(cx)))),
            ..EntityBar::of(model, saved_id.as_deref())
        }
    }

    /// Creates a draft, or writes a saved request's edit now instead of a moment from now.
    pub fn save(&self, mode: SaveMode, window: &mut Window, cx: &mut App) {
        match self {
            Session::Rest(s) => s.update(cx, |s, cx| s.save(mode, window, cx)),
            Session::WebSocket(s) => s.update(cx, |s, cx| s.save(mode, window, cx)),
            Session::Protocol(s) => s.update(cx, |s, cx| s.save(mode, window, cx)),
        }
    }

    /// Writes a saved request's waiting edit now: its tab is about to close.
    pub fn flush(&self, window: &mut Window, cx: &mut App) {
        match self {
            Session::Rest(s) => s.update(cx, |s, cx| s.flush(window, cx)),
            Session::WebSocket(s) => s.update(cx, |s, cx| s.flush(window, cx)),
            Session::Protocol(s) => s.update(cx, |s, cx| s.flush(window, cx)),
        }
    }

    /// What this draft keeps if it becomes a request of another type.
    pub fn carry(&self, cx: &App) -> Carry {
        match self {
            Session::Rest(s) => s.read(cx).carry(cx),
            Session::WebSocket(s) => s.read(cx).carry(cx),
            Session::Protocol(s) => s.read(cx).carry(cx),
        }
    }

    /// Fills this draft with what was typed into a draft of another type.
    pub fn apply_carry(&self, carry: &Carry, window: &mut Window, cx: &mut App) {
        match self {
            Session::Rest(s) => s.update(cx, |s, cx| s.apply_carry(carry, window, cx)),
            Session::WebSocket(s) => s.update(cx, |s, cx| s.apply_carry(carry, window, cx)),
            Session::Protocol(s) => s.update(cx, |s, cx| s.apply_carry(carry, window, cx)),
        }
    }

    /// Fills this draft with a request of its type that the engine built, from a cURL command.
    pub fn adopt_import(&self, entity: serde_json::Map<String, serde_json::Value>, window: &mut Window, cx: &mut App) {
        match self {
            Session::Rest(s) => s.update(cx, |s, cx| s.adopt_import(entity, window, cx)),
            Session::WebSocket(s) => s.update(cx, |s, cx| s.adopt_import(entity, window, cx)),
            Session::Protocol(s) => s.update(cx, |s, cx| s.adopt_import(entity, window, cx)),
        }
    }

    /// The request's main verb: send it, or open its connection.
    pub fn run(&self, window: &mut Window, cx: &mut App) {
        match self {
            Session::Rest(s) => s.update(cx, |s, cx| s.send(window, cx)),
            Session::WebSocket(s) => s.update(cx, |s, cx| s.connect(window, cx)),
            Session::Protocol(s) => s.update(cx, |s, cx| s.send(window, cx)),
        }
    }

    pub fn same_as(&self, other: &Session) -> bool {
        self.entity_id() == other.entity_id()
    }

    fn entity_id(&self) -> gpui_kit::EntityId {
        match self {
            Session::Rest(s) => s.entity_id(),
            Session::WebSocket(s) => s.entity_id(),
            Session::Protocol(s) => s.entity_id(),
        }
    }

    /// Like [`subscribe`](Self::subscribe), for an owner that also needs the window, and which session spoke.
    pub fn subscribe_in<T: 'static>(
        &self,
        window: &mut Window,
        cx: &mut Context<T>,
        handler: impl Fn(&mut T, &Session, &SessionEvent, &mut Window, &mut Context<T>) + 'static,
    ) -> gpui_kit::Subscription {
        let session = self.clone();
        match self {
            Session::Rest(s) => {
                cx.subscribe_in(s, window, move |this, _, event, window, cx| handler(this, &session, event, window, cx))
            }
            Session::WebSocket(s) => {
                cx.subscribe_in(s, window, move |this, _, event, window, cx| handler(this, &session, event, window, cx))
            }
            Session::Protocol(s) => {
                cx.subscribe_in(s, window, move |this, _, event, window, cx| handler(this, &session, event, window, cx))
            }
        }
    }

    pub fn subscribe<T: 'static>(
        &self,
        cx: &mut Context<T>,
        mut handler: impl FnMut(&mut T, &SessionEvent, &mut Context<T>) + 'static,
    ) -> gpui_kit::Subscription {
        match self {
            Session::Rest(s) => cx.subscribe(s, move |this, _, event, cx| handler(this, event, cx)),
            Session::WebSocket(s) => cx.subscribe(s, move |this, _, event, cx| handler(this, event, cx)),
            Session::Protocol(s) => cx.subscribe(s, move |this, _, event, cx| handler(this, event, cx)),
        }
    }
}
