//! The folder tree of the Flows screen: every saved flow, in its folders.
//!
//! The tree itself is the shared [`FolderTree`]; this is what the Flows screen gives it: the flows and
//! folders of the model, and the items that start a flow.

use gpui_kit::{
    WeakEntity,
    component::{
        IconName,
        menu::{PopupMenu, PopupMenuItem},
    },
};

use crate::{
    app_model::{AppModel, Deleted},
    components::folder_tree::{FolderTree, FolderTreeEvent, FolderTreeSource, Lead, TreeText, emitter},
    i18n::t,
    model::tree::{FolderRow, ItemRow, TreeKey, TreeNode, build_tree},
};

/// What only a flow tree can ask for.
#[derive(Debug, Clone)]
pub enum FlowExtra {
    NewFlow { folder: Option<String> },
}

pub type FlowTreeEvent = FolderTreeEvent<TreeKey, FlowExtra>;
pub type FlowTree = FolderTree<FlowSource>;

pub struct FlowSource;

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

/// The version of Bifurc the flow needs, when a newer one than this last saved it.
pub fn locked_to(model: &AppModel, key: &TreeKey) -> Option<String> {
    match key {
        TreeKey::Folder(_) => None,
        TreeKey::Item(id) => model.config().flows.iter().find(|f| f.id == *id).and_then(|f| f.locked.clone()),
    }
}

impl FolderTreeSource for FlowSource {
    type Key = TreeKey;
    type Extra = FlowExtra;
    const TEXT: TreeText = TreeText {
        filter_placeholder: "flows.tree.filter",
        new_tooltip: "flows.tree.new_tooltip",
        empty_title: "flows.tree.empty.title",
        empty_description: "flows.tree.empty.description",
        no_matches_title: "flows.tree.no_matches.title",
        no_matches_description: "flows.tree.no_matches.description",
        open: "flows.common.open",
        duplicate: "flows.common.duplicate",
        rename: "flows.menu.rename",
        delete: "flows.menu.delete",
        revert_delete: "flows.menu.revert_delete",
        confirm_delete: "flows.menu.confirm_delete",
        update_required: "flows.menu.update_required",
        new_folder: "flows.menu.new_folder",
        new_folder_here: "flows.menu.new_folder_here",
    };

    fn nodes(model: &AppModel, filter: &str) -> Vec<TreeNode> {
        let config = model.config();
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
        build_tree(&folders, &items, filter)
    }

    fn is_empty(model: &AppModel) -> bool {
        let config = model.config();
        config.flows.is_empty() && config.flow_folders.is_empty()
    }

    fn folder_rows(model: &AppModel) -> Vec<FolderRow<'_>> {
        model.config().flow_folder_rows()
    }

    fn item_folder(model: &AppModel, key: &TreeKey) -> Option<Option<String>> {
        let TreeKey::Item(id) = key else { return None };
        model.config().flows.iter().find(|f| f.id == *id).map(|f| f.folder_id.clone())
    }

    fn is_deleted(model: &AppModel, key: &TreeKey) -> bool {
        is_deleted(model, key)
    }

    fn locked_to(model: &AppModel, key: &TreeKey) -> Option<String> {
        locked_to(model, key)
    }

    fn lead(_: &AppModel, _: &TreeKey) -> Option<Lead> {
        Some(Lead::Icon(IconName::LayoutDashboard))
    }

    fn folder_menu(menu: PopupMenu, folder: &str, tree: &WeakEntity<FlowTree>) -> PopupMenu {
        let event = FolderTreeEvent::Extra(FlowExtra::NewFlow { folder: Some(folder.to_owned()) });
        menu.item(PopupMenuItem::new(t("flows.menu.new_flow_here")).on_click(emitter(tree, event)))
    }

    fn new_menu(menu: PopupMenu, folder: Option<String>, tree: &WeakEntity<FlowTree>) -> PopupMenu {
        menu.item(
            PopupMenuItem::new(t("flows.menu.new_flow"))
                .on_click(emitter(tree, FolderTreeEvent::Extra(FlowExtra::NewFlow { folder }))),
        )
    }
}
