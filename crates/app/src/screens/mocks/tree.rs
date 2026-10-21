//! The folder tree of the Mocks screen: every mock rule, in its folders.
//!
//! The tree itself is the shared [`FolderTree`]; this is what the Mocks screen gives it: the mocks and
//! folders of the model, the method badge, the switch that turns a mock on or off, and the items that start
//! a mock.

use gpui_kit::component::{
    Sizable as _,
    menu::{PopupMenu, PopupMenuItem},
    switch::Switch,
};
use gpui_kit::{AnyElement, App, Entity, IntoElement as _, WeakEntity};

use crate::{
    app_model::{AppModel, Deleted},
    components::{
        CURL_TAB_TITLE,
        folder_tree::{FolderTree, FolderTreeEvent, FolderTreeSource, Lead, TreeText, emitter},
    },
    i18n::t,
    model::tree::{FolderRow, ItemRow, TreeKey, TreeNode, build_tree},
};

/// What only a mock tree can ask for.
#[derive(Debug, Clone)]
pub enum MockExtra {
    NewMock { folder: Option<String> },
    ImportCurl { folder: Option<String> },
    CheckContract { folder: String },
}

pub type MockTreeEvent = FolderTreeEvent<TreeKey, MockExtra>;
pub type MockTree = FolderTree<MockSource>;

pub struct MockSource;

/// How the model names a deleted mock or folder.
pub fn deleted_target(key: &TreeKey) -> Deleted {
    match key {
        TreeKey::Folder(id) => Deleted::Folder { kind: "mock", id: id.clone() },
        TreeKey::Item(id) => Deleted::Entity { kind: "mocks", id: id.clone() },
    }
}

/// Whether the mock or folder is deleted here, with the deletion not confirmed or reverted yet.
pub fn is_deleted(model: &AppModel, key: &TreeKey) -> bool {
    let config = model.config();
    match key {
        TreeKey::Folder(id) => config.mock_folders.iter().any(|f| f.id == *id && f.deleted),
        TreeKey::Item(id) => config.mock(id).is_some_and(|m| m.deleted),
    }
}

/// The version of Bifurc the mock needs, when a newer one than this last saved it.
pub fn locked_to(model: &AppModel, key: &TreeKey) -> Option<String> {
    match key {
        TreeKey::Folder(_) => None,
        TreeKey::Item(id) => model.config().mock(id).and_then(|m| m.locked.clone()),
    }
}

impl FolderTreeSource for MockSource {
    type Key = TreeKey;
    type Extra = MockExtra;
    const TEXT: TreeText = TreeText {
        filter_placeholder: "mocks.filter.placeholder",
        new_tooltip: "mocks.tree.new_tooltip",
        empty_title: "mocks.tree.empty_title",
        empty_description: "mocks.tree.empty_body",
        no_matches_title: "mocks.tree.no_matches_title",
        no_matches_description: "mocks.tree.no_matches_body",
        open: "mocks.menu.open",
        duplicate: "mocks.menu.duplicate",
        rename: "mocks.menu.rename",
        delete: "mocks.menu.delete",
        revert_delete: "mocks.menu.revert_delete",
        confirm_delete: "mocks.menu.confirm_delete",
        update_required: "mocks.menu.update_required",
        new_folder: "mocks.menu.folder",
        new_folder_here: "mocks.menu.new_folder_here",
    };

    fn nodes(model: &AppModel, filter: &str) -> Vec<TreeNode> {
        let config = model.config();
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
        build_tree(&folders, &items, filter)
    }

    fn is_empty(model: &AppModel) -> bool {
        let config = model.config();
        config.mocks.is_empty() && config.mock_folders.is_empty()
    }

    fn folder_rows(model: &AppModel) -> Vec<FolderRow<'_>> {
        model.config().mock_folder_rows()
    }

    fn item_folder(model: &AppModel, key: &TreeKey) -> Option<Option<String>> {
        let TreeKey::Item(id) = key else { return None };
        model.config().mock(id).map(|m| m.folder_id.clone())
    }

    fn is_deleted(model: &AppModel, key: &TreeKey) -> bool {
        is_deleted(model, key)
    }

    fn locked_to(model: &AppModel, key: &TreeKey) -> Option<String> {
        locked_to(model, key)
    }

    fn lead(model: &AppModel, key: &TreeKey) -> Option<Lead> {
        let TreeKey::Item(id) = key else { return None };
        let method = model.config().mock(id)?.method.as_str();
        Some(Lead::Badge(if method == "*" { "ANY".to_owned() } else { method.to_owned() }))
    }

    /// The switch that turns the mock on or off. It is not on a deleted mock or a locked one, which answer
    /// nothing until they are brought back or Bifurc is updated.
    fn trailing(model: &Entity<AppModel>, key: &TreeKey, cx: &App) -> Option<AnyElement> {
        let TreeKey::Item(id) = key else { return None };
        let mock = model.read(cx).config().mock(id)?;
        let (toggle_model, toggle_id) = (model.clone(), id.clone());
        Some(
            Switch::new(gpui_kit::ElementId::Name(format!("mock-enabled-{id}").into()))
                .checked(mock.enabled)
                .small()
                .on_change(move |checked: &bool, _, cx| {
                    // Turning a mock on or off is not opening it.
                    cx.stop_propagation();
                    let checked = *checked;
                    toggle_model.update(cx, |m, cx| m.set_enabled("mocks", &toggle_id, checked, cx));
                })
                .into_any_element(),
        )
    }

    fn folder_menu(menu: PopupMenu, folder: &str, tree: &WeakEntity<MockTree>) -> PopupMenu {
        let event = FolderTreeEvent::Extra(MockExtra::NewMock { folder: Some(folder.to_owned()) });
        let check = FolderTreeEvent::Extra(MockExtra::CheckContract { folder: folder.to_owned() });
        menu.item(PopupMenuItem::new(t("mocks.menu.new_mock_here")).on_click(emitter(tree, event)))
            .item(PopupMenuItem::new(t("contract.menu.check_folder")).on_click(emitter(tree, check)))
    }

    fn new_menu(menu: PopupMenu, folder: Option<String>, tree: &WeakEntity<MockTree>) -> PopupMenu {
        let curl = FolderTreeEvent::Extra(MockExtra::ImportCurl { folder: folder.clone() });
        menu.item(
            PopupMenuItem::new(t("mocks.menu.mock"))
                .on_click(emitter(tree, FolderTreeEvent::Extra(MockExtra::NewMock { folder }))),
        )
        .item(PopupMenuItem::new(t(CURL_TAB_TITLE)).on_click(emitter(tree, curl)))
    }
}
