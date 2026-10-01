//! The Mocks screen: the folder tree of mock rules, the open ones as tabs, and the active rule and its
//! response as docks.
//!
//! It has the shape of the Requests screen for the same reason: a mock is a saved entity in folders, opened
//! to be edited. The tree comes from the model's snapshot; an open mock is a [`MockEditor`].

mod editor;

use std::{collections::HashSet, rc::Rc};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    dock::{DockArea, DockLayout, DockSkin, panel_handle},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    switch::Switch,
    tab::{Tab, TabBar},
    tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::{
    AnyView, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, WeakEntity, Window, div, px, rems,
};
use serde_json::{Map, json};

use self::editor::{MockEditor, MockEvent};
use crate::{
    app_model::{AppModel, Load, ModelEvent},
    components::{
        EntityAction, EntityBar, FolderPicker,
        dialogs::{PromptField, confirm_danger, prompt},
        entity_bar, page,
    },
    engine::{call, failure, patch_entity},
    model::{
        git::FileState,
        tree::{ItemRow, TreeKey, TreeNode, build_tree, folder_path},
    },
    panels::host::{HostPanel, PartView},
    preferences::Preferences,
    session::SaveMode,
    shell::ActiveTarget,
};

/// How a tab is identified before and after it has an id from the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TabKey {
    Saved(String),
    Draft(u64),
}

struct OpenTab {
    key: TabKey,
    editor: Entity<MockEditor>,
    rule: AnyView,
    response: AnyView,
    _subscription: Subscription,
}

pub struct MocksScreen {
    model: Entity<AppModel>,
    filter: Entity<InputState>,
    tree: Entity<TreeState>,
    /// Folders the user has opened, by tree key, so a rebuild does not collapse them.
    expanded: HashSet<String>,
    selected: Option<String>,
    tabs: Vec<OpenTab>,
    active: Option<usize>,
    dock: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    rule_panel: Entity<HostPanel>,
    response_panel: Entity<HostPanel>,
    next_draft: u64,
    _subscriptions: Vec<Subscription>,
}

impl MocksScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter mocks"));
        let tree = cx.new(|cx| TreeState::new(cx));
        let (dock, skin) = DockSkin::dock_area("mocks-dock", Some(1), window, cx);
        let rule_panel =
            cx.new(|cx| HostPanel::new("MockRulePanel", "Rule", "No mock open", "Open a mock to edit it here.", cx));
        let response_panel = cx.new(|cx| {
            HostPanel::new("MockResponsePanel", "Response", "No mock open", "A mock's response is edited here.", cx)
        });
        // Dock sizes are physical starting points the user resizes; they are not part of the rem scale.
        dock.update(cx, |area, cx| {
            area.set_center(
                DockLayout::v_split()
                    .child(DockLayout::tabs().panel_view(panel_handle(rule_panel.clone()), cx), Some(px(230.)))
                    .child(DockLayout::tabs().panel_view(panel_handle(response_panel.clone()), cx), None),
                window,
                cx,
            );
        });

        let subscriptions = vec![
            cx.subscribe(&model, |this, _, event: &ModelEvent, cx| match event {
                ModelEvent::ConfigChanged => this.close_vanished_tabs(cx),
                ModelEvent::WorkspaceSwitched => {
                    this.tabs.clear();
                    this.set_active(None, cx);
                }
                _ => {}
            }),
            cx.observe(&model, |this, _, cx| {
                this.rebuild(cx);
                cx.notify();
            }),
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
        ];
        let mut this = Self {
            model,
            filter,
            tree,
            expanded: HashSet::new(),
            selected: None,
            tabs: Vec::new(),
            active: None,
            dock,
            _skin: skin,
            rule_panel,
            response_panel,
            next_draft: 1,
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
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
        let folders = config.mock_folder_rows();
        let items: Vec<ItemRow> = config
            .mocks
            .iter()
            .map(|m| ItemRow {
                id: &m.id,
                label: if m.name.is_empty() { m.url_pattern.clone() } else { m.name.clone() },
                folder_id: m.folder_id.as_deref(),
                detail: &m.url_pattern,
            })
            .collect();
        let nodes = build_tree(&folders, &items, &filter);
        let filtering = !filter.trim().is_empty();
        let tree_items: Vec<TreeItem> = nodes.iter().map(|node| self.to_item(node, filtering)).collect();
        let selected: Option<SharedString> = self.selected.clone().map(Into::into);
        self.tree.update(cx, |tree, cx| {
            tree.set_items(tree_items, cx);
            if let Some(ix) = selected.as_ref().and_then(|id| tree.index_of(id)) {
                tree.set_selected_index(Some(ix), cx);
            }
        });
    }

    pub fn active_editor(&self) -> Option<&Entity<MockEditor>> {
        self.active.and_then(|ix| self.tabs.get(ix)).map(|tab| &tab.editor)
    }

    /// The open mock, for the breadcrumb and the right sidebar.
    pub fn active_target(&self, cx: &App) -> Option<ActiveTarget> {
        let editor = self.active_editor()?.read(cx);
        let folders = self.model.read(cx).config().mock_folder_rows();
        Some(ActiveTarget {
            folders: editor.folder_id().map(|id| folder_path(&folders, id)).unwrap_or_default(),
            ..ActiveTarget::named(editor.title(cx), editor.saved_id().map(str::to_owned), editor.name_input().clone())
        })
    }

    fn set_active(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        self.active = ix.filter(|ix| *ix < self.tabs.len());
        let (rule, response) = match self.active.and_then(|ix| self.tabs.get(ix)) {
            Some(tab) => (Some(tab.rule.clone()), Some(tab.response.clone())),
            None => (None, None),
        };
        self.rule_panel.update(cx, |panel, cx| panel.set_view(rule, cx));
        self.response_panel.update(cx, |panel, cx| panel.set_view(response, cx));
        cx.notify();
    }

    pub fn open_mock(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let key = TabKey::Saved(id.to_owned());
        if let Some(ix) = self.tabs.iter().position(|tab| tab.key == key) {
            self.set_active(Some(ix), cx);
            return;
        }
        let (model, id) = (self.model.clone(), id.to_owned());
        let editor = cx.new(|cx| MockEditor::open_saved(model, id, window, cx));
        self.add_tab(key, editor, cx);
    }

    pub fn new_mock(&mut self, folder: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let editor = cx.new(|cx| MockEditor::new_draft(model, folder, window, cx));
        let key = TabKey::Draft(self.next_draft);
        self.next_draft += 1;
        self.add_tab(key, editor, cx);
    }

    fn add_tab(&mut self, key: TabKey, editor: Entity<MockEditor>, cx: &mut Context<Self>) {
        let subscription = cx.subscribe(&editor, |this, editor, event: &MockEvent, cx| {
            if let MockEvent::Saved { id } = event {
                if let Some(tab) = this.tabs.iter_mut().find(|tab| tab.editor == editor) {
                    tab.key = TabKey::Saved(id.clone());
                }
                this.selected = Some(TreeKey::Item(id.clone()).encode());
            }
            cx.notify();
        });
        let rule = PartView::new(&editor, MockEditor::render_rule, cx).into();
        let response = PartView::new(&editor, MockEditor::render_response, cx).into();
        self.tabs.push(OpenTab { key, editor, rule, response, _subscription: subscription });
        self.set_active(Some(self.tabs.len() - 1), cx);
    }

    fn remove_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        let next = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > ix => Some(active - 1),
            Some(active) => Some(active.min(self.tabs.len() - 1)),
            None => None,
        };
        self.set_active(next, cx);
    }

    /// Closes a tab. A saved mock has nothing to lose: an edit still waiting is written first. A draft with
    /// edits asks first, since closing it discards them.
    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        let editor = tab.editor.clone();
        if editor.read(cx).saved_id().is_some() {
            editor.update(cx, |editor, cx| editor.flush(window, cx));
            self.remove_tab(ix, cx);
            return;
        }
        if !editor.read(cx).is_dirty(cx) {
            self.remove_tab(ix, cx);
            return;
        }
        let title = editor.read(cx).title(cx);
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            format!("Discard changes to “{title}”?"),
            "The edits in this tab have not been saved.",
            "Discard changes",
            move |this, _, cx| {
                if let Some(ix) = this.tabs.iter().position(|tab| tab.editor == editor) {
                    this.remove_tab(ix, cx);
                }
            },
        );
    }

    /// Does what the footer asked for to the mock in the active tab.
    pub fn entity_action(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        let Some(editor) = self.active_editor().cloned() else { return };
        let saved_id = editor.read(cx).saved_id().map(str::to_owned);
        match action {
            EntityAction::Save => editor.update(cx, |editor, cx| editor.save(SaveMode::Explicit, window, cx)),
            EntityAction::Move(folder) => editor.update(cx, |editor, cx| editor.set_folder(folder, cx)),
            EntityAction::Close => self.close_tab(ix, window, cx),
            EntityAction::Sync => {
                if let Some(id) = saved_id {
                    self.model.update(cx, |model, cx| model.sync_entity(&id, cx));
                }
            }
            EntityAction::Revert => {
                let Some(id) = saved_id else { return };
                let name = editor.read(cx).title(cx);
                confirm_danger(
                    cx.weak_entity(),
                    window,
                    cx,
                    format!("Revert “{name}” to the last synced version?"),
                    "The changes saved since then are discarded.",
                    "Revert",
                    move |this, _, cx| this.model.update(cx, |model, cx| model.revert_entity(&id, cx)),
                );
            }
            EntityAction::Delete => match saved_id {
                Some(id) => self.confirm_delete(TreeKey::Item(id), window, cx),
                None => self.close_tab(ix, window, cx),
            },
        }
    }

    /// A saved mock that is gone from the workspace has nothing left to show, edited or not: writing an
    /// edit to it would only fail. A draft stays: saving it creates the mock.
    fn close_vanished_tabs(&mut self, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        if !matches!(model.load(), Load::Ready) {
            return;
        }
        let gone: Vec<Entity<MockEditor>> = self
            .tabs
            .iter()
            .filter(|tab| match &tab.key {
                TabKey::Saved(id) => model.config().mock(id).is_none(),
                TabKey::Draft(_) => false,
            })
            .map(|tab| tab.editor.clone())
            .collect();
        for editor in gone {
            if let Some(ix) = self.tabs.iter().position(|tab| tab.editor == editor) {
                self.remove_tab(ix, cx);
            }
        }
    }

    fn report(&mut self, what: &str, message: String, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.notice(format!("{what} {message}"), cx));
    }

    pub fn prompt_new_folder(&mut self, parent: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            "New mock folder",
            "Create",
            vec![PromptField::new("Name", "Folder name")],
            move |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let payload = json!({
                    "kind": "mock", "name": values[0], "parentId": parent, "workspaceId": model.workspace_id(),
                });
                call(&client, cx, "folder.add", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report("Couldn't create the folder.", message, cx);
                    }
                });
            },
        );
    }

    fn name_of(&self, key: &TreeKey, cx: &App) -> String {
        let config = self.model.read(cx).config();
        match key {
            TreeKey::Folder(id) => config.mock_folders.iter().find(|f| f.id == *id).map(|f| f.name.clone()),
            TreeKey::Item(id) => config.mock(id).map(|m| m.name.clone()),
        }
        .unwrap_or_default()
    }

    fn prompt_rename(&mut self, key: TreeKey, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::{WindowExt as _, notification::Notification};
        if let TreeKey::Item(id) = &key
            && self.tabs.iter().any(|tab| tab.key == TabKey::Saved(id.clone()))
        {
            window.push_notification(
                Notification::info("This mock is open. Change its name in the breadcrumb above the tabs.")
                    .title("Rename in the breadcrumb"),
                cx,
            );
            return;
        }
        let current = self.name_of(&key, cx);
        prompt(
            cx.weak_entity(),
            window,
            cx,
            format!("Rename “{current}”"),
            "Rename",
            vec![PromptField::new("Name", "Name").initial(current.clone())],
            move |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                match &key {
                    TreeKey::Folder(id) => {
                        let payload = json!({ "kind": "mock", "id": id, "name": values[0] });
                        call(&client, cx, "folder.rename", payload, |this, outcome, cx| {
                            if let Some(message) = failure(&outcome) {
                                this.report("Couldn't rename the folder.", message, cx);
                            }
                        });
                    }
                    TreeKey::Item(id) => {
                        let workspace_id = model.workspace_id().to_owned();
                        let mut patch = Map::new();
                        patch.insert("name".into(), json!(values[0]));
                        patch_entity(&client, cx, &workspace_id, "mocks", id, patch, |this, outcome, cx| {
                            if let Err(message) = outcome {
                                this.report("Couldn't rename the mock.", message, cx);
                            }
                        });
                    }
                }
            },
        );
    }

    fn confirm_delete(&mut self, key: TreeKey, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_of(&key, cx);
        let (what, consequence) = match &key {
            TreeKey::Folder(_) => ("folder", "The folder and every mock in it will be deleted."),
            TreeKey::Item(_) => ("mock", "Requests it answered will reach the real server again."),
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
                let (action, payload) = match &key {
                    TreeKey::Folder(id) => ("folder.delete", json!({ "kind": "mock", "id": id })),
                    TreeKey::Item(id) => ("entity.delete", json!({ "kind": "mocks", "id": id })),
                };
                call(&client, cx, action, payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report("Couldn't delete.", message, cx);
                    }
                });
            },
        );
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let (row_view, menu_view, new_view) = (this.clone(), this.clone(), this.clone());
        let model = self.model.clone();
        let config = self.model.read(cx).config();
        let has_mocks = !config.mocks.is_empty() || !config.mock_folders.is_empty();
        let filtering = !self.filter.read(cx).value().trim().is_empty();

        let body = if !has_mocks {
            page::empty("No mocks yet", "Use + to create the first one.")
        } else if filtering && self.tree.read(cx).entry(0).is_none() {
            page::empty("No matches", "No mock matches the filter.")
        } else {
            tree(&self.tree, move |ix, entry, selected, _, cx| render_row(ix, entry, selected, &model, &row_view, cx))
                .context_menu(move |_, entry, menu, _, _| context_menu(entry, menu, &menu_view))
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
                        Button::new("mock-tree-new")
                            .icon(IconName::Plus)
                            .ghost()
                            .small()
                            .tooltip("New mock or folder")
                            .dropdown_menu(move |menu, _, _| new_menu(menu, &new_view, None)),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
    }

    fn render_tab_strip(&self, tree_open: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let this = cx.weak_entity();
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let editor = tab.editor.read(cx);
            let title = editor.title(cx);
            let label: SharedString = if editor.is_dirty(cx) { format!("{title} ●").into() } else { title };
            Tab::new()
                .label(label)
                .prefix(div().pl_2().text_xs().text_color(theme.muted_foreground).child(editor.badge(cx)))
                .suffix(div().pr_1().child(
                    Button::new(("close-mock-tab", ix)).icon(IconName::Close).ghost().xsmall().on_click(cx.listener(
                        move |this, _, window, cx| {
                            // The tab under the button would otherwise take the click as a selection.
                            cx.stop_propagation();
                            this.close_tab(ix, window, cx);
                        },
                    )),
                ))
        });
        TabBar::new("mock-tabs")
            .small()
            .menu(true)
            .selected_index(self.active.unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, _, cx| this.set_active(Some(*ix), cx)))
            .prefix(
                h_flex().px_1().child(
                    Button::new("toggle-mock-tree")
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
            .suffix(h_flex().px_1().child(
                Button::new("new-mock-tab").icon(IconName::Plus).ghost().small().tooltip("New mock").on_click(
                    move |_, window, cx| {
                        this.update(cx, |this, cx| this.new_mock(None, window, cx)).ok();
                    },
                ),
            ))
    }
}

fn act(
    view: &WeakEntity<MocksScreen>,
    f: impl Fn(&mut MocksScreen, &mut Window, &mut Context<MocksScreen>) + 'static,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let view = view.clone();
    move |_, window, cx| {
        view.update(cx, |this, cx| f(this, window, cx)).ok();
    }
}

fn new_menu(menu: PopupMenu, view: &WeakEntity<MocksScreen>, folder: Option<String>) -> PopupMenu {
    let parent = folder.clone();
    menu.item(
        PopupMenuItem::new("Mock")
            .on_click(act(view, move |this, window, cx| this.new_mock(folder.clone(), window, cx))),
    )
    .item(
        PopupMenuItem::new("Folder…")
            .on_click(act(view, move |this, window, cx| this.prompt_new_folder(parent.clone(), window, cx))),
    )
}

fn context_menu(entry: &TreeEntry, menu: PopupMenu, view: &WeakEntity<MocksScreen>) -> PopupMenu {
    let Some(key) = TreeKey::parse(entry.item().id.as_str()) else { return menu };
    let (rename, delete) = (key.clone(), key.clone());
    let menu = match &key {
        TreeKey::Item(id) => {
            let id = id.clone();
            menu.item(
                PopupMenuItem::new("Open").on_click(act(view, move |this, window, cx| this.open_mock(&id, window, cx))),
            )
        }
        TreeKey::Folder(id) => {
            let (mock_parent, folder_parent) = (id.clone(), id.clone());
            menu.item(
                PopupMenuItem::new("New mock here")
                    .on_click(act(view, move |this, window, cx| this.new_mock(Some(mock_parent.clone()), window, cx))),
            )
            .item(PopupMenuItem::new("New folder here…").on_click(act(view, move |this, window, cx| {
                this.prompt_new_folder(Some(folder_parent.clone()), window, cx)
            })))
        }
    };
    menu.separator()
        .item(
            PopupMenuItem::new("Rename…")
                .on_click(act(view, move |this, window, cx| this.prompt_rename(rename.clone(), window, cx))),
        )
        .item(
            PopupMenuItem::new("Delete…")
                .on_click(act(view, move |this, window, cx| this.confirm_delete(delete.clone(), window, cx))),
        )
}

fn render_row(
    ix: usize,
    entry: &TreeEntry,
    selected: bool,
    model: &Entity<AppModel>,
    view: &WeakEntity<MocksScreen>,
    cx: &mut App,
) -> ListItem {
    let theme = cx.theme().clone();
    let key = TreeKey::parse(entry.item().id.as_str());
    let label = entry.item().label.clone();
    let mut pending = false;
    let mut toggle: Option<gpui_kit::AnyElement> = None;
    let lead: gpui_kit::AnyElement = match &key {
        Some(TreeKey::Folder(_)) => {
            Icon::new(if entry.is_expanded() { IconName::FolderOpen } else { IconName::Folder })
                .small()
                .text_color(theme.muted_foreground)
                .into_any_element()
        }
        Some(TreeKey::Item(id)) => {
            let app_model = model.read(cx);
            pending = app_model.git().entity_state(id).is_some_and(|state| state != FileState::Clean);
            let mock = app_model.config().mock(id);
            let method = mock.map(|m| if m.method == "*" { "ANY".to_owned() } else { m.method.clone() });
            let enabled = mock.is_some_and(|m| m.enabled);
            let (toggle_model, toggle_id) = (model.clone(), id.clone());
            toggle = Some(
                Switch::new(gpui_kit::ElementId::Name(format!("mock-enabled-{id}").into()))
                    .checked(enabled)
                    .small()
                    .on_change(move |checked: &bool, _, cx| {
                        // Turning a mock on or off is not opening it.
                        cx.stop_propagation();
                        let checked = *checked;
                        toggle_model.update(cx, |m, cx| m.set_enabled("mocks", &toggle_id, checked, cx));
                    })
                    .into_any_element(),
            );
            div()
                .w_12()
                .flex_none()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(method.unwrap_or_default())
                .into_any_element()
        }
        None => div().into_any_element(),
    };

    let open = match key {
        Some(TreeKey::Item(id)) => Some(id),
        _ => None,
    };
    let view = view.clone();
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
                .children(pending.then(|| div().flex_none().text_xs().text_color(theme.warning).child("●")))
                .children(toggle.map(|toggle| div().flex_none().pr_2().child(toggle))),
        )
        .on_click(move |_, window, cx| {
            if let Some(id) = open.clone() {
                view.update(cx, |this, cx| this.open_mock(&id, window, cx)).ok();
            }
        })
}

impl Render for MocksScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree_open = Preferences::global(cx).folder_tree_open;
        let body = if self.tabs.is_empty() {
            page::empty_with_action(
                "No mock open",
                "A mock answers matching requests through the proxy instead of the real server.",
                Button::new("welcome-new-mock")
                    .label("New mock")
                    .on_click(cx.listener(|this, _, window, cx| this.new_mock(None, window, cx))),
            )
        } else {
            self.dock.clone().into_any_element()
        };
        let bar = self.active_editor().map(|editor| {
            let editor = editor.read(cx);
            let saved_id = editor.saved_id().map(str::to_owned);
            EntityBar {
                saving: editor.is_saving(),
                unsaved: saved_id.is_some() && editor.is_dirty(cx),
                problem: editor.problem(cx),
                closable: true,
                folder: Some(FolderPicker::new(
                    editor.folder_id().map(str::to_owned),
                    &self.model.read(cx).config().mock_folder_rows(),
                )),
                ..EntityBar::of(self.model.read(cx), saved_id.as_deref())
            }
        });
        let footer = bar.map(|bar| {
            entity_bar(bar, cx, |this, action, window, cx| this.entity_action(action, window, cx)).into_any_element()
        });
        let main = v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_tab_strip(tree_open, cx))
            .child(div().flex_1().min_h_0().child(body))
            .children(footer);
        // The split's sizes are physical starting points the user resizes, not part of the rem scale.
        h_resizable("mocks-split")
            .child(
                resizable_panel()
                    .visible(tree_open)
                    .size(px(260.))
                    .size_range(px(180.)..px(520.))
                    .child(self.render_tree(cx)),
            )
            .child(resizable_panel().child(main))
    }
}
