//! Open requests as tabs: opening, finding, closing, and keeping the response and details panels pointed at
//! the active one.

use gpui_kit::component::{
    WindowExt as _,
    dock::{PanelId, panel_handle},
    notification::Notification,
};
use gpui_kit::{AppContext as _, Context, Entity, Subscription, Window};

use super::Shell;
use crate::{
    model::{NodeRef, RequestKind},
    panels::request_tab::RequestTab,
    session::{Session, SessionEvent, protocol::ProtocolSession, rest::RestSession, websocket::WsSession},
};
use gpui_kit::component::dock::DockPlacement;

/// How a tab is identified before and after it has an id from the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TabKey {
    Saved { kind: RequestKind, id: String },
    Draft(u64),
}

pub(super) struct OpenTab {
    pub key: TabKey,
    pub session: Session,
    pub panel: Entity<RequestTab>,
    _subscription: Subscription,
}

impl Shell {
    fn tab_for(&self, key: &TabKey) -> Option<&OpenTab> {
        self.tabs.iter().find(|tab| tab.key == *key)
    }

    fn tab_index_of(&self, session: &Session) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.session.same_as(session))
    }

    pub fn open_node(&mut self, node: &NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let NodeRef::Request { kind, id } = node else { return };
        let key = TabKey::Saved { kind: *kind, id: id.clone() };
        if let Some(tab) = self.tab_for(&key) {
            let panel = PanelId::from(tab.panel.entity_id());
            self.dock.update(cx, |area, cx| area.select_panel(panel, window, cx));
            return;
        }
        let model = self.model.clone();
        let id = id.clone();
        let session = match kind {
            RequestKind::Rest => Session::Rest(cx.new(|cx| RestSession::open_saved(model, id, window, cx))),
            RequestKind::WebSocket => Session::WebSocket(cx.new(|cx| WsSession::open_saved(model, id, window, cx))),
            RequestKind::GraphQl | RequestKind::Grpc | RequestKind::Soap => {
                let kind = *kind;
                Session::Protocol(cx.new(|cx| ProtocolSession::open_saved(model, kind, id, window, cx)))
            }
        };
        self.add_tab(key, session, window, cx);
    }

    /// Starts an unsaved request. Only the kinds the desktop UI can author are offered; the others open from
    /// the tree once they exist.
    pub fn new_request(
        &mut self,
        kind: RequestKind,
        folder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let model = self.model.clone();
        let session = match kind {
            RequestKind::Rest => Session::Rest(cx.new(|cx| RestSession::new_draft(model, folder, window, cx))),
            RequestKind::WebSocket => Session::WebSocket(cx.new(|cx| WsSession::new_draft(model, folder, window, cx))),
            RequestKind::GraphQl | RequestKind::Grpc | RequestKind::Soap => {
                window.push_notification(
                    Notification::info(format!("{} requests can't be created here yet.", kind.label())),
                    cx,
                );
                return;
            }
        };
        let key = TabKey::Draft(self.next_draft);
        self.next_draft += 1;
        self.add_tab(key, session, window, cx);
    }

    fn add_tab(&mut self, key: TabKey, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        let shell = cx.weak_entity();
        let panel = cx.new(|cx| RequestTab::new(session.clone(), shell, cx));
        let panel_id = PanelId::from(panel.entity_id());
        let subscription = session.subscribe_in(window, cx, Self::on_session_event);
        self.tabs.push(OpenTab { key, session: session.clone(), panel: panel.clone(), _subscription: subscription });
        self.dock.update(cx, |area, cx| {
            area.add_panel_view(panel_handle(panel), DockPlacement::Center, None, window, cx);
            area.select_panel(panel_id, window, cx);
        });
        self.activate_session(session, cx);
    }

    fn on_session_event(
        &mut self,
        session: &Session,
        event: &SessionEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SessionEvent::Changed => cx.notify(),
            SessionEvent::Saved { id } => {
                if let Some(ix) = self.tab_index_of(session) {
                    let kind = session.kind(cx);
                    self.tabs[ix].key = TabKey::Saved { kind, id: id.clone() };
                    self.explorer
                        .update(cx, |explorer, cx| explorer.reveal(&NodeRef::Request { kind, id: id.clone() }, cx));
                }
                cx.notify();
            }
            SessionEvent::Activity(line) => self.activity.update(cx, |activity, cx| activity.push(line.clone(), cx)),
            SessionEvent::Notice(text) => window.push_notification(Notification::warning(text.clone()), cx),
        }
    }

    /// The tab that is now in front. The response and details panels follow it.
    pub fn activate_session(&mut self, session: Session, cx: &mut Context<Self>) {
        let view = session.response_view(cx);
        self.response.update(cx, |response, cx| response.set_view(Some(view), cx));
        self.details.update(cx, |details, cx| details.set_active(Some(session.clone()), cx));
        self.active = Some(session);
        cx.notify();
    }

    /// A tab left the dock (closed, or displaced); drop what pointed at it.
    pub fn forget_session(&mut self, session: &Session, cx: &mut Context<Self>) {
        self.tabs.retain(|tab| !tab.session.same_as(session));
        if self.active.as_ref().is_some_and(|active| active.same_as(session)) {
            self.active = None;
            self.response.update(cx, |response, cx| response.set_view(None, cx));
            self.details.update(cx, |details, cx| details.set_active(None, cx));
        }
        cx.notify();
    }

    fn close_session(&mut self, session: &Session, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tab_index_of(session) else { return };
        let panel = self.tabs[ix].panel.clone();
        self.dock.update(cx, |area, cx| area.remove_panel(panel, window, cx));
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active.clone() else { return };
        if session.is_dirty(cx) {
            window.push_notification(
                Notification::warning("Save or revert the changes to close this tab.").title("Unsaved changes"),
                cx,
            );
            return;
        }
        self.close_session(&session, window, cx);
    }

    pub(super) fn close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panels: Vec<_> = self.tabs.iter().map(|tab| tab.panel.clone()).collect();
        self.dock.update(cx, |area, cx| {
            for panel in panels {
                area.remove_panel(panel, window, cx);
            }
        });
    }

    /// A saved request that is gone from the workspace (deleted here, elsewhere, or with its folder) has
    /// nothing left to show. A tab with unsaved edits stays: saving it creates the request again.
    pub(super) fn close_vanished_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if !matches!(model.load(), crate::app_model::Load::Ready) {
            return;
        }
        let gone: Vec<Session> = self
            .tabs
            .iter()
            .filter(|tab| match &tab.key {
                TabKey::Saved { kind, id } => model.config().request(*kind, id).is_none() && !tab.session.is_dirty(cx),
                TabKey::Draft(_) => false,
            })
            .map(|tab| tab.session.clone())
            .collect();
        for session in gone {
            self.close_session(&session, window, cx);
        }
    }

    pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.active.clone() {
            session.save(window, cx);
        }
    }

    pub fn revert_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.active.clone() {
            session.revert(window, cx);
        }
    }

    /// Send, or connect: the active request's main verb.
    pub fn run_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.active.clone() {
            session.run(window, cx);
        }
    }

    /// The open tab for a saved request, if any.
    pub(super) fn has_open_tab(&self, kind: RequestKind, id: &str) -> bool {
        self.tab_for(&TabKey::Saved { kind, id: id.to_owned() }).is_some()
    }
}
