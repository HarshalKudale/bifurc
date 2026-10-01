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
    components::page,
    model::{ExplorerNode, NodeRef, RequestKind, build_explorer, git::FileState},
};

/// What the user asked for in the tree. The screen acts on it.
#[derive(Debug, Clone)]
pub enum RequestTreeEvent {
    Open(NodeRef),
    NewRequest { kind: RequestKind, folder: Option<String> },
    NewFolder { kind: RequestKind, parent: Option<String> },
    Rename(NodeRef),
    Delete(NodeRef),
    /// Copy the request, next to the original.
    Duplicate(NodeRef),
    /// Open the REST folder with this id as a runner tab.
    RunFolder(String),
}

pub struct RequestTree {
    model: Entity<AppModel>,
    filter: Entity<InputState>,
    tree: Entity<TreeState>,
    /// Folders the user has opened, by tree id, so a rebuild does not collapse them.
    expanded: HashSet<String>,
    selected: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RequestTreeEvent> for RequestTree {}

impl RequestTree {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter requests"));
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

        let mut this =
            Self { model, filter, tree, expanded: HashSet::new(), selected: None, _subscriptions: subscriptions };
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
                    .tooltip("New request or folder")
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
    let mut pending = false;
    let lead: gpui_kit::AnyElement = match &node {
        Some(NodeRef::Folder { .. }) => {
            Icon::new(if entry.is_expanded() { IconName::FolderOpen } else { IconName::Folder })
                .small()
                .text_color(theme.muted_foreground)
                .into_any_element()
        }
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
    let tree = tree.clone();
    ListItem::new(ix)
        .selected(selected)
        .pl(rems(0.5 + entry.depth() as f32 * 1.0))
        .py_1()
        .text_sm()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(lead)
                .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().child(label))
                // A request with changes that are not published yet says so with a mark, not only a color.
                .children(pending.then(|| div().flex_none().pr_2().text_xs().text_color(theme.warning).child("●"))),
        )
        .on_click(move |_, _, cx| {
            if let Some(node) = open.clone() {
                tree.update(cx, |_, cx| cx.emit(RequestTreeEvent::Open(node))).ok();
            }
        })
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

fn context_menu(entry: &TreeEntry, menu: PopupMenu, tree: &WeakEntity<RequestTree>) -> PopupMenu {
    let Some(node) = NodeRef::parse(entry.item().id.as_str()) else { return menu };
    match node {
        NodeRef::Request { .. } => menu
            .item(PopupMenuItem::new("Open").on_click(emitter(tree, RequestTreeEvent::Open(node.clone()))))
            .item(PopupMenuItem::new("Duplicate").on_click(emitter(tree, RequestTreeEvent::Duplicate(node.clone()))))
            .separator()
            .item(PopupMenuItem::new("Rename…").on_click(emitter(tree, RequestTreeEvent::Rename(node.clone()))))
            .item(PopupMenuItem::new("Delete…").on_click(emitter(tree, RequestTreeEvent::Delete(node.clone())))),
        NodeRef::Folder { kind, ref id } => menu
            // The engine runs folders of REST requests; the other types have no runner.
            .when(kind == RequestKind::Rest, |menu| {
                menu.item(
                    PopupMenuItem::new("Open as runner")
                        .on_click(emitter(tree, RequestTreeEvent::RunFolder(id.clone()))),
                )
                .separator()
            })
            .item(
                PopupMenuItem::new(format!("New {} request here", kind.label()))
                    .on_click(emitter(tree, RequestTreeEvent::NewRequest { kind, folder: Some(id.clone()) })),
            )
            .item(
                PopupMenuItem::new("New folder here…")
                    .on_click(emitter(tree, RequestTreeEvent::NewFolder { kind, parent: Some(id.clone()) })),
            )
            .separator()
            .item(PopupMenuItem::new("Rename…").on_click(emitter(tree, RequestTreeEvent::Rename(node.clone()))))
            .item(PopupMenuItem::new("Delete…").on_click(emitter(tree, RequestTreeEvent::Delete(node.clone())))),
    }
}

/// The "new" menu: one request of each type, and a folder for each type's tree.
pub fn new_menu(
    menu: PopupMenu,
    tree: &WeakEntity<RequestTree>,
    folder: Option<String>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let menu = RequestKind::ALL.into_iter().fold(menu, |menu, kind| {
        menu.item(
            PopupMenuItem::new(format!("{} request", kind.label()))
                .on_click(emitter(tree, RequestTreeEvent::NewRequest { kind, folder: folder.clone() })),
        )
    });
    let tree = tree.clone();
    menu.separator().submenu("Folder", window, cx, move |submenu, _, _| {
        RequestKind::ALL.into_iter().fold(submenu, |submenu, kind| {
            submenu.item(
                PopupMenuItem::new(format!("{} folder…", kind.label()))
                    .on_click(emitter(&tree, RequestTreeEvent::NewFolder { kind, parent: None })),
            )
        })
    })
}

impl Render for RequestTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.clone();
        let this = cx.entity().downgrade();
        let menu_tree = this.clone();
        let config = self.model.read(cx).config();
        let has_requests = !config.requests.is_empty() || !config.folders.is_empty();
        let filtering = !self.filter.read(cx).value().trim().is_empty();

        let body = if !has_requests {
            page::empty("No requests yet", "Use + to create the first one.")
        } else if filtering && self.is_tree_empty(cx) {
            page::empty("No matches", "No request matches the filter.")
        } else {
            tree(&self.tree, move |ix, entry, selected, _, cx| render_row(ix, entry, selected, &model, &this, cx))
                .context_menu(move |_, entry, menu, _, _| context_menu(entry, menu, &menu_tree))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(self.render_header(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}
