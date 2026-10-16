//! The folder tree of the Flows screen: every saved flow, in its folders.
//!
//! It owns only view state: what is expanded, selected, and typed into the filter. What the tree shows comes
//! from the model's snapshot, and everything the user asks for leaves as a [`FlowTreeEvent`] for the screen,
//! which turns it into engine commands. After a command the engine says what changed and the tree rebuilds.

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
    app_model::{AppModel, Deleted},
    components::{page, tree_drag},
    i18n::t,
    model::{
        git::FileState,
        tree::{FolderRow, ItemRow, Moved, TreeKey, TreeNode, build_tree, can_move},
    },
};

/// What the user asked for in the tree. The screen acts on it.
#[derive(Debug, Clone)]
pub enum FlowTreeEvent {
    Open(String),
    NewFlow { folder: Option<String> },
    NewFolder { parent: Option<String> },
    Rename(TreeKey),
    Duplicate(String),
    Delete(TreeKey),
    /// Put the flow or folder in another folder; a folder goes to the top level with `None`.
    Move { key: TreeKey, folder: Option<String> },
}

/// What `entity.delete` and `folder.delete` call a flow and a folder of flows.
pub fn deleted_target(key: &TreeKey) -> Deleted {
    match key {
        TreeKey::Folder(id) => Deleted::Folder { kind: "flow", id: id.clone() },
        TreeKey::Item(id) => Deleted::Entity { kind: "flows", id: id.clone() },
    }
}

/// Whether the flow or folder is deleted here, with the deletion not confirmed or reverted yet.
pub fn is_deleted(model: &AppModel, key: &TreeKey) -> bool {
    let config = model.config();
    match key {
        TreeKey::Folder(id) => config.flow_folders.iter().any(|f| f.id == *id && f.deleted),
        TreeKey::Item(id) => config.flows.iter().any(|f| f.id == *id && f.deleted),
    }
}

/// The version of Bifurc the flow needs, when a newer one than this last saved it. A folder is never locked:
/// it is a directory, and moving or renaming it changes nothing in it.
fn locked_to(model: &AppModel, key: &TreeKey) -> Option<String> {
    match key {
        TreeKey::Folder(_) => None,
        TreeKey::Item(id) => model.config().flows.iter().find(|f| f.id == *id).and_then(|f| f.locked.clone()),
    }
}

fn folder_of(model: &AppModel, id: &str) -> Option<String> {
    model.config().flows.iter().find(|f| f.id == id).and_then(|f| f.folder_id.clone())
}

/// Whether the dragged flow or folder can be put in `folder`, or at the top level with `None`.
fn accepts(model: &AppModel, dragged: &TreeKey, folder: Option<&str>) -> bool {
    let config = model.config();
    let item_folder;
    let moved = match dragged {
        TreeKey::Folder(id) => Moved::Folder(id),
        TreeKey::Item(id) => {
            item_folder = folder_of(model, id);
            Moved::Item { folder: item_folder.as_deref() }
        }
    };
    can_move(&config.flow_folder_rows(), moved, folder)
}

pub struct FlowTree {
    model: Entity<AppModel>,
    filter: Entity<InputState>,
    tree: Entity<TreeState>,
    /// Folders the user has opened, by tree key, so a rebuild does not collapse them.
    expanded: HashSet<String>,
    selected: Option<String>,
    /// The row being dragged, while a drag is on.
    dragging: Option<TreeKey>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FlowTreeEvent> for FlowTree {}

impl FlowTree {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t("flows.tree.filter")));
        let tree = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
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

    /// Mark the flow that is open, opening the folders above it.
    pub fn reveal(&mut self, flow_id: Option<&str>, cx: &mut Context<Self>) {
        self.selected = flow_id.map(|id| TreeKey::Item(id.to_owned()).encode());
        let Some(id) = self.selected.clone().map(SharedString::from) else { return };
        self.tree.update(cx, |tree, cx| {
            tree.reveal_item(&id, gpui_kit::ScrollStrategy::Center, cx);
            if let Some(ix) = tree.index_of(&id) {
                tree.set_selected_index(Some(ix), cx);
            }
        });
    }

    fn to_item(&self, node: &TreeNode, force_open: bool) -> TreeItem {
        let open = force_open || self.expanded.contains(&node.key);
        TreeItem::new(node.key.clone(), node.label.clone())
            .expanded(open)
            .children(node.children.iter().map(|child| self.to_item(child, force_open)))
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).value().to_string();
        let config = self.model.read(cx).config();
        // Deleted folders stay in the tree, so a deleted flow shows inside its deleted folder.
        let folders: Vec<FolderRow> = config
            .flow_folders
            .iter()
            .map(|f| FolderRow { id: &f.id, name: &f.name, parent_id: f.parent_id.as_deref() })
            .collect();
        let items: Vec<ItemRow> = config
            .flows
            .iter()
            .map(|f| ItemRow { id: &f.id, label: f.name.clone(), folder_id: f.folder_id.as_deref(), detail: "" })
            .collect();
        let nodes = build_tree(&folders, &items, &filter);
        let filtering = !filter.trim().is_empty();
        let items: Vec<TreeItem> = nodes.iter().map(|node| self.to_item(node, filtering)).collect();
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
    fn drag_started(&mut self, key: TreeKey, cx: &mut Context<Self>) {
        if matches!(key, TreeKey::Folder(_)) {
            let id = key.encode();
            if !self.expanded.remove(&id) {
                self.expanded.insert(id);
            }
            self.rebuild(cx);
        }
        self.dragging = Some(key);
        cx.notify();
    }

    /// A row was dropped on a folder, or on the top level. The folder is opened, so the row is seen landing.
    fn drop_on(&mut self, key: TreeKey, folder: Option<String>, cx: &mut Context<Self>) {
        self.dragging = None;
        if let Some(id) = &folder {
            self.expanded.insert(TreeKey::Folder(id.clone()).encode());
        }
        cx.emit(FlowTreeEvent::Move { key, folder });
    }
}

fn emitter(
    tree: &WeakEntity<FlowTree>,
    event: FlowTreeEvent,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let tree = tree.clone();
    move |_, _, cx| {
        tree.update(cx, |_, cx| cx.emit(event.clone())).ok();
    }
}

fn new_menu(menu: PopupMenu, tree: &WeakEntity<FlowTree>, folder: Option<String>) -> PopupMenu {
    menu.item(PopupMenuItem::new(t("flows.menu.new_flow")).on_click(emitter(tree, FlowTreeEvent::NewFlow { folder: folder.clone() })))
        .item(PopupMenuItem::new(t("flows.menu.new_folder")).on_click(emitter(tree, FlowTreeEvent::NewFolder { parent: folder })))
}

fn context_menu(
    entry: &TreeEntry,
    menu: PopupMenu,
    tree: &WeakEntity<FlowTree>,
    model: &Entity<AppModel>,
    cx: &App,
) -> PopupMenu {
    let Some(key) = TreeKey::parse(entry.item().id.as_str()) else { return menu };
    // What is deleted can only be brought back or deleted for good.
    if is_deleted(model.read(cx), &key) {
        let (revert_model, confirm_model) = (model.clone(), model.clone());
        let (revert, confirm) = (deleted_target(&key), deleted_target(&key));
        return menu
            .item(PopupMenuItem::new(t("flows.menu.revert_delete")).on_click(move |_, _, cx| {
                revert_model.update(cx, |model, cx| model.revert_delete(revert.clone(), cx));
            }))
            .item(PopupMenuItem::new(t("flows.menu.confirm_delete")).on_click(move |_, _, cx| {
                confirm_model.update(cx, |model, cx| model.confirm_delete(confirm.clone(), cx));
            }));
    }
    // What a newer Bifurc saved can only be asked about: opening it says which version it needs.
    if let (TreeKey::Item(id), Some(_)) = (&key, locked_to(model.read(cx), &key)) {
        return menu.item(
            PopupMenuItem::new(t("flows.menu.update_required")).on_click(emitter(tree, FlowTreeEvent::Open(id.clone()))),
        );
    }
    let menu = match &key {
        TreeKey::Item(id) => menu
            .item(PopupMenuItem::new(t("flows.common.open")).on_click(emitter(tree, FlowTreeEvent::Open(id.clone()))))
            .item(PopupMenuItem::new(t("flows.common.duplicate")).on_click(emitter(tree, FlowTreeEvent::Duplicate(id.clone())))),
        TreeKey::Folder(id) => menu
            .item(
                PopupMenuItem::new(t("flows.menu.new_flow_here"))
                    .on_click(emitter(tree, FlowTreeEvent::NewFlow { folder: Some(id.clone()) })),
            )
            .item(
                PopupMenuItem::new(t("flows.menu.new_folder_here"))
                    .on_click(emitter(tree, FlowTreeEvent::NewFolder { parent: Some(id.clone()) })),
            ),
    };
    menu.separator()
        .item(PopupMenuItem::new(t("flows.menu.rename")).on_click(emitter(tree, FlowTreeEvent::Rename(key.clone()))))
        .item(PopupMenuItem::new(t("flows.menu.delete")).on_click(emitter(tree, FlowTreeEvent::Delete(key.clone()))))
}

fn render_row(
    ix: usize,
    entry: &TreeEntry,
    selected: bool,
    model: &Entity<AppModel>,
    tree: &WeakEntity<FlowTree>,
    cx: &mut App,
) -> ListItem {
    let theme = cx.theme().clone();
    let key = TreeKey::parse(entry.item().id.as_str());
    let label = entry.item().label.clone();
    let deleted = key.as_ref().is_some_and(|key| is_deleted(model.read(cx), key));
    let locked = key.as_ref().is_some_and(|key| locked_to(model.read(cx), key).is_some());
    let icon = match &key {
        Some(TreeKey::Folder(_)) if entry.is_expanded() => IconName::FolderOpen,
        Some(TreeKey::Folder(_)) => IconName::Folder,
        _ => IconName::LayoutDashboard,
    };
    // A flow with changes that are not published yet says so with a mark, not only a color.
    let pending = match &key {
        Some(TreeKey::Item(id)) => {
            model.read(cx).git().entity_state(id).is_some_and(|state| state != FileState::Clean)
        }
        _ => false,
    };
    let open = match &key {
        Some(TreeKey::Item(id)) => Some(id.clone()),
        _ => None,
    };
    // A row dropped on a flow goes to the folder that flow is in.
    let folder = match &key {
        Some(TreeKey::Folder(id)) => Some(id.clone()),
        Some(TreeKey::Item(id)) => folder_of(model.read(cx), id),
        None => None,
    };
    let (click_tree, drag_tree, drop_tree, drop_model) = (tree.clone(), tree.clone(), tree.clone(), model.clone());
    let row = ListItem::new(ix)
        .selected(selected)
        .pl(rems(0.5 + entry.depth() as f32 * 1.0))
        .py_1()
        .text_sm()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(icon).small().text_color(theme.muted_foreground))
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
                // Saved by a newer Bifurc: the sign for an update. Opening it says which version.
                .children(locked.then(|| Icon::new(IconName::ArrowUp).small().text_color(theme.muted_foreground)))
                .children(pending.then(|| div().flex_none().pr_2().text_xs().text_color(theme.warning).child("●"))),
        )
        .on_click(move |_, _, cx| {
            if let Some(id) = open.clone() {
                click_tree.update(cx, |_, cx| cx.emit(FlowTreeEvent::Open(id))).ok();
            }
        });
    let row = match folder {
        Some(folder) => {
            let target = folder.clone();
            tree_drag::drop_target(
                row,
                move |dragged: &TreeKey, cx| accepts(drop_model.read(cx), dragged, Some(&target)),
                move |dragged: &TreeKey, _, cx| {
                    let (key, folder) = (dragged.clone(), Some(folder.clone()));
                    drop_tree.update(cx, |this, cx| this.drop_on(key, folder, cx)).ok();
                },
            )
        }
        None => row,
    };
    // What is deleted stays where it is until it is reverted, and what is locked until Bifurc is updated.
    match key.filter(|_| !deleted && !locked) {
        Some(key) => tree_drag::draggable(row, key, label, move |key: &TreeKey, cx| {
            let key = key.clone();
            drag_tree.update(cx, |this, cx| this.drag_started(key, cx)).ok();
        }),
        None => row,
    }
}

impl Render for FlowTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let (row_tree, menu_tree, new_tree) = (this.clone(), this.clone(), this.clone());
        let (model, menu_model) = (self.model.clone(), self.model.clone());
        let config = self.model.read(cx).config();
        let has_flows = !config.flows.is_empty() || !config.flow_folders.is_empty();
        let filtering = !self.filter.read(cx).value().trim().is_empty();
        if !cx.has_active_drag() {
            self.dragging = None;
        }
        // A folder that is in a folder can be dragged out to the top level.
        let top_level =
            self.dragging.as_ref().filter(|dragged| accepts(self.model.read(cx), dragged, None)).map(|_| {
                let (drop_model, drop_tree) = (self.model.clone(), this.clone());
                tree_drag::top_level_target(
                    "flow-tree-top-level",
                    move |dragged: &TreeKey, cx| accepts(drop_model.read(cx), dragged, None),
                    move |dragged: &TreeKey, _, cx| {
                        let key = dragged.clone();
                        drop_tree.update(cx, |this, cx| this.drop_on(key, None, cx)).ok();
                    },
                    cx,
                )
            });

        let body = if !has_flows {
            page::empty(t("flows.tree.empty.title"), t("flows.tree.empty.description"))
        } else if filtering && self.tree.read(cx).entry(0).is_none() {
            page::empty(t("flows.tree.no_matches.title"), t("flows.tree.no_matches.description"))
        } else {
            tree(&self.tree, move |ix, entry, selected, _, cx| render_row(ix, entry, selected, &model, &row_tree, cx))
                .context_menu(move |_, entry, menu, _, cx| context_menu(entry, menu, &menu_tree, &menu_model, cx))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .p_2()
                    .child(div().flex_1().min_w_0().child(
                        Input::new(&self.filter).small().prefix(Icon::new(IconName::Search).small()).cleanable(true),
                    ))
                    .child(
                        Button::new("flow-tree-new")
                            .icon(IconName::Plus)
                            .ghost()
                            .small()
                            .tooltip(t("flows.tree.new_tooltip"))
                            .dropdown_menu(move |menu, _, _| new_menu(menu, &new_tree, None)),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .children(top_level)
    }
}
