//! The Mocks screen: the folder tree of mock rules, the open ones as tabs, and the active rule and its
//! response as docks.
//!
//! It has the shape of the Requests screen for the same reason: a mock is a saved entity in folders, opened
//! to be edited. The tree comes from the model's snapshot; an open mock is a [`MockEditor`].

mod editor;

use std::{collections::HashSet, rc::Rc};

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    dock::{DockArea, DockLayout, DockSkin, panel_handle},
    h_flex,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    switch::Switch,
    tree::{TreeEntry, TreeEvent, TreeItem, TreeState, tree},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyView, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, ScrollStrategy,
    SharedString, Styled as _, Subscription, WeakEntity, Window, div, px, rems,
};
use serde_json::{Map, Value, json};

use self::editor::{MockEditor, MockEvent};
use crate::{
    app_model::{AppModel, Deleted, Load, ModelEvent},
    components::{
        CURL_TAB_BADGE, CURL_TAB_TITLE, CurlImport, CurlImportEvent, CurlTarget, EntityAction, EntityBar, FolderPicker,
        StripTab, TabAction, TabStrip,
        dialogs::{
            FOLDER_DELETE_CONSEQUENCE, PromptField, confirm_danger, delete_consequence, needs_update, prompt,
            resolve_deleted,
        },
        entity_bar, page, tab_strip, tree_drag,
    },
    engine::{call, call_in, failure, patch_entity},
    i18n::{t, t_with},
    model::{
        git::FileState,
        tree::{FolderRow, ItemRow, Moved, TreeKey, TreeNode, build_tree, can_move, folder_path},
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
    /// The box asking for a cURL command, while the tab is waiting for one. It fills the tab until the
    /// engine has turned the command into a mock, which the editor then holds.
    curl: Option<(Entity<CurlImport>, Subscription)>,
    _subscription: Subscription,
}

impl OpenTab {
    /// A draft that was typed into: closing its tab loses what was typed.
    fn is_unsaved_draft(&self, cx: &App) -> bool {
        let editor = self.editor.read(cx);
        editor.saved_id().is_none() && editor.is_dirty(cx)
    }
}

pub struct MocksScreen {
    model: Entity<AppModel>,
    filter: Entity<InputState>,
    tree: Entity<TreeState>,
    /// Folders the user has opened, by tree key, so a rebuild does not collapse them.
    expanded: HashSet<String>,
    selected: Option<String>,
    /// The tree row being dragged, while a drag is on.
    dragging: Option<TreeKey>,
    tabs: Vec<OpenTab>,
    active: Option<usize>,
    tab_scroll: VirtualListScrollHandle,
    dock: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    rule_panel: Entity<HostPanel>,
    response_panel: Entity<HostPanel>,
    next_draft: u64,
    _subscriptions: Vec<Subscription>,
}

impl MocksScreen {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(t("mocks.filter.placeholder")));
        let tree = cx.new(|cx| TreeState::new(cx));
        let (dock, skin) = DockSkin::dock_area("mocks-dock", Some(1), window, cx);
        let rule_panel =
            cx.new(|cx| {
                HostPanel::new(
                    "MockRulePanel",
                    t("mocks.panel.rule_title"),
                    t("mocks.no_mock_open"),
                    t("mocks.panel.rule_empty"),
                    cx,
                )
            });
        let response_panel = cx.new(|cx| {
            HostPanel::new(
                "MockResponsePanel",
                t("mocks.panel.response_title"),
                t("mocks.no_mock_open"),
                t("mocks.panel.response_empty"),
                cx,
            )
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
            dragging: None,
            tabs: Vec::new(),
            active: None,
            tab_scroll: VirtualListScrollHandle::new(),
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
        // Deleted folders stay in the tree, so a deleted mock shows inside its deleted folder.
        let folders: Vec<FolderRow> = config
            .mock_folders
            .iter()
            .map(|f| FolderRow { id: &f.id, name: &f.name, parent_id: f.parent_id.as_deref() })
            .collect();
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

    /// The cURL box of the tab in front, while that tab is waiting for a command.
    fn active_curl(&self) -> Option<&Entity<CurlImport>> {
        let tab = self.active.and_then(|ix| self.tabs.get(ix))?;
        tab.curl.as_ref().map(|(import, _)| import)
    }

    /// Sends the command in the tab in front to the engine, when that tab is waiting for one.
    pub fn submit_curl(&mut self, cx: &mut Context<Self>) {
        if let Some(import) = self.active_curl().cloned() {
            import.update(cx, |import, cx| import.submit(cx));
        }
    }

    /// The open mock, for the breadcrumb and the right sidebar.
    pub fn active_target(&self, cx: &App) -> Option<ActiveTarget> {
        if self.active_curl().is_some() {
            return Some(ActiveTarget {
                title: t(CURL_TAB_TITLE).into(),
                entity_id: None,
                request: None,
                name: None,
                kind: None,
                folders: Vec::new(),
            });
        }
        let editor = self.active_editor()?.read(cx);
        let folders = self.model.read(cx).config().mock_folder_rows();
        Some(ActiveTarget {
            folders: editor.folder_id().map(|id| folder_path(&folders, id)).unwrap_or_default(),
            ..ActiveTarget::named(editor.title(cx), editor.saved_id().map(str::to_owned), editor.name_input().clone())
        })
    }

    /// Brings the tab `by` places away from the one in front to the front, wrapping around.
    pub fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let len = self.tabs.len() as isize;
        if len > 0 {
            self.set_active(Some((self.active.unwrap_or(0) as isize + by).rem_euclid(len) as usize), cx);
        }
    }

    fn set_active(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        self.active = ix.filter(|ix| *ix < self.tabs.len());
        if let Some(ix) = self.active {
            self.tab_scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        let (rule, response) = match self.active.and_then(|ix| self.tabs.get(ix)) {
            Some(tab) => (Some(tab.rule.clone()), Some(tab.response.clone())),
            None => (None, None),
        };
        self.rule_panel.update(cx, |panel, cx| panel.set_view(rule, cx));
        self.response_panel.update(cx, |panel, cx| panel.set_view(response, cx));
        cx.notify();
    }

    pub fn open_mock(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // A deleted mock has nothing to open: it is brought back or deleted for good.
        let key = TreeKey::Item(id.to_owned());
        if is_deleted(self.model.read(cx), &key) {
            let name = self.name_of(&key, cx);
            resolve_deleted(self.model.clone(), deleted_target(&key), name, window, cx);
            return;
        }
        // Neither has one a newer Bifurc saved: the engine does not read it until this one is updated.
        if let Some(version) = locked_to(self.model.read(cx), &key) {
            needs_update(self.name_of(&key, cx), version, window, cx);
            return;
        }
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

    /// Opens a tab that asks for a cURL command. The engine reads the command, and the tab becomes a draft
    /// of a mock of the call it makes.
    pub fn import_curl(&mut self, folder: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.new_mock(folder, window, cx);
        let model = self.model.clone();
        let import = cx.new(|cx| CurlImport::new(model, CurlTarget::Mock, window, cx));
        let subscription = cx.subscribe_in(&import, window, |this, import, event: &CurlImportEvent, window, cx| {
            let CurlImportEvent::Built(built) = event;
            this.adopt_curl(import, built, window, cx);
        });
        if let Some(tab) = self.tabs.last_mut() {
            tab.curl = Some((import.clone(), subscription));
        }
        import.update(cx, |import, cx| import.focus(window, cx));
        cx.notify();
    }

    /// Fills the tab `import` is in with what the engine built from its command: `{ mock }`.
    fn adopt_curl(&mut self, import: &Entity<CurlImport>, built: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let is_tab = |tab: &OpenTab| tab.curl.as_ref().is_some_and(|(open, _)| open == import);
        let Some(ix) = self.tabs.iter().position(is_tab) else { return };
        let Some(mock) = built.get("mock").and_then(Value::as_object).cloned() else {
            self.report(t("mocks.import.failed"), t("mocks.import.no_mock").to_owned(), cx);
            return;
        };
        let tab = &mut self.tabs[ix];
        tab.curl = None;
        tab.editor.clone().update(cx, |editor, cx| editor.adopt_import(mock, window, cx));
        cx.notify();
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
        self.tabs.push(OpenTab { key, editor, rule, response, curl: None, _subscription: subscription });
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
        if !tab.is_unsaved_draft(cx) {
            self.close_keys(vec![tab.key.clone()], window, cx);
            return;
        }
        let key = tab.key.clone();
        let title = tab.editor.read(cx).title(cx);
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            t_with("mocks.close.title_changes", &[("title", &title)]),
            t("mocks.close.body_changes"),
            t("mocks.close.button_changes"),
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
        let title = if unsaved == 1 {
            t("mocks.close.title_one").to_owned()
        } else {
            t_with("mocks.close.title_other", &[("count", &unsaved)])
        };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            title,
            t("mocks.close.body_unsaved"),
            t("mocks.close.button_discard"),
            move |this, window, cx| this.close_keys(keys.clone(), window, cx),
        );
    }

    /// Closes the tabs with these keys, writing a saved mock's waiting edit on the way out. The tab in
    /// front stays in front when it is not one of them.
    fn close_keys(&mut self, keys: Vec<TabKey>, window: &mut Window, cx: &mut Context<Self>) {
        let front = self.active.and_then(|ix| self.tabs.get(ix)).map(|tab| tab.key.clone());
        let position = self.active.unwrap_or(0);
        for tab in self.tabs.iter().filter(|tab| keys.contains(&tab.key)) {
            if matches!(tab.key, TabKey::Saved(_)) {
                tab.editor.update(cx, |editor, cx| editor.flush(window, cx));
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
                if let Some(TabKey::Saved(id)) = self.tabs.get(ix).map(|tab| tab.key.clone()) {
                    self.duplicate(id, window, cx);
                }
            }
        }
    }

    fn describe_tab(&self, ix: usize, cx: &App) -> Option<StripTab> {
        let tab = self.tabs.get(ix)?;
        let id = match &tab.key {
            TabKey::Saved(id) => format!("mock-{id}"),
            TabKey::Draft(number) => format!("draft-{number}"),
        };
        if tab.curl.is_some() {
            return Some(StripTab {
                id: id.into(),
                badge: CURL_TAB_BADGE.into(),
                title: t(CURL_TAB_TITLE).into(),
                dirty: false,
                can_duplicate: false,
            });
        }
        let editor = tab.editor.read(cx);
        Some(StripTab {
            id: id.into(),
            badge: editor.badge(cx).into(),
            title: editor.title(cx),
            dirty: editor.is_dirty(cx),
            can_duplicate: matches!(tab.key, TabKey::Saved(_)),
        })
    }

    /// Copies a saved mock. The engine makes the copy, named "<name> copy", in the same folder and switched
    /// off, so the original keeps answering; it is then opened.
    fn duplicate(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        // An edit still waiting to be written belongs in the copy too.
        let open = self.tabs.iter().find(|tab| tab.key == TabKey::Saved(id.clone())).map(|tab| tab.editor.clone());
        if let Some(editor) = open {
            editor.update(cx, |editor, cx| editor.flush(window, cx));
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let payload = json!({ "workspaceId": model.workspace_id(), "kind": "mocks", "id": id });
        call_in(&client, window, cx, "entity.duplicate", payload, move |this, outcome, window, cx| {
            if let Some(message) = failure(&outcome) {
                this.report(t("mocks.error.duplicate"), message, cx);
                return;
            }
            let copy = outcome
                .ok()
                .and_then(|result| result.get("entity")?.get("id")?.as_str().map(str::to_owned));
            let Some(copy) = copy else { return };
            // The copy is in the engine but not yet in this window's snapshot, which the tree is built from.
            this.model.update(cx, |model, cx| model.refresh(cx));
            this.selected = Some(TreeKey::Item(copy.clone()).encode());
            this.open_mock(&copy, window, cx);
        });
    }

    /// Does what the footer asked for to the mock in the active tab.
    pub fn entity_action(&mut self, action: EntityAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else { return };
        // A tab waiting for a command holds no mock yet: the only thing to do with it is close it.
        if self.active_curl().is_some() {
            if action == EntityAction::Close {
                self.close_tab(ix, window, cx);
            }
            return;
        }
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
                    t_with("mocks.revert.title", &[("name", &name)]),
                    t("mocks.revert.body"),
                    t("mocks.revert.button"),
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
                // Deleted and still listed counts as gone: there is no file an edit could be written to. So does
                // one a newer Bifurc saved since the tab was opened: the engine would refuse the edit.
                TabKey::Saved(id) => model.config().mock(id).is_none_or(|m| m.deleted || m.locked.is_some()),
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
        let text = t_with("mocks.report", &[("what", &what), ("message", &message)]);
        self.model.update(cx, |model, cx| model.notice(text, cx));
    }

    pub fn prompt_new_folder(&mut self, parent: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t("mocks.prompt.new_folder_title"),
            t("mocks.prompt.create"),
            vec![PromptField::new(t("mocks.prompt.name"), t("mocks.prompt.folder_name"))],
            move |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                let payload = json!({
                    "kind": "mock", "name": values[0], "parentId": parent, "workspaceId": model.workspace_id(),
                });
                call(&client, cx, "folder.add", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report(t("mocks.error.create_folder"), message, cx);
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
                Notification::info(t("mocks.rename.open_body")).title(t("mocks.rename.open_title")),
                cx,
            );
            return;
        }
        let current = self.name_of(&key, cx);
        prompt(
            cx.weak_entity(),
            window,
            cx,
            t_with("mocks.rename.title", &[("current", &current)]),
            t("mocks.rename.button"),
            vec![PromptField::new(t("mocks.prompt.name"), t("mocks.prompt.name")).initial(current.clone())],
            move |this, values, _, cx| {
                let model = this.model.read(cx);
                let client = model.client().clone();
                match &key {
                    TreeKey::Folder(id) => {
                        let payload = json!({ "kind": "mock", "id": id, "name": values[0] });
                        call(&client, cx, "folder.rename", payload, |this, outcome, cx| {
                            if let Some(message) = failure(&outcome) {
                                this.report(t("mocks.error.rename_folder"), message, cx);
                            }
                        });
                    }
                    TreeKey::Item(id) => {
                        let workspace_id = model.workspace_id().to_owned();
                        let mut patch = Map::new();
                        patch.insert("name".into(), json!(values[0]));
                        patch_entity(&client, cx, &workspace_id, "mocks", id, patch, |this, outcome, cx| {
                            if let Err(message) = outcome {
                                this.report(t("mocks.error.rename_mock"), message, cx);
                            }
                        });
                    }
                }
            },
        );
    }

    /// A tree row was picked up. Pressing a folder row opened or closed it before the press turned into a
    /// drag: it is put back as it was.
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

    /// A row was dropped on a folder, or on the top level: it is put there. The folder is opened, so the
    /// row is seen landing. A mock that is open is moved by its own next write, so the tab and the file
    /// cannot disagree about where it is.
    fn move_node(&mut self, key: TreeKey, folder: Option<String>, cx: &mut Context<Self>) {
        self.dragging = None;
        if let Some(id) = &folder {
            self.expanded.insert(TreeKey::Folder(id.clone()).encode());
        }
        let model = self.model.read(cx);
        let client = model.client().clone();
        let workspace_id = model.workspace_id().to_owned();
        match key {
            TreeKey::Folder(id) => {
                let payload = json!({ "kind": "mock", "id": id, "parentId": folder, "workspaceId": workspace_id });
                call(&client, cx, "folder.move", payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report(t("mocks.error.move_folder"), message, cx);
                    }
                });
            }
            TreeKey::Item(id) => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.key == TabKey::Saved(id.clone())) {
                    return tab.editor.clone().update(cx, |editor, cx| editor.set_folder(folder, cx));
                }
                let mut patch = Map::new();
                patch.insert("folderId".into(), json!(folder));
                patch_entity(&client, cx, &workspace_id, "mocks", &id, patch, |this, outcome, cx| {
                    if let Err(message) = outcome {
                        this.report(t("mocks.error.move_mock"), message, cx);
                    }
                });
            }
        }
    }

    fn confirm_delete(&mut self, key: TreeKey, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_of(&key, cx);
        let (title, consequence) = match &key {
            TreeKey::Folder(_) => (t_with("mocks.delete.title_folder", &[("name", &name)]), t(FOLDER_DELETE_CONSEQUENCE)),
            TreeKey::Item(id) => (
                t_with("mocks.delete.title_mock", &[("name", &name)]),
                delete_consequence(self.model.read(cx).git().entity_state(id)),
            ),
        };
        confirm_danger(
            cx.weak_entity(),
            window,
            cx,
            title,
            consequence,
            t("mocks.delete.button"),
            move |this, _, cx| {
                let client = this.model.read(cx).client().clone();
                let (action, payload) = match &key {
                    TreeKey::Folder(id) => ("folder.delete", json!({ "kind": "mock", "id": id })),
                    TreeKey::Item(id) => ("entity.delete", json!({ "kind": "mocks", "id": id })),
                };
                call(&client, cx, action, payload, |this, outcome, cx| {
                    if let Some(message) = failure(&outcome) {
                        this.report(t("mocks.error.delete"), message, cx);
                    }
                });
            },
        );
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let (row_view, menu_view, new_view) = (this.clone(), this.clone(), this.clone());
        let model = self.model.clone();
        let menu_model = self.model.clone();
        let config = self.model.read(cx).config();
        let has_mocks = !config.mocks.is_empty() || !config.mock_folders.is_empty();
        let filtering = !self.filter.read(cx).value().trim().is_empty();
        // A folder that is in a folder can be dragged out to the top level.
        let top_level =
            self.dragging.as_ref().filter(|dragged| accepts(self.model.read(cx), dragged, None)).map(|_| {
                let (drop_model, drop_view) = (self.model.clone(), this.clone());
                tree_drag::top_level_target(
                    "mock-tree-top-level",
                    move |dragged: &TreeKey, cx| accepts(drop_model.read(cx), dragged, None),
                    move |dragged: &TreeKey, _, cx| {
                        let key = dragged.clone();
                        drop_view.update(cx, |this, cx| this.move_node(key, None, cx)).ok();
                    },
                    cx,
                )
            });

        let body = if !has_mocks {
            page::empty(t("mocks.tree.empty_title"), t("mocks.tree.empty_body"))
        } else if filtering && self.tree.read(cx).entry(0).is_none() {
            page::empty(t("mocks.tree.no_matches_title"), t("mocks.tree.no_matches_body"))
        } else {
            tree(&self.tree, move |ix, entry, selected, _, cx| render_row(ix, entry, selected, &model, &row_view, cx))
                .context_menu(move |_, entry, menu, _, cx| context_menu(entry, menu, &menu_view, &menu_model, cx))
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
                            .tooltip(t("mocks.tree.new_tooltip"))
                            .dropdown_menu(move |menu, _, _| new_menu(menu, &new_view, None)),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .children(top_level)
    }

    fn render_tab_strip(&self, tree_open: bool, window: &Window, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let this = cx.weak_entity();
        let strip = TabStrip {
            id: "mock-tabs",
            count: self.tabs.len(),
            active: self.active,
            scroll: self.tab_scroll.clone(),
            describe: Self::describe_tab,
            on_action: Self::tab_action,
        };
        let prefix = h_flex().flex_none().px_1().child(
            Button::new("toggle-mock-tree")
                .icon(if tree_open { IconName::FolderOpen } else { IconName::Folder })
                .ghost()
                .small()
                .selected(tree_open)
                .tooltip(if tree_open { t("mocks.tabs.hide_tree") } else { t("mocks.tabs.show_tree") })
                .on_click(cx.listener(|_, _, _, cx| {
                    Preferences::update(cx, |p| p.folder_tree_open = !p.folder_tree_open);
                    cx.notify();
                })),
        );
        let suffix = h_flex().flex_none().px_1().child(
            Button::new("new-mock-tab")
                .icon(IconName::Plus)
                .ghost()
                .small()
                .tooltip(t("mocks.new_mock"))
                .dropdown_menu(move |menu, _, _| new_tab_menu(menu, &this)),
        );
        tab_strip(self, strip, prefix, suffix, window, cx)
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

/// What a new tab can start as: an empty mock, or one made from a cURL command.
fn new_tab_menu(menu: PopupMenu, view: &WeakEntity<MocksScreen>) -> PopupMenu {
    menu.item(
        PopupMenuItem::new(t("mocks.menu.mock"))
            .on_click(act(view, |this, window, cx| this.new_mock(None, window, cx))),
    )
        .item(
            PopupMenuItem::new(t(CURL_TAB_TITLE))
                .on_click(act(view, |this, window, cx| this.import_curl(None, window, cx))),
        )
}

fn new_menu(menu: PopupMenu, view: &WeakEntity<MocksScreen>, folder: Option<String>) -> PopupMenu {
    let (parent, curl_folder) = (folder.clone(), folder.clone());
    menu.item(
        PopupMenuItem::new(t("mocks.menu.mock"))
            .on_click(act(view, move |this, window, cx| this.new_mock(folder.clone(), window, cx))),
    )
    .item(
        PopupMenuItem::new(t(CURL_TAB_TITLE))
            .on_click(act(view, move |this, window, cx| this.import_curl(curl_folder.clone(), window, cx))),
    )
    .item(
        PopupMenuItem::new(t("mocks.menu.folder"))
            .on_click(act(view, move |this, window, cx| this.prompt_new_folder(parent.clone(), window, cx))),
    )
}

/// How the model names a deleted mock or folder.
fn deleted_target(key: &TreeKey) -> Deleted {
    match key {
        TreeKey::Folder(id) => Deleted::Folder { kind: "mock", id: id.clone() },
        TreeKey::Item(id) => Deleted::Entity { kind: "mocks", id: id.clone() },
    }
}

/// Whether the mock or folder is deleted here, with the deletion not confirmed or reverted yet.
fn is_deleted(model: &AppModel, key: &TreeKey) -> bool {
    let config = model.config();
    match key {
        TreeKey::Folder(id) => config.mock_folders.iter().any(|f| f.id == *id && f.deleted),
        TreeKey::Item(id) => config.mock(id).is_some_and(|m| m.deleted),
    }
}

/// The version of Bifurc the mock needs, when a newer one than this last saved it. A folder is never locked:
/// it is a directory, and moving or renaming it changes nothing in it.
fn locked_to(model: &AppModel, key: &TreeKey) -> Option<String> {
    match key {
        TreeKey::Folder(_) => None,
        TreeKey::Item(id) => model.config().mock(id).and_then(|m| m.locked.clone()),
    }
}

/// The folder a drop on this row goes to: the folder itself, or the folder the mock is in.
fn drop_folder(model: &AppModel, key: &TreeKey) -> Option<String> {
    match key {
        TreeKey::Folder(id) => Some(id.clone()),
        TreeKey::Item(id) => model.config().mock(id).and_then(|m| m.folder_id.clone()),
    }
}

/// Whether the dragged mock or folder can be put in `folder`, or at the top level with `None`.
fn accepts(model: &AppModel, dragged: &TreeKey, folder: Option<&str>) -> bool {
    let config = model.config();
    let moved = match dragged {
        TreeKey::Folder(id) => Moved::Folder(id),
        TreeKey::Item(id) => match config.mock(id) {
            Some(mock) => Moved::Item { folder: mock.folder_id.as_deref() },
            None => return false,
        },
    };
    can_move(&config.mock_folder_rows(), moved, folder)
}

fn context_menu(
    entry: &TreeEntry,
    menu: PopupMenu,
    view: &WeakEntity<MocksScreen>,
    model: &Entity<AppModel>,
    cx: &App,
) -> PopupMenu {
    let Some(key) = TreeKey::parse(entry.item().id.as_str()) else { return menu };
    // What is deleted can only be brought back or deleted for good.
    if is_deleted(model.read(cx), &key) {
        let (revert_model, confirm_model) = (model.clone(), model.clone());
        let (revert, confirm) = (deleted_target(&key), deleted_target(&key));
        return menu
            .item(PopupMenuItem::new(t("mocks.menu.revert_delete")).on_click(move |_, _, cx| {
                revert_model.update(cx, |model, cx| model.revert_delete(revert.clone(), cx));
            }))
            .item(PopupMenuItem::new(t("mocks.menu.confirm_delete")).on_click(move |_, _, cx| {
                confirm_model.update(cx, |model, cx| model.confirm_delete(confirm.clone(), cx));
            }));
    }
    // What a newer Bifurc saved can only be asked about: opening it says which version it needs.
    if let (TreeKey::Item(id), Some(_)) = (&key, locked_to(model.read(cx), &key)) {
        let id = id.clone();
        return menu.item(
            PopupMenuItem::new(t("mocks.menu.update_required"))
                .on_click(act(view, move |this, window, cx| this.open_mock(&id, window, cx))),
        );
    }
    let (rename, delete) = (key.clone(), key.clone());
    let menu = match &key {
        TreeKey::Item(id) => {
            let (open, copy) = (id.clone(), id.clone());
            menu.item(
                PopupMenuItem::new(t("mocks.menu.open"))
                    .on_click(act(view, move |this, window, cx| this.open_mock(&open, window, cx))),
            )
            .item(
                PopupMenuItem::new(t("mocks.menu.duplicate"))
                    .on_click(act(view, move |this, window, cx| this.duplicate(copy.clone(), window, cx))),
            )
        }
        TreeKey::Folder(id) => {
            let (mock_parent, folder_parent) = (id.clone(), id.clone());
            menu.item(
                PopupMenuItem::new(t("mocks.menu.new_mock_here"))
                    .on_click(act(view, move |this, window, cx| this.new_mock(Some(mock_parent.clone()), window, cx))),
            )
            .item(PopupMenuItem::new(t("mocks.menu.new_folder_here")).on_click(act(view, move |this, window, cx| {
                this.prompt_new_folder(Some(folder_parent.clone()), window, cx)
            })))
        }
    };
    menu.separator()
        .item(
            PopupMenuItem::new(t("mocks.menu.rename"))
                .on_click(act(view, move |this, window, cx| this.prompt_rename(rename.clone(), window, cx))),
        )
        .item(
            PopupMenuItem::new(t("mocks.menu.delete"))
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
    let deleted = key.as_ref().is_some_and(|key| is_deleted(model.read(cx), key));
    let locked = key.as_ref().is_some_and(|key| locked_to(model.read(cx), key).is_some());
    let mut pending = false;
    let mut toggle: Option<gpui_kit::AnyElement> = None;
    let lead: gpui_kit::AnyElement = match &key {
        Some(TreeKey::Folder(_)) => {
            Icon::new(if entry.is_expanded() { IconName::FolderOpen } else { IconName::Folder })
                .small()
                .text_color(theme.muted_foreground)
                .into_any_element()
        }
        // Saved by a newer Bifurc: the sign for an update stands where its method would, and it has no switch.
        // It answers nothing until this one is updated.
        Some(TreeKey::Item(_)) if locked => div()
            .w_12()
            .flex_none()
            .child(Icon::new(IconName::ArrowUp).small().text_color(theme.muted_foreground))
            .into_any_element(),
        Some(TreeKey::Item(id)) => {
            let app_model = model.read(cx);
            pending = app_model.git().entity_state(id).is_some_and(|state| state != FileState::Clean);
            let mock = app_model.config().mock(id);
            let method = mock.map(|m| if m.method == "*" { "ANY".to_owned() } else { m.method.clone() });
            let enabled = mock.is_some_and(|m| m.enabled);
            let (toggle_model, toggle_id) = (model.clone(), id.clone());
            // A deleted mock has no switch: it answers nothing until it is reverted.
            toggle = (!mock.is_some_and(|m| m.deleted)).then(|| {
                Switch::new(gpui_kit::ElementId::Name(format!("mock-enabled-{id}").into()))
                    .checked(enabled)
                    .small()
                    .on_change(move |checked: &bool, _, cx| {
                        // Turning a mock on or off is not opening it.
                        cx.stop_propagation();
                        let checked = *checked;
                        toggle_model.update(cx, |m, cx| m.set_enabled("mocks", &toggle_id, checked, cx));
                    })
                    .into_any_element()
            });
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

    let open = match &key {
        Some(TreeKey::Item(id)) => Some(id.clone()),
        _ => None,
    };
    let folder = key.as_ref().and_then(|key| drop_folder(model.read(cx), key));
    let (drag_view, drop_view, drop_model) = (view.clone(), view.clone(), model.clone());
    let view = view.clone();
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
                .children(pending.then(|| div().flex_none().text_xs().text_color(theme.warning).child("●")))
                .children(toggle.map(|toggle| div().flex_none().pr_2().child(toggle))),
        )
        .on_click(move |_, window, cx| {
            if let Some(id) = open.clone() {
                view.update(cx, |this, cx| this.open_mock(&id, window, cx)).ok();
            }
        });
    // A row dropped on a mock goes to the folder that mock is in.
    let row = match folder {
        Some(folder) => {
            let target = folder.clone();
            tree_drag::drop_target(
                row,
                move |dragged: &TreeKey, cx| accepts(drop_model.read(cx), dragged, Some(&target)),
                move |dragged: &TreeKey, _, cx| {
                    let (key, folder) = (dragged.clone(), Some(folder.clone()));
                    drop_view.update(cx, |this, cx| this.move_node(key, folder, cx)).ok();
                },
            )
        }
        None => row,
    };
    // What is deleted stays where it is until it is reverted, and what is locked until Bifurc is updated.
    match key.filter(|_| !deleted && !locked) {
        Some(key) => tree_drag::draggable(row, key, label, move |key: &TreeKey, cx| {
            let key = key.clone();
            drag_view.update(cx, |this, cx| this.drag_started(key, cx)).ok();
        }),
        None => row,
    }
}

impl Render for MocksScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree_open = Preferences::global(cx).folder_tree_open;
        if !cx.has_active_drag() {
            self.dragging = None;
        }
        let curl = self.active_curl().cloned();
        let body = if self.tabs.is_empty() {
            page::empty_with_action(
                t("mocks.no_mock_open"),
                t("mocks.welcome.body"),
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("welcome-new-mock")
                            .label(t("mocks.new_mock"))
                            .on_click(cx.listener(|this, _, window, cx| this.new_mock(None, window, cx))),
                    )
                    .child(
                        Button::new("welcome-import-curl")
                            .label(t(CURL_TAB_TITLE))
                            .on_click(cx.listener(|this, _, window, cx| this.import_curl(None, window, cx))),
                    ),
            )
        } else if let Some(import) = &curl {
            import.clone().into_any_element()
        } else {
            self.dock.clone().into_any_element()
        };
        // A tab waiting for a command has no mock to save, move or delete yet.
        let bar = self.active_editor().filter(|_| curl.is_none()).map(|editor| {
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
            .child(self.render_tab_strip(tree_open, window, cx))
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
