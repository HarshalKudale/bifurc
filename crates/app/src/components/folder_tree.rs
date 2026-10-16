//! The folder tree every screen with saved entities in folders shows: Requests, Mocks and Flows.
//!
//! It owns the view state (what is expanded, selected, typed into the filter, dragged) and everything the
//! screens had in common: the filter, rebuilding on a model change, the rows with their states (deleted,
//! locked, pending), drag and drop, the context menu's shared items, and the empty states. What differs is
//! given by a [`FolderTreeSource`]: what the model holds, how a row's lead looks, and the items that start
//! something new. What the user asks for leaves as a [`FolderTreeEvent`] for the screen, which turns it into
//! engine commands; after a command the engine says what changed and the tree rebuilds.

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
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, WeakEntity, Window, div, rems,
};

use crate::{
    app_model::AppModel,
    components::{page, tree_drag},
    i18n::t,
    model::{
        NodeRef,
        git::FileState,
        tree::{FolderRow, Moved, TreeKey, TreeNode, can_move},
    },
};

/// What a row of a tree stands for: a folder, or an item in one. The key is also the tree item's id.
pub trait FolderTreeKey: Clone + 'static {
    fn encode(&self) -> String;
    fn parse(encoded: &str) -> Option<Self>;
    fn folder(id: &str) -> Self;
    /// The id of the folder, when the row is one.
    fn folder_id(&self) -> Option<&str>;
    /// The id of the entity, when the row is one.
    fn item_id(&self) -> Option<&str>;
}

impl FolderTreeKey for TreeKey {
    fn encode(&self) -> String {
        TreeKey::encode(self)
    }
    fn parse(encoded: &str) -> Option<Self> {
        TreeKey::parse(encoded)
    }
    fn folder(id: &str) -> Self {
        TreeKey::Folder(id.to_owned())
    }
    fn folder_id(&self) -> Option<&str> {
        match self {
            TreeKey::Folder(id) => Some(id),
            TreeKey::Item(_) => None,
        }
    }
    fn item_id(&self) -> Option<&str> {
        match self {
            TreeKey::Folder(_) => None,
            TreeKey::Item(id) => Some(id),
        }
    }
}

impl FolderTreeKey for NodeRef {
    fn encode(&self) -> String {
        NodeRef::encode(self)
    }
    fn parse(encoded: &str) -> Option<Self> {
        NodeRef::parse(encoded)
    }
    fn folder(id: &str) -> Self {
        NodeRef::Folder { id: id.to_owned() }
    }
    fn folder_id(&self) -> Option<&str> {
        match self {
            NodeRef::Folder { id } => Some(id),
            NodeRef::Request { .. } => None,
        }
    }
    fn item_id(&self) -> Option<&str> {
        match self {
            NodeRef::Folder { .. } => None,
            NodeRef::Request { id, .. } => Some(id),
        }
    }
}

/// What stands before an item's name.
pub enum Lead {
    /// A short text, such as the method of a request, in a column that lines the names up.
    Badge(String),
    /// An icon for the kind of entity.
    Icon(IconName),
}

/// The words of a tree, as keys of the screen's own i18n namespace.
pub struct TreeText {
    pub filter_placeholder: &'static str,
    pub new_tooltip: &'static str,
    pub empty_title: &'static str,
    pub empty_description: &'static str,
    pub no_matches_title: &'static str,
    pub no_matches_description: &'static str,
    pub open: &'static str,
    pub duplicate: &'static str,
    pub rename: &'static str,
    pub delete: &'static str,
    pub revert_delete: &'static str,
    pub confirm_delete: &'static str,
    pub update_required: &'static str,
    pub new_folder: &'static str,
    pub new_folder_here: &'static str,
}

/// What a screen gives its tree. The functions read the model; they do not change it.
pub trait FolderTreeSource: 'static {
    type Key: FolderTreeKey;
    /// What this screen's menus can ask for besides what every tree can: starting an entity of its kind.
    type Extra: Clone + 'static;
    const TEXT: TreeText;

    /// The tree as the model holds it, filtered by the text typed.
    fn nodes(model: &AppModel, filter: &str) -> Vec<TreeNode>;
    /// Whether the model has no entity and no folder of this kind.
    fn is_empty(model: &AppModel) -> bool;
    /// The folders an entity of this kind can be kept in.
    fn folder_rows(model: &AppModel) -> Vec<FolderRow<'_>>;
    /// The folder the entity is in: `None` when it is not known, `Some(None)` at the top.
    fn item_folder(model: &AppModel, key: &Self::Key) -> Option<Option<String>>;
    /// Whether the entity or folder is deleted, with the deletion not confirmed or reverted yet.
    fn is_deleted(model: &AppModel, key: &Self::Key) -> bool;
    /// The version of Bifurc the entity needs, when a newer one than this last saved it. A folder is never
    /// locked: it is a directory, and moving or renaming it changes nothing in it.
    fn locked_to(model: &AppModel, key: &Self::Key) -> Option<String>;
    /// What stands before the item's name, if it is not locked.
    fn lead(model: &AppModel, key: &Self::Key) -> Option<Lead>;
    /// What stands after the item's name, such as a switch.
    fn trailing(_model: &Entity<AppModel>, _key: &Self::Key, _cx: &App) -> Option<AnyElement> {
        None
    }
    /// The items of a folder's menu that start something in it, and any other that only this screen has.
    fn folder_menu(menu: PopupMenu, folder: &str, tree: &WeakEntity<FolderTree<Self>>) -> PopupMenu
    where
        Self: Sized;
    /// The items of the "new" menu, before "new folder", which the tree adds.
    fn new_menu(menu: PopupMenu, folder: Option<String>, tree: &WeakEntity<FolderTree<Self>>) -> PopupMenu
    where
        Self: Sized;
}

/// What the user asked for in the tree. The screen acts on it.
#[derive(Debug, Clone)]
pub enum FolderTreeEvent<K, X> {
    Open(K),
    Rename(K),
    Delete(K),
    /// Copy the entity, next to the original.
    Duplicate(K),
    /// Bring back a deleted entity or folder.
    RevertDelete(K),
    /// Sync the deletion of a deleted entity or folder, removing it for good.
    ConfirmDelete(K),
    NewFolder {
        parent: Option<String>,
    },
    /// Put the entity or folder in another folder; a folder goes to the top level with `None`.
    Move {
        key: K,
        folder: Option<String>,
    },
    /// What only this screen's tree can ask for.
    Extra(X),
}

pub type EventOf<S> = FolderTreeEvent<<S as FolderTreeSource>::Key, <S as FolderTreeSource>::Extra>;

/// A click handler for a menu item that sends `event` to the screen.
pub fn emitter<S: FolderTreeSource>(
    tree: &WeakEntity<FolderTree<S>>,
    event: EventOf<S>,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let tree = tree.clone();
    move |_, _, cx| {
        tree.update(cx, |_, cx| cx.emit(event.clone())).ok();
    }
}

pub struct FolderTree<S: FolderTreeSource> {
    model: Entity<AppModel>,
    filter: Entity<InputState>,
    tree: Entity<TreeState>,
    /// Folders the user has opened, by tree id, so a rebuild does not collapse them.
    expanded: HashSet<String>,
    selected: Option<String>,
    /// The row being dragged, while a drag is on.
    dragging: Option<S::Key>,
    _subscriptions: Vec<Subscription>,
}

impl<S: FolderTreeSource> EventEmitter<EventOf<S>> for FolderTree<S> {}

/// The tree's items for the nodes. Folders are open when the user opened them, or all are while a filter
/// shows only what matches.
fn to_items(nodes: &[TreeNode], expanded: &HashSet<String>, force_open: bool) -> Vec<TreeItem> {
    nodes
        .iter()
        .map(|node| {
            TreeItem::new(node.key.clone(), node.label.clone())
                .expanded(force_open || expanded.contains(&node.key))
                .children(to_items(&node.children, expanded, force_open))
        })
        .collect()
}

/// Opens a folder that is closed, and closes one that is open.
fn flip(expanded: &mut HashSet<String>, id: String) {
    if !expanded.remove(&id) {
        expanded.insert(id);
    }
}

impl<S: FolderTreeSource> FolderTree<S> {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t(S::TEXT.filter_placeholder)));
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

    /// Marks the entity that is open, opening the folders above it; `None` when nothing is open.
    pub fn reveal(&mut self, key: Option<&S::Key>, cx: &mut Context<Self>) {
        self.selected = key.map(FolderTreeKey::encode);
        let id: Option<SharedString> = self.selected.clone().map(Into::into);
        self.tree.update(cx, |tree, cx| match id {
            Some(id) => {
                tree.reveal_item(&id, gpui_kit::ScrollStrategy::Center, cx);
                if let Some(ix) = tree.index_of(&id) {
                    tree.set_selected_index(Some(ix), cx);
                }
            }
            None => tree.set_selected_index(None, cx),
        });
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).value().to_string();
        let nodes = S::nodes(self.model.read(cx), &filter);
        let items = to_items(&nodes, &self.expanded, !filter.trim().is_empty());
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
    fn drag_started(&mut self, key: S::Key, cx: &mut Context<Self>) {
        if key.folder_id().is_some() {
            flip(&mut self.expanded, key.encode());
            self.rebuild(cx);
        }
        self.dragging = Some(key);
        cx.notify();
    }

    /// A row was dropped on a folder, or on the top level. The folder is opened, so the row is seen landing.
    fn drop_on(&mut self, key: S::Key, folder: Option<String>, cx: &mut Context<Self>) {
        self.dragging = None;
        if let Some(id) = &folder {
            self.expanded.insert(S::Key::folder(id).encode());
        }
        cx.emit(FolderTreeEvent::Move { key, folder });
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
                Button::new("folder-tree-new")
                    .icon(IconName::Plus)
                    .ghost()
                    .small()
                    .tooltip(t(S::TEXT.new_tooltip))
                    .dropdown_menu(move |menu, _, _| new_menu::<S>(menu, None, &this)),
            )
    }
}

/// The folder a drop on this row goes to: the folder itself, or the folder the item is in.
fn drop_folder<S: FolderTreeSource>(model: &AppModel, key: &S::Key) -> Option<String> {
    match key.folder_id() {
        Some(id) => Some(id.to_owned()),
        None => S::item_folder(model, key).flatten(),
    }
}

/// Whether the dragged item or folder can be put in `folder`, or at the top level with `None`. An item that
/// is no longer there cannot be.
fn accepts<S: FolderTreeSource>(model: &AppModel, dragged: &S::Key, folder: Option<&str>) -> bool {
    let item_folder;
    let moved = match dragged.folder_id() {
        Some(id) => Moved::Folder(id),
        None => match S::item_folder(model, dragged) {
            Some(current) => {
                item_folder = current;
                Moved::Item { folder: item_folder.as_deref() }
            }
            None => return false,
        },
    };
    can_move(&S::folder_rows(model), moved, folder)
}

/// The "new" menu: what the screen starts, then a folder.
pub fn new_menu<S: FolderTreeSource>(
    menu: PopupMenu,
    folder: Option<String>,
    tree: &WeakEntity<FolderTree<S>>,
) -> PopupMenu {
    let menu = S::new_menu(menu, folder.clone(), tree);
    menu.item(
        PopupMenuItem::new(t(S::TEXT.new_folder))
            .on_click(emitter(tree, FolderTreeEvent::NewFolder { parent: folder })),
    )
}

fn context_menu<S: FolderTreeSource>(
    entry: &TreeEntry,
    menu: PopupMenu,
    tree: &WeakEntity<FolderTree<S>>,
    model: &Entity<AppModel>,
    cx: &App,
) -> PopupMenu {
    let Some(key) = S::Key::parse(entry.item().id.as_str()) else { return menu };
    let text = &S::TEXT;
    // What is deleted can only be brought back or deleted for good.
    if S::is_deleted(model.read(cx), &key) {
        return menu
            .item(
                PopupMenuItem::new(t(text.revert_delete))
                    .on_click(emitter(tree, FolderTreeEvent::RevertDelete(key.clone()))),
            )
            .item(
                PopupMenuItem::new(t(text.confirm_delete))
                    .on_click(emitter(tree, FolderTreeEvent::ConfirmDelete(key.clone()))),
            );
    }
    // What a newer Bifurc saved can only be asked about: opening it says which version it needs.
    if S::locked_to(model.read(cx), &key).is_some() {
        return menu
            .item(PopupMenuItem::new(t(text.update_required)).on_click(emitter(tree, FolderTreeEvent::Open(key))));
    }
    let menu = match key.folder_id() {
        Some(id) => S::folder_menu(menu, id, tree).item(
            PopupMenuItem::new(t(text.new_folder_here))
                .on_click(emitter(tree, FolderTreeEvent::NewFolder { parent: Some(id.to_owned()) })),
        ),
        None => menu
            .item(PopupMenuItem::new(t(text.open)).on_click(emitter(tree, FolderTreeEvent::Open(key.clone()))))
            .item(
                PopupMenuItem::new(t(text.duplicate)).on_click(emitter(tree, FolderTreeEvent::Duplicate(key.clone()))),
            ),
    };
    menu.separator()
        .item(PopupMenuItem::new(t(text.rename)).on_click(emitter(tree, FolderTreeEvent::Rename(key.clone()))))
        .item(PopupMenuItem::new(t(text.delete)).on_click(emitter(tree, FolderTreeEvent::Delete(key))))
}

fn render_row<S: FolderTreeSource>(
    ix: usize,
    entry: &TreeEntry,
    selected: bool,
    model: &Entity<AppModel>,
    tree: &WeakEntity<FolderTree<S>>,
    cx: &mut App,
) -> ListItem {
    let theme = cx.theme().clone();
    let key = S::Key::parse(entry.item().id.as_str());
    let label = entry.item().label.clone();
    let (deleted, locked, pending, lead) = {
        let app = model.read(cx);
        let deleted = key.as_ref().is_some_and(|key| S::is_deleted(app, key));
        let locked = key.as_ref().is_some_and(|key| S::locked_to(app, key).is_some());
        // An entity with changes that are not published yet says so with a mark, not only a color.
        let pending = key
            .as_ref()
            .and_then(FolderTreeKey::item_id)
            .is_some_and(|id| app.git().entity_state(id).is_some_and(|state| state != FileState::Clean));
        let lead = key.as_ref().filter(|key| key.item_id().is_some()).and_then(|key| S::lead(app, key));
        (deleted, locked, pending, lead)
    };
    let muted = |icon: IconName| Icon::new(icon).small().text_color(theme.muted_foreground);
    let lead: AnyElement = match (&key, lead) {
        (Some(key), _) if key.folder_id().is_some() => {
            muted(if entry.is_expanded() { IconName::FolderOpen } else { IconName::Folder }).into_any_element()
        }
        // Saved by a newer Bifurc: the sign for an update stands where the lead would, which this one cannot
        // vouch for.
        (_, Some(Lead::Icon(_))) if locked => muted(IconName::ArrowUp).into_any_element(),
        (_, _) if locked => div().w_12().flex_none().child(muted(IconName::ArrowUp)).into_any_element(),
        (_, Some(Lead::Badge(badge))) => {
            div().w_12().flex_none().text_xs().text_color(theme.muted_foreground).child(badge).into_any_element()
        }
        (_, Some(Lead::Icon(icon))) => muted(icon).into_any_element(),
        (_, None) => div().into_any_element(),
    };
    let trailing = key.as_ref().filter(|_| !deleted && !locked).and_then(|key| S::trailing(model, key, cx));

    let has_trailing = trailing.is_some();
    let open = key.clone().filter(|key| key.item_id().is_some());
    let folder = key.as_ref().and_then(|key| drop_folder::<S>(model.read(cx), key));
    let (click_tree, drag_tree, drop_tree, drop_model) = (tree.clone(), tree.clone(), tree.clone(), model.clone());
    // Rows are named after what they stand for, which is also how UI tests find one.
    let row_id: gpui_kit::ElementId = match key.as_ref().and_then(|key| key.item_id().or(key.folder_id())) {
        Some(id) => gpui_kit::ElementId::Name(format!("tree-row-{id}").into()),
        None => ix.into(),
    };
    let row = ListItem::new(row_id)
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
                .children(pending.then(|| {
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.warning)
                        .when(!has_trailing, |dot| dot.pr_2())
                        .child("●")
                }))
                .children(trailing.map(|trailing| div().flex_none().pr_2().child(trailing))),
        )
        .on_click(move |_, _, cx| {
            if let Some(key) = open.clone() {
                click_tree.update(cx, |_, cx| cx.emit(FolderTreeEvent::Open(key))).ok();
            }
        });
    // A row dropped on an item goes to the folder that item is in.
    let row = match folder {
        Some(folder) => {
            let target = folder.clone();
            tree_drag::drop_target(
                row,
                move |dragged: &S::Key, cx| accepts::<S>(drop_model.read(cx), dragged, Some(&target)),
                move |dragged: &S::Key, _, cx| {
                    let (key, folder) = (dragged.clone(), Some(folder.clone()));
                    drop_tree.update(cx, |this, cx| this.drop_on(key, folder, cx)).ok();
                },
            )
        }
        None => row,
    };
    // What is deleted stays where it is until it is reverted, and what is locked until Bifurc is updated.
    match key.filter(|_| !deleted && !locked) {
        Some(key) => tree_drag::draggable(row, key, label, move |key: &S::Key, cx| {
            let key = key.clone();
            drag_tree.update(cx, |this, cx| this.drag_started(key, cx)).ok();
        }),
        None => row,
    }
}

impl<S: FolderTreeSource> Render for FolderTree<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let (row_tree, menu_tree) = (this.clone(), this.clone());
        let (model, menu_model) = (self.model.clone(), self.model.clone());
        let filtering = !self.filter.read(cx).value().trim().is_empty();
        if !cx.has_active_drag() {
            self.dragging = None;
        }
        // A folder that is in a folder can be dragged out to the top level.
        let top_level =
            self.dragging.as_ref().filter(|dragged| accepts::<S>(self.model.read(cx), dragged, None)).map(|_| {
                let (drop_model, drop_tree) = (self.model.clone(), this.clone());
                tree_drag::top_level_target(
                    "folder-tree-top-level",
                    move |dragged: &S::Key, cx| accepts::<S>(drop_model.read(cx), dragged, None),
                    move |dragged: &S::Key, _, cx| {
                        let key = dragged.clone();
                        drop_tree.update(cx, |this, cx| this.drop_on(key, None, cx)).ok();
                    },
                    cx,
                )
            });

        let text = &S::TEXT;
        let body = if S::is_empty(self.model.read(cx)) {
            page::empty(t(text.empty_title), t(text.empty_description))
        } else if filtering && self.tree.read(cx).entry(0).is_none() {
            page::empty(t(text.no_matches_title), t(text.no_matches_description))
        } else {
            tree(&self.tree, move |ix, entry, selected, _, cx| {
                render_row::<S>(ix, entry, selected, &model, &row_tree, cx)
            })
            .context_menu(move |_, entry, menu, _, cx| context_menu::<S>(entry, menu, &menu_tree, &menu_model, cx))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RequestKind;

    fn node(key: &str, children: Vec<TreeNode>) -> TreeNode {
        TreeNode { key: key.into(), label: key.into(), children }
    }

    fn nodes() -> Vec<TreeNode> {
        vec![node("folder:a", vec![node("folder:b", vec![node("item:1", vec![])])]), node("item:2", vec![])]
    }

    #[test]
    fn folders_are_open_when_the_user_opened_them() {
        let expanded: HashSet<String> = ["folder:b".to_owned()].into();
        let items = to_items(&nodes(), &expanded, false);
        assert!(!items[0].is_expanded());
        assert!(items[0].children[0].is_expanded());
        assert_eq!(items[0].children[0].children[0].id.as_str(), "item:1");
        assert_eq!(items[1].id.as_str(), "item:2");
    }

    #[test]
    fn a_filter_opens_every_folder() {
        let items = to_items(&nodes(), &HashSet::new(), true);
        assert!(items[0].is_expanded() && items[0].children[0].is_expanded());
    }

    #[test]
    fn flipping_opens_a_closed_folder_and_closes_an_open_one() {
        let mut expanded = HashSet::new();
        flip(&mut expanded, "folder:a".into());
        assert!(expanded.contains("folder:a"));
        flip(&mut expanded, "folder:a".into());
        assert!(expanded.is_empty());
    }

    #[test]
    fn both_key_kinds_tell_folders_from_items() {
        let folder = TreeKey::Folder("f".into());
        let item = TreeKey::Item("i".into());
        assert_eq!((folder.folder_id(), folder.item_id()), (Some("f"), None));
        assert_eq!((item.folder_id(), item.item_id()), (None, Some("i")));
        let folder = NodeRef::Folder { id: "f".into() };
        let request = NodeRef::Request { kind: RequestKind::Rest, id: "r".into() };
        assert_eq!((folder.folder_id(), folder.item_id()), (Some("f"), None));
        assert_eq!((request.folder_id(), request.item_id()), (None, Some("r")));
        assert_eq!(<NodeRef as FolderTreeKey>::parse(&FolderTreeKey::encode(&request)), Some(request));
    }
}
