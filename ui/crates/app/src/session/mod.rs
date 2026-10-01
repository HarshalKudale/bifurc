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

use gpui_kit::{AnyView, App, Context, Entity, SharedString, Window};

use crate::model::RequestKind;

use self::{protocol::ProtocolSession, rest::RestSession, websocket::WsSession};

#[derive(Debug, Clone, PartialEq)]
pub enum LoadState {
    Loading,
    Ready,
    Failed(String),
}

/// What a session tells the shell, which owns the tabs.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Something the tab strip shows may have changed: the title, the dirty mark, the state.
    Changed,
    /// A draft was saved and now has this id.
    Saved { id: String },
    /// One line for the activity log.
    Activity(String),
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
            Session::Protocol(s) => Some(s.read(cx).saved_id().to_owned()),
        }
    }

    pub fn title(&self, cx: &App) -> SharedString {
        match self {
            Session::Rest(s) => s.read(cx).title(cx),
            Session::WebSocket(s) => s.read(cx).title(cx),
            Session::Protocol(s) => s.read(cx).title(),
        }
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        match self {
            Session::Rest(s) => s.read(cx).is_dirty(cx),
            Session::WebSocket(s) => s.read(cx).is_dirty(cx),
            Session::Protocol(_) => false,
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

    pub fn save(&self, window: &mut Window, cx: &mut App) {
        match self {
            Session::Rest(s) => s.update(cx, |s, cx| s.save(window, cx)),
            Session::WebSocket(s) => s.update(cx, |s, cx| s.save(window, cx)),
            Session::Protocol(_) => {}
        }
    }

    pub fn revert(&self, window: &mut Window, cx: &mut App) {
        match self {
            Session::Rest(s) => s.update(cx, |s, cx| s.revert(window, cx)),
            Session::WebSocket(s) => s.update(cx, |s, cx| s.revert(window, cx)),
            Session::Protocol(_) => {}
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
