//! The Requests screen: the folder tree, the open requests as tabs, and the active request's editor and
//! response as docks.
//!
//! The screen owns which requests are open and which is in front. It holds one dock area, not one per tab:
//! its "Request" and "Response" panels stay where the user put them and follow the active tab. An open
//! request is a [`Session`]; the workspace comes from [`AppModel`]; everything that changes a saved request
//! is an engine command.

mod tree;

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    dock::{DockArea, DockLayout, DockSkin, panel_handle},
    h_flex,
    menu::DropdownMenu as _,
    resizable::{h_resizable, resizable_panel},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Window, div, px,
};
use serde_json::{Value, json};

use self::tree::{RequestTree, RequestTreeEvent, new_menu};
use crate::{
    app_model::{AppModel, Load, ModelEvent},
    components::{
        dialogs::{PromptField, confirm_danger, prompt},
        page,
    },
    engine::{call, describe},
    model::{NodeRef, RequestKind},
    panels::host::HostPanel,
    preferences::Preferences,
    session::{Session, SessionEvent, protocol::ProtocolSession, rest::RestSession, websocket::WsSession},
};

/// How a tab is identified before and after it has an id from the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TabKey {
    Saved { kind: RequestKind, id: String },
    Draft(u64),
}

struct OpenTab {
    key: TabKey,
    session: Session,
    _subscription: Subscription,
}

pub struct RequestsScreen {
    model: Entity<AppModel>,
    tree: Entity<RequestTree>,
    tabs: Vec<OpenTab>,
    active: Option<usize>,
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
            cx.subscribe_in(&tree, window, Self::on_tree_event),
        ];
        Self {
            model,
            tree,
            tabs: Vec::new(),
            active: None,
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
            RequestTreeEvent::NewFolder { kind, parent } => self.prompt_new_folder(*kind, parent.clone(), window, cx),
            RequestTreeEvent::Rename(node) => self.prompt_rename(node.clone(), window, cx),
            RequestTreeEvent::Delete(node) => self.confirm_delete(node.clone(), window, cx),
        }
    }

    pub fn active_session(&self) -> Option<&Session> {
        self.active.and_then(|ix| self.tabs.get(ix)).map(|tab| &tab.session)
    }

    /// Selects a request in the tree, opening the folders above it.
    pub fn reveal(&mut self, node: &NodeRef, cx: &mut Context<Self>) {
        self.tree.update(cx, |tree, cx| tree.reveal(node, cx));
    }

    /// Brings a tab to the front and points the dock panels at it.
    fn set_active(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        self.active = ix.filter(|ix| *ix < self.tabs.len());
        let session = self.active_session().cloned();
        let editor = session.as_ref().map(Session::editor_view);
        let response = session.as_ref().map(|s| s.response_view(cx));
        self.request_panel.update(cx, |panel, cx| panel.set_view(editor, cx));
        self.response_panel.update(cx, |panel, cx| panel.set_view(response, cx));
        cx.notify();
    }

    pub fn open_node(&mut self, node: &NodeRef, window: &mut Window, cx: &mut Context<Self>) {
        let NodeRef::Request { kind, id } = node else { return };
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
            RequestKind::GraphQl | RequestKind::Grpc | RequestKind::Soap => {
                Session::Protocol(cx.new(|cx| ProtocolSession::open_saved(model, kind, id, window, cx)))
            }
        };
        self.add_tab(key, session, window, cx);
    }

    /// Starts an unsaved request of `kind`, to be created in `folder` when it is saved.
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
                Session::Protocol(cx.new(|cx| ProtocolSession::new_draft(model, kind, folder, window, cx)))
            }
        };
        let key = TabKey::Draft(self.next_draft);
        self.next_draft += 1;
        self.add_tab(key, session, window, cx);
    }

    fn add_tab(&mut self, key: TabKey, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        let subscription = session.subscribe_in(window, cx, Self::on_session_event);
        self.tabs.push(OpenTab { key, session, _subscription: subscription });
        self.set_active(Some(self.tabs.len() - 1), cx);
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
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.session.same_as(session)) {
                    let kind = session.kind(cx);
                    tab.key = TabKey::Saved { kind, id: id.clone() };
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

    /// Closes a tab, asking first if it holds edits that would be lost.
    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if !tab.session.is_dirty(cx) {
            self.remove_tab(ix, cx);
            return;
        }
        let session = tab.session.clone();
        let title = session.title(cx);
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Discard changes to “{title}”?"),
            "The edits in this tab have not been saved.",
            "Discard changes",
            move |this, _, cx| {
                if let Some(ix) = this.tabs.iter().position(|tab| tab.session.same_as(&session)) {
                    this.remove_tab(ix, cx);
                }
            },
        );
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.close_tab(ix, window, cx);
        }
    }

    /// A saved request that is gone from the workspace (deleted here, elsewhere, or with its folder) has
    /// nothing left to show. A tab with unsaved edits stays: saving it creates the request again.
    fn close_vanished_tabs(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if !matches!(model.load(), Load::Ready) {
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
            if let Some(ix) = self.tabs.iter().position(|tab| tab.session.same_as(&session)) {
                self.remove_tab(ix, cx);
            }
        }
    }

    pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.active_session().cloned() {
            session.save(window, cx);
        }
    }

    pub fn revert_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.active_session().cloned() {
            session.revert(window, cx);
        }
    }

    /// Send, or connect: the active request's main verb.
    pub fn run_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.active_session().cloned() {
            session.run(window, cx);
        }
    }

    fn has_open_tab(&self, kind: RequestKind, id: &str) -> bool {
        self.tabs.iter().any(|tab| tab.key == TabKey::Saved { kind, id: id.to_owned() })
    }

    fn notify_failure(&mut self, text: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(text, cx));
    }

    pub fn prompt_new_folder(
        &mut self,
        kind: RequestKind,
        parent: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            format!("New {} folder", kind.label()),
            "Create",
            vec![PromptField::new("Name", "Folder name")],
            move |this, values, _, cx| {
                let client = this.model.read(cx).client().clone();
                let payload = json!({
                    "kind": kind.folder_kind(),
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
            NodeRef::Folder { kind, id } => config.folder(*kind, id).map(|f| f.name.clone()),
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
                Notification::info("This request is open. Change its name in the tab and save.")
                    .title("Rename in the tab"),
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
                    NodeRef::Folder { kind, id } => {
                        let payload = json!({ "kind": kind.folder_kind(), "id": id, "name": values[0] });
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
            NodeRef::Folder { .. } => ("folder", "The folder and everything in it will be deleted."),
            NodeRef::Request { .. } => ("request", "This can't be undone."),
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
                    NodeRef::Folder { kind, id } => ("folder.delete", json!({ "kind": kind.folder_kind(), "id": id })),
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

    fn render_tab_strip(&self, tree_open: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let tree = self.tree.downgrade();
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let title = tab.session.title(cx);
            let label: SharedString = if tab.session.is_dirty(cx) { format!("{title} ●").into() } else { title };
            Tab::new()
                .label(label)
                .prefix(div().pl_2().text_xs().text_color(theme.muted_foreground).child(tab.session.badge(cx)))
                .suffix(div().pr_1().child(
                    Button::new(("close-tab", ix)).icon(IconName::Close).ghost().xsmall().on_click(cx.listener(
                        move |this, _, window, cx| {
                            // The tab under the button would otherwise take the click as a selection.
                            cx.stop_propagation();
                            this.close_tab(ix, window, cx);
                        },
                    )),
                ))
        });
        TabBar::new("request-tabs")
            .small()
            .menu(true)
            .selected_index(self.active.unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, _, cx| this.set_active(Some(*ix), cx)))
            .prefix(
                h_flex().px_1().child(
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
                ),
            )
            .children(tabs)
            .suffix(
                h_flex().px_1().child(
                    Button::new("new-tab")
                        .icon(IconName::Plus)
                        .ghost()
                        .small()
                        .tooltip("New request")
                        .dropdown_menu(move |menu, window, cx| new_menu(menu, &tree, None, window, cx)),
                ),
            )
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        page::empty_with_action(
            "No request open",
            "Open one from the folder tree, or start a new one.",
            h_flex()
                .gap_2()
                .child(
                    Button::new("welcome-new-rest").label("New REST request").on_click(
                        cx.listener(|this, _, window, cx| this.new_request(RequestKind::Rest, None, window, cx)),
                    ),
                )
                .child(Button::new("welcome-new-ws").label("New WebSocket").on_click(
                    cx.listener(|this, _, window, cx| this.new_request(RequestKind::WebSocket, None, window, cx)),
                )),
        )
    }
}

impl Render for RequestsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree_open = Preferences::global(cx).folder_tree_open;
        let body = if self.tabs.is_empty() { self.render_welcome(cx) } else { self.dock.clone().into_any_element() };
        let main = v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_tab_strip(tree_open, cx))
            .child(div().flex_1().min_h_0().child(body));
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
