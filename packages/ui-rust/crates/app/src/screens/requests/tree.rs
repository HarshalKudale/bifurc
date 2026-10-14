//! The folder tree of the Requests screen: every saved request of every type, in its folders.
//!
//! It owns only view state — what is expanded, selected, and typed into the filter. What the tree shows
//! comes from the model's snapshot, and everything the user asks for (open, rename, delete, create) leaves
//! as a [`RequestTreeEvent`] for the screen, which turns it into engine commands. After a command the engine
//! says what changed and the tree rebuilds.

use std::collections::HashSet;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, WeakEntity, Window, div, rems,
};

use crate::{
    app_model::AppModel,
    components::{CURL_TAB_TITLE, page, tree_drag},
    i18n::{t, t_with},
    model::{
        ExplorerNode, NodeRef, RequestKind, build_explorer,
        git::FileState,
        tree::{Moved, can_move},
    },
};

/// What the user asked for in the tree. The screen acts on it.
#[derive(Debug, Clone)]
pub enum RequestTreeEvent {
    Open(NodeRef),
    NewRequest { kind: RequestKind, folder: Option<String> },
    /// Start a request from a cURL command; its type comes from the command.
    ImportCurl,
    NewFolder { parent: Option<String> },
    Rename(NodeRef),
    Delete(NodeRef),
    /// Copy the request, next to the original.
    Duplicate(NodeRef),
    /// Open the folder with this id as a runner tab.
    RunFolder(String),
    /// Bring back a deleted request or folder.
    RevertDelete(NodeRef),
    /// Sync the deletion of a deleted request or folder, removing it for good.
    ConfirmDelete(NodeRef),
    /// Put the request or folder in another folder; a folder goes to the top level with `None`.
    Move { node: NodeRef, folder: Option<String> },
}

/// Whether the request or folder is deleted here, with the deletion not confirmed or reverted yet.
pub fn is_deleted(model: &AppModel, node: &NodeRef) -> bool {
    let config = model.config();
    match node {
        NodeRef::Folder { id } => config.folder(id).is_some_and(|f| f.deleted),
        NodeRef::Request { kind, id } => config.request(*kind, id).is_some_and(|r| r.deleted),
    }
}

/// The version of Bifurc the request needs, when a newer one than this last saved it. A folder is never
/// locked: it is a directory, and moving or renaming it changes nothing in it.
pub fn locked_to(model: &AppModel, node: &NodeRef) -> Option<String> {
    match node {
        NodeRef::Folder { .. } => None,
        NodeRef::Request { kind, id } => model.config().request(*kind, id).and_then(|r| r.locked.clone()),
    }
}

/// The folder a drop on this row goes to: the folder itself, or the folder the request is in.
fn drop_folder(model: &AppModel, node: &NodeRef) -> Option<String> {
    match node {
        NodeRef::Folder { id } => Some(id.clone()),
        NodeRef::Request { kind, id } => model.config().request(*kind, id).and_then(|r| r.folder_id.clone()),
    }
}

/// Whether the dragged request or folder can be put in `folder`, or at the top level with `None`.
fn accepts(model: &AppModel, dragged: &NodeRef, folder: Option<&str>) -> bool {
    let config = model.config();
    let moved = match dragged {
        NodeRef::Folder { id } => Moved::Folder(id),
        NodeRef::Request { kind, id } => match config.request(*kind, id) {
            Some(request) => Moved::Item { folder: request.folder_id.as_deref() },
            None => return false,
        },
    };
    can_move(&config.request_folder_rows(), moved, folder)
}

pub struct RequestTree {
    model: Entity<AppModel>,
    filter: Entity<InputState>,
    tree: Entity<TreeState>,
    /// Folders the user has opened, by tree id, so a rebuild does not collapse them.
    expanded: HashSet<String>,
    selected: Option<String>,
    /// The row being dragged, while a drag is on.
    dragging: Option<NodeRef>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RequestTreeEvent> for RequestTree {}

impl RequestTree {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t("requests.tree.filter_placeholder")));
        let tree = cx.new(|cx| TreeState::new(cx));

        let subscriptions = vec![
            cx.subscribe_in(&filter, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild(cx);
                }
            }),
            cx.subscribe(&tree, |this, _, event: &TreeEvent, _| match event {
                TreeEvent::Expanded(id) => {
                    this.expanded.insert(id.to_string());
                }
                TreeEvent::Collapsed(id) => {
                    this.expanded.remove(id.as_str());
                }
            }),
            cx.observe(&model, |this, _, cx| {
                this.rebuild(cx);
                cx.notify();
            }),
        ];

        let mut this = Self {
            model,
            filter,
            tree,
            expanded: HashSet::new(),
            selected: None,
            dragging: None,
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
    }

    /// Selects a request in the tree, opening the folders above it.
    pub fn reveal(&mut self, node: &NodeRef, cx: &mut Context<Self>) {
        let id = node.encode();
        self.selected = Some(id.clone());
        let id: SharedString = id.into();
        self.tree.update(cx, |tree, cx| {
            tree.reveal_item(&id, gpui_kit::ScrollStrategy::Center, cx);
            if let Some(ix) = tree.index_of(&id) {
                tree.set_selected_index(Some(ix), cx);
            }
        });
    }

    fn to_item(&self, node: &ExplorerNode, force_open: bool) -> TreeItem {
        let id = node.node.encode();
        let open = force_open || self.expanded.contains(&id);
        TreeItem::new(id, node.label.clone())
            .expanded(open)
            .children(node.children.iter().map(|child| self.to_item(child, force_open)))
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).value().to_string();
        let nodes = build_explorer(self.model.read(cx).config(), &filter);
        let filtering = !filter.trim().is_empty();
        let items: Vec<TreeItem> = nodes.iter().map(|n| self.to_item(n, filtering)).collect();
        let selected: Option<SharedString> = self.selected.clone().map(Into::into);
        self.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            if let Some(ix) = selected.as_ref().and_then(|id| tree.index_of(id)) {
                tree.set_selected_index(Some(ix), cx);
            }
        });
    }

    /// A row was picked up. Pressing a folder row opened or closed it before the press turned into a drag:
    /// it is put back as it was.
    fn drag_started(&mut self, node: NodeRef, cx: &mut Context<Self>) {
        if matches!(node, NodeRef::Folder { .. }) {
            let id = node.encode();
            if !self.expanded.remove(&id) {
                self.expanded.insert(id);
            }
            self.rebuild(cx);
        }
        self.dragging = Some(node);
        cx.notify();
    }

    /// A row was dropped on a folder, or on the top level. The folder is opened, so the row is seen landing.
    fn drop_on(&mut self, node: NodeRef, folder: Option<String>, cx: &mut Context<Self>) {
        self.dragging = None;
        if let Some(id) = &folder {
            self.expanded.insert(NodeRef::Folder { id: id.clone() }.encode());
        }
        cx.emit(RequestTreeEvent::Move { node, folder });
    }

    fn is_tree_empty(&self, cx: &App) -> bool {
        self.tree.read(cx).entry(0).is_none()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        h_flex()
            .flex_none()
            .gap_1()
            .p_2()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                ),
            )
            .child(
                Button::new("tree-new")
                    .icon(IconName::Plus)
                    .ghost()
                    .small()
                    .tooltip(t("requests.tree.new_tooltip"))
                    .dropdown_menu(move |menu, window, cx| new_menu(menu, &this, None, window, cx)),
            )
    }
}

fn render_row(
    ix: usize,
    entry: &TreeEntry,
    selected: bool,
    model: &Entity<AppModel>,
    tree: &WeakEntity<RequestTree>,
    cx: &mut App,
) -> ListItem {
    let theme = cx.theme().clone();
    let node = NodeRef::parse(entry.item().id.as_str());
    let label = entry.item().label.clone();
    let deleted = node.as_ref().is_some_and(|node| is_deleted(model.read(cx), node));
    let locked = node.as_ref().is_some_and(|node| locked_to(model.read(cx), node).is_some());
    let mut pending = false;
    let lead: gpui_kit::AnyElement = match &node {
        Some(NodeRef::Folder { .. }) => {
            Icon::new(if entry.is_expanded() { IconName::FolderOpen } else { IconName::Folder })
                .small()
                .text_color(theme.muted_foreground)
                .into_any_element()
        }
        // Saved by a newer Bifurc: the sign for an update stands where its type would, which this one cannot
        // vouch for.
        Some(NodeRef::Request { .. }) if locked => div()
            .w_12()
            .flex_none()
            .child(Icon::new(IconName::ArrowUp).small().text_color(theme.muted_foreground))
            .into_any_element(),
        Some(NodeRef::Request { kind, id }) => {
            let model = model.read(cx);
            pending = model.git().entity_state(id).is_some_and(|state| state != FileState::Clean);
            let badge = model
                .config()
                .request(*kind, id)
                .and_then(|r| r.method.clone())
                .unwrap_or_else(|| kind.badge().to_owned());
            div().w_12().flex_none().text_xs().text_color(theme.muted_foreground).child(badge).into_any_element()
        }
        None => div().into_any_element(),
    };

    let open = node.clone().filter(|n| matches!(n, NodeRef::Request { .. }));
    let folder = node.as_ref().and_then(|node| drop_folder(model.read(cx), node));
    let (drag_tree, drop_tree, drop_model) = (tree.clone(), tree.clone(), model.clone());
    let tree = tree.clone();
    let row = ListItem::new(ix)
        .selected(selected)
        .pl(rems(0.5 + entry.depth() as f32 * 1.0))
        .py_1()
        .text_sm()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(lead)
                // Deleted, and waiting to be confirmed or reverted: still listed, struck through.
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .when(deleted, |this| this.line_through().text_color(theme.muted_foreground))
                        .when(locked, |this| this.text_color(theme.muted_foreground))
                        .child(label.clone()),
                )
                // A request with changes that are not published yet says so with a mark, not only a color.
                .children(pending.then(|| div().flex_none().pr_2().text_xs().text_color(theme.warning).child("●"))),
        )
        .on_click(move |_, _, cx| {
            if let Some(node) = open.clone() {
                tree.update(cx, |_, cx| cx.emit(RequestTreeEvent::Open(node))).ok();
            }
        });
    // A row dropped on a request goes to the folder that request is in.
    let row = match folder {
        Some(folder) => {
            let target = folder.clone();
            tree_drag::drop_target(
                row,
                move |dragged: &NodeRef, cx| accepts(drop_model.read(cx), dragged, Some(&target)),
                move |dragged: &NodeRef, _, cx| {
                    let (node, folder) = (dragged.clone(), Some(folder.clone()));
                    drop_tree.update(cx, |this, cx| this.drop_on(node, folder, cx)).ok();
                },
            )
        }
        None => row,
    };
    // What is deleted stays where it is until it is reverted, and what is locked until Bifurc is updated.
    match node.filter(|_| !deleted && !locked) {
        Some(node) => tree_drag::draggable(row, node, label, move |node: &NodeRef, cx| {
            let node = node.clone();
            drag_tree.update(cx, |this, cx| this.drag_started(node, cx)).ok();
        }),
        None => row,
    }
}

fn emitter(
    tree: &WeakEntity<RequestTree>,
    event: RequestTreeEvent,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let tree = tree.clone();
    move |_, _, cx| {
        tree.update(cx, |_, cx| cx.emit(event.clone())).ok();
    }
}

fn context_menu(
    entry: &TreeEntry,
    menu: PopupMenu,
    tree: &WeakEntity<RequestTree>,
    model: &Entity<AppModel>,
    cx: &App,
) -> PopupMenu {
    let Some(node) = NodeRef::parse(entry.item().id.as_str()) else { return menu };
    // What is deleted can only be brought back or deleted for good.
    if is_deleted(model.read(cx), &node) {
        return menu
            .item(
                PopupMenuItem::new(t("requests.tree.menu.revert_delete"))
                    .on_click(emitter(tree, RequestTreeEvent::RevertDelete(node.clone()))),
            )
            .item(
                PopupMenuItem::new(t("requests.tree.menu.confirm_delete"))
                    .on_click(emitter(tree, RequestTreeEvent::ConfirmDelete(node.clone()))),
            );
    }
    // What a newer Bifurc saved can only be asked about: opening it says which version it needs.
    if locked_to(model.read(cx), &node).is_some() {
        return menu
            .item(PopupMenuItem::new(t("requests.tree.menu.update_required")).on_click(emitter(tree, RequestTreeEvent::Open(node.clone()))));
    }
    match node {
        NodeRef::Request { .. } => menu
            .item(PopupMenuItem::new(t("requests.common.open")).on_click(emitter(tree, RequestTreeEvent::Open(node.clone()))))
            .item(PopupMenuItem::new(t("requests.common.duplicate")).on_click(emitter(tree, RequestTreeEvent::Duplicate(node.clone()))))
            .separator()
            .item(PopupMenuItem::new(t("requests.tree.menu.rename")).on_click(emitter(tree, RequestTreeEvent::Rename(node.clone()))))
            .item(PopupMenuItem::new(t("requests.tree.menu.delete")).on_click(emitter(tree, RequestTreeEvent::Delete(node.clone())))),
        NodeRef::Folder { ref id } => menu
            // The engine runs the REST requests of a folder; the other types in it have no runner.
            .item(PopupMenuItem::new(t("requests.tree.menu.open_as_runner")).on_click(emitter(tree, RequestTreeEvent::RunFolder(id.clone()))))
            .separator()
            // A new request starts as REST; its type is changed in the breadcrumb until it is saved.
            .item(PopupMenuItem::new(t("requests.tree.menu.new_request_here")).on_click(emitter(
                tree,
                RequestTreeEvent::NewRequest { kind: RequestKind::Rest, folder: Some(id.clone()) },
            )))
            .item(
                PopupMenuItem::new(t("requests.tree.menu.new_folder_here"))
                    .on_click(emitter(tree, RequestTreeEvent::NewFolder { parent: Some(id.clone()) })),
            )
            .separator()
            .item(PopupMenuItem::new(t("requests.tree.menu.rename")).on_click(emitter(tree, RequestTreeEvent::Rename(node.clone()))))
            .item(PopupMenuItem::new(t("requests.tree.menu.delete")).on_click(emitter(tree, RequestTreeEvent::Delete(node.clone())))),
    }
}

/// The "new" menu: one request of each type, one from a cURL command, and a folder. A folder takes requests
/// of every type.
pub fn new_menu(
    menu: PopupMenu,
    tree: &WeakEntity<RequestTree>,
    folder: Option<String>,
    _: &mut Window,
    _: &mut Context<PopupMenu>,
) -> PopupMenu {
    let menu = RequestKind::ALL.into_iter().fold(menu, |menu, kind| {
        menu.item(
            PopupMenuItem::new(t_with("requests.tree.new_menu.kind_request", &[("kind", &kind.label())]))
                .on_click(emitter(tree, RequestTreeEvent::NewRequest { kind, folder: folder.clone() })),
        )
    });
    let menu = menu
        .separator()
        .item(PopupMenuItem::new(t(CURL_TAB_TITLE)).on_click(emitter(tree, RequestTreeEvent::ImportCurl)));
    menu.separator()
        .item(PopupMenuItem::new(t("requests.tree.new_menu.folder")).on_click(emitter(tree, RequestTreeEvent::NewFolder { parent: folder })))
}

impl Render for RequestTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.clone();
        let this = cx.entity().downgrade();
        let menu_tree = this.clone();
        let menu_model = self.model.clone();
        let config = self.model.read(cx).config();
        let has_requests = !config.requests.is_empty() || !config.folders.is_empty();
        let filtering = !self.filter.read(cx).value().trim().is_empty();
        if !cx.has_active_drag() {
            self.dragging = None;
        }
        // A folder that is in a folder can be dragged out to the top level.
        let top_level =
            self.dragging.as_ref().filter(|dragged| accepts(self.model.read(cx), dragged, None)).map(|_| {
                let (drop_model, drop_tree) = (self.model.clone(), this.clone());
                tree_drag::top_level_target(
                    "request-tree-top-level",
                    move |dragged: &NodeRef, cx| accepts(drop_model.read(cx), dragged, None),
                    move |dragged: &NodeRef, _, cx| {
                        let node = dragged.clone();
                        drop_tree.update(cx, |this, cx| this.drop_on(node, None, cx)).ok();
                    },
                    cx,
                )
            });

        let body = if !has_requests {
            page::empty(t("requests.tree.empty.title"), t("requests.tree.empty.description"))
        } else if filtering && self.is_tree_empty(cx) {
            page::empty(t("requests.tree.no_matches.title"), t("requests.tree.no_matches.description"))
        } else {
            tree(&self.tree, move |ix, entry, selected, _, cx| render_row(ix, entry, selected, &model, &this, cx))
                .context_menu(move |_, entry, menu, _, cx| context_menu(entry, menu, &menu_tree, &menu_model, cx))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(self.render_header(cx))
            .child(div().flex_1().min_h_0().child(body))
            .children(top_level)
    }
}
