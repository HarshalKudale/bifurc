//! The Requests screen: the folder tree, the open requests as tabs, and the active request's editor and
//! response as docks.
//!
//! The screen owns which requests are open and which is in front. It holds one dock area, not one per tab:
//! its "Request" and "Response" panels stay where the user put them and follow the active tab. An open
//! request is a [`Session`]; a folder opened to be run is a [`FolderRunner`], which fills the tab instead of
//! the docks. The workspace comes from [`AppModel`]; everything that changes a saved request is an engine
//! command.

mod runner;
mod tree;

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _, StyledExt as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    dock::{DockArea, DockLayout, DockSkin, panel_handle},
    h_flex,
    menu::DropdownMenu as _,
    resizable::{h_resizable, resizable_panel},
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, ScrollStrategy, Styled as _,
    Subscription, Window, div, px, rems,
};
use serde_json::{Value, json};

use self::runner::{FolderRunner, RunnerEvent};
use self::tree::{RequestTree, RequestTreeEvent, is_deleted, new_menu};
use crate::{
    app_model::{AppModel, Deleted, Load, ModelEvent},
    components::{
        CURL_TAB_BADGE, CURL_TAB_TITLE, CurlImport, CurlImportEvent, CurlTarget, EntityAction, StripTab, TabAction,
        TabStrip,
        dialogs::{
            FOLDER_DELETE_CONSEQUENCE, PromptField, confirm_danger, delete_consequence, prompt, resolve_deleted,
        },
        entity_bar, tab_strip,
    },
    engine::{call, call_in, describe, patch_entity},
    model::{NodeRef, RequestKind, config::REQUEST_FOLDER_KIND, tree::folder_path},
    panels::host::HostPanel,
    preferences::Preferences,
    session::{
        SaveMode, Session, SessionEvent, mcp::McpSession, protocol::ProtocolSession, rest::RestSession,
        socketio::SocketIoSession,
        sse::SseSession, websocket::WsSession,
    },
    shell::ActiveTarget,
};

/// How a tab is identified before and after it has an id from the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TabKey {
    Saved { kind: RequestKind, id: String },
    Draft(u64),
    /// The runner of the REST folder with this id.
    Runner(String),
}

enum TabContent {
    Request(Session),
    Runner(Entity<FolderRunner>),
    /// A tab waiting for a cURL command. It becomes a `Request` of the type the command describes.
    Curl(Entity<CurlImport>),
}

struct OpenTab {
    key: TabKey,
    content: TabContent,
    _subscription: Subscription,
}

impl OpenTab {
    fn session(&self) -> Option<&Session> {
        match &self.content {
            TabContent::Request(session) => Some(session),
            TabContent::Runner(_) | TabContent::Curl(_) => None,
        }
    }

    /// A draft that was typed into: closing its tab loses what was typed.
    fn is_unsaved_draft(&self, cx: &App) -> bool {
        matches!(self.key, TabKey::Draft(_)) && self.session().is_some_and(|session| session.is_dirty(cx))
    }
}

/// One line about a request type, for the list a screen with nothing open offers.
fn kind_summary(kind: RequestKind) -> &'static str {
    match kind {
        RequestKind::Rest => "An HTTP call with a method, headers and a body",
        RequestKind::GraphQl => "A query or mutation, with its variables",
        RequestKind::Grpc => "A call to a method of a gRPC service",
        RequestKind::Soap => "An XML envelope sent to a SOAP action",
        RequestKind::WebSocket => "A connection that sends and receives messages",
        RequestKind::Sse => "A one-way stream of events from a server",
        RequestKind::SocketIo => "A connection that emits and receives named events",
        RequestKind::Mcp => "The tools, prompts and resources of an MCP server",
    }
}

pub struct RequestsScreen {
    model: Entity<AppModel>,
    tree: Entity<RequestTree>,
    tabs: Vec<OpenTab>,
    active: Option<usize>,
    tab_scroll: VirtualListScrollHandle,
    dock: Entity<DockArea>,
    /// Kept so the dock's appearance can be changed later; the area holds the same skin.
    _skin: Rc<DockSkin>,
    request_panel: Entity<HostPanel>,
    response_panel: Entity<HostPanel>,
    /// Numbers drafts so each gets a stable identity before it has an id from the engine.
    next_draft: u64,
    _subscriptions: Vec<Subscription>,
}

impl RequestsScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| RequestTree::new(model.clone(), window, cx));
        let (dock, skin) = DockSkin::dock_area("requests-dock", Some(1), window, cx);
        let request_panel = cx.new(|cx| {
            HostPanel::new("RequestPanel", "Request", "No request open", "Open a request to edit it here.", cx)
        });
        let response_panel = cx.new(|cx| {
            HostPanel::new("ResponsePanel", "Response", "No response", "Send a request to see its response here.", cx)
        });
        // Dock sizes are physical starting points the user resizes; they are not part of the rem scale.
        dock.update(cx, |area, cx| {
            area.set_center(
                DockLayout::v_split()
                    .child(DockLayout::tabs().panel_view(panel_handle(request_panel.clone()), cx), None)
                    .child(DockLayout::tabs().panel_view(panel_handle(response_panel.clone()), cx), Some(px(320.))),
                window,
                cx,
            );
        });

        let subscriptions = vec![
            cx.subscribe_in(&model, window, Self::on_model_event),
            // The tab's footer shows the request's git state, which the model holds.
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&tree, window, Self::on_tree_event),
        ];
        Self {
            model,
            tree,
            tabs: Vec::new(),
            active: None,
            tab_scroll: VirtualListScrollHandle::new(),
            dock,
            _skin: skin,
            request_panel,
            response_panel,
            next_draft: 1,
            _subscriptions: subscriptions,
        }
    }

    fn on_model_event(&mut self, _: &Entity<AppModel>, event: &ModelEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event {
            ModelEvent::ConfigChanged => self.close_vanished_tabs(cx),
            // Anything open belongs to the workspace that was just left.
            ModelEvent::WorkspaceSwitched => {
                self.tabs.clear();
                self.set_active(None, cx);
            }
            _ => {}
        }
    }

    fn on_tree_event(
        &mut self,
        _: &Entity<RequestTree>,
        event: &RequestTreeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RequestTreeEvent::Open(node) => self.open_node(node, window, cx),
            RequestTreeEvent::NewRequest { kind, folder } => self.new_request(*kind, folder.clone(), window, cx),
            RequestTreeEvent::ImportCurl => self.import_curl(window, cx),
            RequestTreeEvent::NewFolder { parent } => self.prompt_new_folder(parent.clone(), window, cx),
            RequestTreeEvent::Rename(node) => self.prompt_rename(node.clone(), window, cx),
            RequestTreeEvent::Delete(node) => self.confirm_delete(node.clone(), window, cx),
            RequestTreeEvent::Duplicate(node) => self.duplicate(node.clone(), window, cx),
            RequestTreeEvent::RunFolder(folder) => self.open_runner(folder, window, cx),
            RequestTreeEvent::RevertDelete(node) => {
                let target = Self::deleted_target(node);
                self.model.update(cx, |model, cx| model.revert_delete(target, cx));
            }
            RequestTreeEvent::ConfirmDelete(node) => {
                let target = Self::deleted_target(node);
                self.model.update(cx, |model, cx| model.confirm_delete(target, cx));
            }
            RequestTreeEvent::Move { node, folder } => self.move_node(node.clone(), folder.clone(), cx),
        }
    }

    /// Puts a request or folder in another folder. A request that is open is moved by its own next write,
    /// so the tab and the file cannot disagree about where it is.
    fn move_node(&mut self, node: NodeRef, folder: Option<String>, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace_id = model.workspace_id().to_owned();
        match node {
            NodeRef::Folder { id } => {
                let payload = json!({
                    "kind": REQUEST_FOLDER_KIND, "id": id, "parentId": folder, "workspaceId": workspace_id,
                });
                call(&client, cx, "folder.move", payload, |this, outcome, cx| {
                    if let Err(error) = outcome {
                        this.notify_failure(format!("Couldn't move the folder. {}", describe(&error)), cx);
                    }
                });
            }
            NodeRef::Request { kind, id } => {
                let open = self.tabs.iter().find(|tab| tab.key == TabKey::Saved { kind, id: id.clone() });
                if let Some(session) = open.and_then(OpenTab::session).cloned() {
                    return session.set_folder(folder, cx);
                }
                let mut patch = serde_json::Map::new();
                patch.insert("folderId".into(), json!(folder));
                patch_entity(&client, cx, &workspace_id, kind.entity_kind(), &id, patch, |this, outcome, cx| {
                    if let Err(message) = outcome {
                        this.notify_failure(format!("Couldn't move the request. {message}"), cx);
                    }
                });
            }
        }
    }

    /// How the model names a deleted request or folder.
    fn deleted_target(node: &NodeRef) -> Deleted {
        match node {
            NodeRef::Folder { id } => Deleted::Folder { kind: REQUEST_FOLDER_KIND, id: id.clone() },
            NodeRef::Request { kind, id } => Deleted::Entity { kind: kind.entity_kind(), id: id.clone() },
        }
    }

    fn active_tab(&self) -> Option<&OpenTab> {
        self.active.and_then(|ix| self.tabs.get(ix))
    }

    pub fn active_session(&self) -> Option<&Session> {
        self.active_tab().and_then(OpenTab::session)
    }

    /// What is open in the tab in front, for the breadcrumb and the right sidebar.
    pub fn active_tab_target(&self, cx: &App) -> Option<ActiveTarget> {
        let model = self.model.read(cx);
        match &self.active_tab()?.content {
            TabContent::Request(session) => Some(ActiveTarget {
                title: session.title(cx),
                entity_id: session.saved_id(cx),
                request: session.request_parts(cx),
                name: Some(session.name_input(cx)),
                kind: Some(session.kind(cx)),
                folders: session.folder_trail(model, cx),
            }),
            TabContent::Runner(runner) => Some(ActiveTarget {
                title: "Runner".into(),
                entity_id: None,
                request: None,
                name: None,
                kind: None,
                folders: folder_path(&model.config().request_folder_rows(), runner.read(cx).folder_id()),
            }),
            TabContent::Curl(_) => Some(ActiveTarget {
                title: CURL_TAB_TITLE.into(),
                entity_id: None,
                request: None,
                name: None,
                kind: None,
                folders: Vec::new(),
            }),
        }
    }

    /// Selects a request in the tree, opening the folders above it.
    pub fn reveal(&mut self, node: &NodeRef, cx: &mut Context<Self>) {
        self.tree.update(cx, |tree, cx| tree.reveal(node, cx));
    }

    /// Brings a tab to the front and points the dock panels at it.
    fn set_active(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        self.active = ix.filter(|ix| *ix < self.tabs.len());
        if let Some(ix) = self.active {
            self.tab_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        let session = self.active_session().cloned();
        let editor = session.as_ref().map(Session::editor_view);
        let response = session.as_ref().map(|s| s.response_view(cx));
        self.request_panel.update(cx, |panel, cx| panel.set_view(editor, cx));
        self.response_panel.update(cx, |panel, cx| panel.set_view(response, cx));
        cx.notify();
    }

    pub fn open_node(&mut self, node: &NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let NodeRef::Request { kind, id } = node else { return };
        // A deleted request has nothing to open: it is brought back or deleted for good.
        if is_deleted(self.model.read(cx), node) {
            let name = self.name_of(node, cx);
            resolve_deleted(self.model.clone(), Self::deleted_target(node), name, window, cx);
            return;
        }
        let key = TabKey::Saved { kind: *kind, id: id.clone() };
        if let Some(ix) = self.tabs.iter().position(|tab| tab.key == key) {
            self.set_active(Some(ix), cx);
            return;
        }
        let model = self.model.clone();
        let (kind, id) = (*kind, id.clone());
        let session = match kind {
            RequestKind::Rest => Session::Rest(cx.new(|cx| RestSession::open_saved(model, id, window, cx))),
            RequestKind::WebSocket => Session::WebSocket(cx.new(|cx| WsSession::open_saved(model, id, window, cx))),
            RequestKind::Sse => Session::Sse(cx.new(|cx| SseSession::open_saved(model, id, window, cx))),
            RequestKind::SocketIo => {
                Session::SocketIo(cx.new(|cx| SocketIoSession::open_saved(model, id, window, cx)))
            }
            RequestKind::Mcp => Session::Mcp(cx.new(|cx| McpSession::open_saved(model, id, window, cx))),
            RequestKind::GraphQl | RequestKind::Grpc | RequestKind::Soap => {
                Session::Protocol(cx.new(|cx| ProtocolSession::open_saved(model, kind, id, window, cx)))
            }
        };
        self.add_tab(key, session, window, cx);
    }

    /// Opens the folder with this id as a tab that runs its REST requests in order.
    pub fn open_runner(&mut self, folder_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let key = TabKey::Runner(folder_id.to_owned());
        if let Some(ix) = self.tabs.iter().position(|tab| tab.key == key) {
            self.set_active(Some(ix), cx);
            return;
        }
        let (model, folder_id) = (self.model.clone(), folder_id.to_owned());
        let runner = cx.new(|cx| FolderRunner::new(model, folder_id, cx));
        let subscription = cx.subscribe_in(&runner, window, |this, _, event: &RunnerEvent, window, cx| match event {
            RunnerEvent::Open(node) => {
                this.open_node(node, window, cx);
                this.reveal(node, cx);
            }
            RunnerEvent::Changed => cx.notify(),
        });
        self.tabs.push(OpenTab { key, content: TabContent::Runner(runner), _subscription: subscription });
        self.set_active(Some(self.tabs.len() - 1), cx);
    }

    fn draft(&self, kind: RequestKind, folder: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Session {
        let model = self.model.clone();
        match kind {
            RequestKind::Rest => Session::Rest(cx.new(|cx| RestSession::new_draft(model, folder, window, cx))),
            RequestKind::WebSocket => Session::WebSocket(cx.new(|cx| WsSession::new_draft(model, folder, window, cx))),
            RequestKind::Sse => Session::Sse(cx.new(|cx| SseSession::new_draft(model, folder, window, cx))),
            RequestKind::SocketIo => {
                Session::SocketIo(cx.new(|cx| SocketIoSession::new_draft(model, folder, window, cx)))
            }
            RequestKind::Mcp => Session::Mcp(cx.new(|cx| McpSession::new_draft(model, folder, window, cx))),
            RequestKind::GraphQl | RequestKind::Grpc | RequestKind::Soap => {
                Session::Protocol(cx.new(|cx| ProtocolSession::new_draft(model, kind, folder, window, cx)))
            }
        }
    }

    /// Starts an unsaved request of `kind`, to be created in `folder` when it is saved.
    pub fn new_request(
        &mut self,
        kind: RequestKind,
        folder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.draft(kind, folder, window, cx);
        let key = TabKey::Draft(self.next_draft);
        self.next_draft += 1;
        self.add_tab(key, session, window, cx);
    }

    /// Opens a tab that asks for a cURL command. The engine reads the command, and the tab becomes a draft
    /// of the request type it describes, filled in from it.
    pub fn import_curl(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let import = cx.new(|cx| CurlImport::new(model, CurlTarget::Request, window, cx));
        let subscription = cx.subscribe_in(&import, window, |this, import, event: &CurlImportEvent, window, cx| {
            let CurlImportEvent::Built(built) = event;
            this.adopt_curl(import, built, window, cx);
        });
        let key = TabKey::Draft(self.next_draft);
        self.next_draft += 1;
        self.tabs.push(OpenTab { key, content: TabContent::Curl(import.clone()), _subscription: subscription });
        self.set_active(Some(self.tabs.len() - 1), cx);
        import.update(cx, |import, cx| import.focus(window, cx));
    }

    /// Turns the tab `import` fills into a draft of what the engine built from its command: `{ kind, request }`.
    fn adopt_curl(&mut self, import: &Entity<CurlImport>, built: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let is_tab = |tab: &OpenTab| matches!(&tab.content, TabContent::Curl(open) if open == import);
        let Some(ix) = self.tabs.iter().position(is_tab) else { return };
        let kind = built.get("kind").and_then(Value::as_str).and_then(RequestKind::from_entity_kind);
        let entity = built.get("request").and_then(Value::as_object).cloned();
        let (Some(kind), Some(entity)) = (kind, entity) else {
            self.notify_failure("The engine did not return a request for this cURL command.".to_owned(), cx);
            return;
        };
        let session = self.draft(kind, None, window, cx);
        session.adopt_import(entity, window, cx);
        let subscription = session.subscribe_in(window, cx, Self::on_session_event);
        let tab = &mut self.tabs[ix];
        tab.content = TabContent::Request(session);
        tab._subscription = subscription;
        // The docks follow the tab in front, which this one may no longer be.
        self.set_active(self.active, cx);
    }

    /// Makes the active draft a request of another type. A request's type is fixed once it is saved, so
    /// this only applies to a draft: the tab is rebuilt as the new type and keeps what the two types share
    /// (the name, the address, the headers, the body).
    pub fn change_active_kind(&mut self, kind: RequestKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        let Some(tab) = self.tabs.get(ix) else { return };
        let Some(current) = tab.session() else { return };
        if !matches!(tab.key, TabKey::Draft(_)) || current.kind(cx) == kind {
            return;
        }
        let carry = current.carry(cx);
        let folder = current.folder_id(cx);
        let session = self.draft(kind, folder, window, cx);
        session.apply_carry(&carry, window, cx);
        let subscription = session.subscribe_in(window, cx, Self::on_session_event);
        let tab = &mut self.tabs[ix];
        tab.content = TabContent::Request(session);
        tab._subscription = subscription;
        self.set_active(Some(ix), cx);
    }

    fn add_tab(&mut self, key: TabKey, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        let subscription = session.subscribe_in(window, cx, Self::on_session_event);
        self.tabs.push(OpenTab { key, content: TabContent::Request(session), _subscription: subscription });
        self.set_active(Some(self.tabs.len() - 1), cx);
    }

    fn position_of(&self, session: &Session) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.session().is_some_and(|open| open.same_as(session)))
    }

    fn on_session_event(
        &mut self,
        session: &Session,
        event: &SessionEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        match event {
            // The tab strip shows the title and the unsaved mark, which live in the session.
            SessionEvent::Changed => cx.notify(),
            SessionEvent::Saved { id } => {
                if let Some(ix) = self.position_of(session) {
                    let kind = session.kind(cx);
                    self.tabs[ix].key = TabKey::Saved { kind, id: id.clone() };
                    let node = NodeRef::Request { kind, id: id.clone() };
                    self.tree.update(cx, |tree, cx| tree.reveal(&node, cx));
                }
                cx.notify();
            }
            SessionEvent::Notice(text) => window.push_notification(Notification::warning(text.clone()), cx),
        }
    }

    fn remove_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        // The tab that takes the closed one's place comes to the front, or the last one if it was at the end.
        let next = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > ix => Some(active - 1),
            Some(active) => Some(active.min(self.tabs.len() - 1)),
            None => None,
        };
        self.set_active(next, cx);
    }

    /// Closes a tab. A saved request has nothing to lose: its last edit is written on the way out. A draft
    /// that was typed into is asked about first, because closing it discards it.
    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if !tab.is_unsaved_draft(cx) {
            self.close_keys(vec![tab.key.clone()], window, cx);
            return;
        }
        let key = tab.key.clone();
        let title = tab.session().map(|session| session.title(cx)).unwrap_or_default();
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Discard “{title}”?"),
            "This request has not been saved. Closing its tab discards it.",
            "Discard",
            move |this, window, cx| this.close_keys(vec![key.clone()], window, cx),
        );
    }

    /// Closes every tab `closes` picks. Drafts that were typed into are asked about once, together.
    fn close_where(&mut self, closes: impl Fn(usize) -> bool, window: &mut Window, cx: &mut Context<Self>) {
        let picked = || self.tabs.iter().enumerate().filter(|(ix, _)| closes(*ix)).map(|(_, tab)| tab);
        let keys: Vec<TabKey> = picked().map(|tab| tab.key.clone()).collect();
        let unsaved = picked().filter(|tab| tab.is_unsaved_draft(cx)).count();
        if unsaved == 0 {
            self.close_keys(keys, window, cx);
            return;
        }
        let what = if unsaved == 1 { "1 unsaved request".to_owned() } else { format!("{unsaved} unsaved requests") };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Discard {what}?"),
            "Requests that have not been saved are discarded when their tabs close.",
            "Discard",
            move |this, window, cx| this.close_keys(keys.clone(), window, cx),
        );
    }

    /// Closes the tabs with these keys, writing a saved request's waiting edit on the way out. The tab in
    /// front stays in front when it is not one of them.
    fn close_keys(&mut self, keys: Vec<TabKey>, window: &mut Window, cx: &mut Context<Self>) {
        let front = self.active_tab().map(|tab| tab.key.clone());
        let position = self.active.unwrap_or(0);
        for tab in self.tabs.iter().filter(|tab| keys.contains(&tab.key)) {
            if let (TabKey::Saved { .. }, Some(session)) = (&tab.key, tab.session()) {
                session.clone().flush(window, cx);
            }
        }
        self.tabs.retain(|tab| !keys.contains(&tab.key));
        let next = match front.and_then(|key| self.tabs.iter().position(|tab| tab.key == key)) {
            Some(ix) => Some(ix),
            None if self.tabs.is_empty() => None,
            // The tab that takes the closed one's place comes to the front, or the last one.
            None => Some(position.min(self.tabs.len() - 1)),
        };
        self.set_active(next, cx);
    }

    /// What a tab asks for, by its close button or its menu.
    fn tab_action(&mut self, action: TabAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            TabAction::Select(ix) => self.set_active(Some(ix), cx),
            TabAction::Close(ix) => self.close_tab(ix, window, cx),
            TabAction::CloseOthers(keep) => self.close_where(|ix| ix != keep, window, cx),
            TabAction::CloseToRight(of) => self.close_where(|ix| ix > of, window, cx),
            TabAction::CloseAll => self.close_where(|_| true, window, cx),
            TabAction::Duplicate(ix) => {
                if let Some(TabKey::Saved { kind, id }) = self.tabs.get(ix).map(|tab| tab.key.clone()) {
                    self.duplicate(NodeRef::Request { kind, id }, window, cx);
                }
            }
        }
    }

    fn describe_tab(&self, ix: usize, cx: &App) -> Option<StripTab> {
        let tab = self.tabs.get(ix)?;
        let id = match &tab.key {
            TabKey::Saved { kind, id } => format!("{}-{id}", kind.entity_kind()),
            TabKey::Draft(number) => format!("draft-{number}"),
            TabKey::Runner(folder) => format!("runner-{folder}"),
        };
        Some(match &tab.content {
            TabContent::Request(session) => StripTab {
                id: id.into(),
                badge: session.badge(cx).into(),
                title: session.title(cx),
                dirty: session.is_dirty(cx),
                can_duplicate: matches!(tab.key, TabKey::Saved { .. }),
            },
            TabContent::Runner(runner) => StripTab {
                id: id.into(),
                badge: "RUN".into(),
                title: runner.read(cx).title(cx),
                dirty: false,
                can_duplicate: false,
            },
            TabContent::Curl(_) => StripTab {
                id: id.into(),
                badge: CURL_TAB_BADGE.into(),
                title: CURL_TAB_TITLE.into(),
                dirty: false,
                can_duplicate: false,
            },
        })
    }

    /// Copies a saved request. The engine makes the copy, next to the original; it is then opened.
    fn duplicate(&mut self, node: NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let NodeRef::Request { kind, id } = node else { return };
        // An edit still waiting to be written belongs in the copy too.
        let open = self.tabs.iter().find(|tab| tab.key == TabKey::Saved { kind, id: id.clone() });
        if let Some(session) = open.and_then(OpenTab::session).cloned() {
            session.flush(window, cx);
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "kind": kind.entity_kind(), "id": id });
        call_in(&client, window, cx, "entity.duplicate", payload, move |this, outcome, window, cx| {
            let copy = match &outcome {
                Ok(result) => result.get("entity").and_then(|entity| entity.get("id")).and_then(Value::as_str),
                Err(error) => {
                    return this.notify_failure(format!("Couldn't duplicate the request. {}", describe(error)), cx);
                }
            };
            let Some(copy) = copy else { return };
            let node = NodeRef::Request { kind, id: copy.to_owned() };
            // The copy is in the engine but not yet in this window's snapshot, which the tree is built from.
            this.model.update(cx, |model, cx| model.refresh(cx));
            this.open_node(&node, window, cx);
            this.reveal(&node, cx);
        });
    }

    /// What the active tab's footer asks for, and the keys that ask for the same things.
    pub fn entity_action(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        let Some(session) = self.active_session().cloned() else {
            // A runner is not an entity: the only thing to do with its tab is close it.
            if action == EntityAction::Close {
                self.close_tab(ix, window, cx);
            }
            return;
        };
        let saved = session.saved_id(cx);
        match (action, saved) {
            (EntityAction::Save, _) => session.save(SaveMode::Explicit, window, cx),
            (EntityAction::Move(folder), _) => session.set_folder(folder, cx),
            (EntityAction::Close, _) => self.close_tab(ix, window, cx),
            (EntityAction::Sync, Some(id)) => self.model.update(cx, |model, cx| model.sync_entity(&id, cx)),
            (EntityAction::Revert, Some(id)) => {
                let title = session.title(cx);
                confirm_danger(
                    cx.weak_entity(),
                    window,
                    cx,
                    format!("Revert “{title}” to the last synced version?"),
                    "The changes saved since then are discarded. This can't be undone.",
                    "Revert",
                    move |this, _, cx| this.model.update(cx, |model, cx| model.revert_entity(&id, cx)),
                );
            }
            (EntityAction::Delete, Some(id)) => {
                self.confirm_delete(NodeRef::Request { kind: session.kind(cx), id }, window, cx)
            }
            // A draft is stored nowhere: discarding it is closing its tab.
            (EntityAction::Delete, None) => self.close_tab(ix, window, cx),
            (EntityAction::Sync | EntityAction::Revert, None) => {}
        }
    }

    /// A saved request that is gone from the workspace (deleted here, elsewhere, or with its folder) has
    /// nothing left to show, and nothing an edit could be written to. The same goes for the runner of a
    /// folder that is gone.
    fn close_vanished_tabs(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if !matches!(model.load(), Load::Ready) {
            return;
        }
        let config = model.config();
        let gone: Vec<TabKey> = self
            .tabs
            .iter()
            .filter(|tab| match &tab.key {
                // Deleted and still listed counts as gone: there is no file an edit could be written to.
                TabKey::Saved { kind, id } => config.request(*kind, id).is_none_or(|r| r.deleted),
                TabKey::Runner(folder) => config.folder(folder).is_none_or(|f| f.deleted),
                TabKey::Draft(_) => false,
            })
            .map(|tab| tab.key.clone())
            .collect();
        for key in gone {
            if let Some(ix) = self.tabs.iter().position(|tab| tab.key == key) {
                self.remove_tab(ix, cx);
            }
        }
    }

    /// Send, connect, or run: the active tab's main verb.
    pub fn run_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active_tab().map(|tab| &tab.content) {
            Some(TabContent::Request(session)) => session.clone().run(window, cx),
            Some(TabContent::Runner(runner)) => runner.clone().update(cx, |runner, cx| runner.run(cx)),
            Some(TabContent::Curl(import)) => import.clone().update(cx, |import, cx| import.submit(cx)),
            None => {}
        }
    }

    fn has_open_tab(&self, kind: RequestKind, id: &str) -> bool {
        self.tabs.iter().any(|tab| tab.key == TabKey::Saved { kind, id: id.to_owned() })
    }

    fn notify_failure(&mut self, text: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(text, cx));
    }

    /// Asks for the name of a new folder in `parent`, or at the top of the tree. It takes requests of every
    /// type.
    pub fn prompt_new_folder(&mut self, parent: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            "New folder",
            "Create",
            vec![PromptField::new("Name", "Folder name")],
            move |this, values, _, cx| {
                let client = this.model.read(cx).client().clone();
                let payload = json!({
                    "kind": REQUEST_FOLDER_KIND,
                    "name": values[0],
                    "parentId": parent,
                    "workspaceId": this.model.read(cx).workspace_id(),
                });
                call(&client, cx, "folder.add", payload, |this, outcome, cx| {
                    if let Err(error) = outcome {
                        this.notify_failure(format!("Couldn't create the folder. {}", describe(&error)), cx);
                    }
                });
            },
        );
    }

    fn name_of(&self, node: &NodeRef, cx: &Context<Self>) -> String {
        let config = self.model.read(cx).config();
        match node {
            NodeRef::Folder { id } => config.folder(id).map(|f| f.name.clone()),
            NodeRef::Request { kind, id } => config.request(*kind, id).map(|r| r.name.clone()),
        }
        .unwrap_or_default()
    }

    fn prompt_rename(&mut self, node: NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        if let NodeRef::Request { kind, id } = &node
            && self.has_open_tab(*kind, id)
        {
            window.push_notification(
                Notification::info("This request is open. Change its name in the breadcrumb above the tabs.")
                    .title("Rename it where it is open"),
                cx,
            );
            return;
        }
        let current = self.name_of(&node, cx);
        prompt(
            cx.weak_entity(),
            window,
            cx,
            format!("Rename “{current}”"),
            "Rename",
            vec![PromptField::new("Name", "Name").initial(current.clone())],
            move |this, values, _, cx| {
                let client = this.model.read(cx).client().clone();
                match node.clone() {
                    NodeRef::Folder { id } => {
                        let payload = json!({ "kind": REQUEST_FOLDER_KIND, "id": id, "name": values[0] });
                        call(&client, cx, "folder.rename", payload, |this, outcome, cx| {
                            if let Err(error) = outcome {
                                this.notify_failure(format!("Couldn't rename the folder. {}", describe(&error)), cx);
                            }
                        });
                    }
                    NodeRef::Request { kind, id } => this.rename_request(kind, id, values[0].clone(), cx),
                }
            },
        );
    }

    /// Renames a saved request: read it, change the name, write it back. The change is one field of the
    /// entity the engine returned, so nothing else about the request is touched.
    fn rename_request(&mut self, kind: RequestKind, id: String, name: String, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let client = model.client().clone();
        let load = json!({ "workspaceId": model.workspace_id(), "kind": kind.entity_kind(), "id": id });
        call(&client.clone(), cx, "entity.load", load, move |this, outcome, cx| {
            let entity = outcome.ok().and_then(|v| v.get("entity").cloned()).and_then(|e| match e {
                Value::Object(mut map) => {
                    map.insert("name".into(), json!(name));
                    Some(Value::Object(map))
                }
                _ => None,
            });
            let Some(entity) = entity else {
                this.notify_failure("Couldn't rename the request: it could not be read.".to_owned(), cx);
                return;
            };
            let update = json!({ "kind": kind.entity_kind(), "entity": entity });
            call(&client, cx, "entity.update", update, |this, outcome, cx| {
                if let Err(error) = outcome {
                    this.notify_failure(format!("Couldn't rename the request. {}", describe(&error)), cx);
                }
            });
        });
    }

    fn confirm_delete(&mut self, node: NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_of(&node, cx);
        let (what, consequence) = match &node {
            NodeRef::Folder { .. } => ("folder", FOLDER_DELETE_CONSEQUENCE),
            NodeRef::Request { id, .. } => ("request", delete_consequence(self.model.read(cx).git().entity_state(id))),
        };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Delete {what} “{name}”?"),
            consequence,
            "Delete",
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                let (action, payload) = match &node {
                    NodeRef::Folder { id } => ("folder.delete", json!({ "kind": REQUEST_FOLDER_KIND, "id": id })),
                    NodeRef::Request { kind, id } => ("entity.delete", json!({ "kind": kind.entity_kind(), "id": id })),
                };
                call(&client, cx, action, payload, |this, outcome, cx| {
                    if let Err(error) = outcome {
                        this.notify_failure(format!("Couldn't delete. {}", describe(&error)), cx);
                    }
                });
            },
        );
    }

    fn render_tab_strip(&self, tree_open: bool, window: &Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let tree = self.tree.downgrade();
        let strip = TabStrip {
            id: "request-tabs",
            count: self.tabs.len(),
            active: self.active,
            scroll: self.tab_scroll.clone(),
            describe: Self::describe_tab,
            on_action: Self::tab_action,
        };
        let prefix = h_flex().flex_none().px_1().child(
            Button::new("toggle-folder-tree")
                .icon(if tree_open { IconName::FolderOpen } else { IconName::Folder })
                .ghost()
                .small()
                .selected(tree_open)
                .tooltip(if tree_open { "Hide the folder tree" } else { "Show the folder tree" })
                .on_click(cx.listener(|_, _, _, cx| {
                    Preferences::update(cx, |p| p.folder_tree_open = !p.folder_tree_open);
                    cx.notify();
                })),
        );
        let suffix = h_flex().flex_none().px_1().child(
            Button::new("new-tab")
                .icon(IconName::Plus)
                .ghost()
                .small()
                .tooltip("New request")
                .dropdown_menu(move |menu, window, cx| new_menu(menu, &tree, None, window, cx)),
        );
        tab_strip(self, strip, prefix, suffix, window, cx)
    }

    /// With nothing open: everything that can be started here, one row per request type.
    fn render_welcome(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let theme = cx.theme().clone();
        let rows = RequestKind::ALL.into_iter().enumerate().map(|(ix, kind)| {
            Button::new(("welcome-new", ix))
                .ghost()
                .w_full()
                .child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(div().w_12().flex_none().child(Tag::secondary().small().child(kind.badge())))
                        .child(div().flex_none().child(format!("New {} request", kind.label())))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_left()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(kind_summary(kind)),
                        ),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.new_request(kind, None, window, cx)))
        });
        let import = Button::new("welcome-import-curl")
            .ghost()
            .w_full()
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .child(div().w_12().flex_none().child(Tag::secondary().small().child(CURL_TAB_BADGE)))
                    .child(div().flex_none().child(CURL_TAB_TITLE))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_left()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("A request of the type a pasted cURL command describes"),
                    ),
            )
            .on_click(cx.listener(|this, _, window, cx| this.import_curl(window, cx)));
        // Wider than the kit's empty state, which is sized for a sentence and one button.
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(rems(40.))
                    .max_w_full()
                    .px_4()
                    .gap_4()
                    .child(
                        v_flex()
                            .items_center()
                            .gap_1()
                            .child(div().font_semibold().child("No request open"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child("Open one from the folder tree, or start a new one."),
                            ),
                    )
                    .child(v_flex().gap_1().children(rows).child(import)),
            )
            .into_any_element()
    }
}

impl Render for RequestsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree_open = Preferences::global(cx).folder_tree_open;
        let body = match self.active_tab().map(|tab| &tab.content) {
            None => self.render_welcome(cx),
            Some(TabContent::Request(_)) => self.dock.clone().into_any_element(),
            Some(TabContent::Runner(runner)) => runner.clone().into_any_element(),
            Some(TabContent::Curl(import)) => import.clone().into_any_element(),
        };
        // The footer belongs to the tab, not to a dock panel: it stays put however the panels are arranged.
        let footer = self.active_session().map(|session| session.bar(self.model.read(cx), cx)).map(|bar| {
            entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)).into_any_element()
        });
        let main = v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_tab_strip(tree_open, window, cx))
            .child(div().flex_1().min_h_0().child(body))
            .children(footer);
        // The split's sizes are physical starting points the user resizes, not part of the rem scale.
        h_resizable("requests-split")
            .child(
                resizable_panel()
                    .visible(tree_open)
                    .size(px(260.))
                    .size_range(px(180.)..px(520.))
                    .child(self.tree.clone()),
            )
            .child(resizable_panel().child(main))
    }
}
