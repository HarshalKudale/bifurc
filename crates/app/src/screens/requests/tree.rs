//! The folder tree of the Requests screen: every saved request of every type, in its folders.
//!
//! The tree itself is the shared [`FolderTree`]; this is what the Requests screen gives it: the explorer's
//! nodes (five request types merged), the method badge, and the items that start a request of a type.

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::WeakEntity;

use crate::{
    app_model::AppModel,
    components::{
        CURL_TAB_TITLE,
        folder_tree::{FolderTree, FolderTreeEvent, FolderTreeSource, Lead, TreeText, emitter},
    },
    i18n::{t, t_with},
    model::{
        ExplorerNode, NodeRef, RequestKind, build_explorer,
        tree::{FolderRow, TreeNode},
    },
};

/// What only a request tree can ask for.
#[derive(Debug, Clone)]
pub enum RequestExtra {
    NewRequest { kind: RequestKind, folder: Option<String> },
    /// Start a request from a cURL command; its type comes from the command.
    ImportCurl,
    /// Open the folder with this id as a runner tab.
    RunFolder(String),
}

pub type RequestTreeEvent = FolderTreeEvent<NodeRef, RequestExtra>;
pub type RequestTree = FolderTree<RequestSource>;

pub struct RequestSource;

/// Whether the request or folder is deleted here, with the deletion not confirmed or reverted yet.
pub fn is_deleted(model: &AppModel, node: &NodeRef) -> bool {
    let config = model.config();
    match node {
        NodeRef::Folder { id } => config.folder(id).is_some_and(|f| f.deleted),
        NodeRef::Request { kind, id } => config.request(*kind, id).is_some_and(|r| r.deleted),
    }
}

/// The version of Bifurc the request needs, when a newer one than this last saved it.
pub fn locked_to(model: &AppModel, node: &NodeRef) -> Option<String> {
    match node {
        NodeRef::Folder { .. } => None,
        NodeRef::Request { kind, id } => model.config().request(*kind, id).and_then(|r| r.locked.clone()),
    }
}

fn to_tree_node(node: &ExplorerNode) -> TreeNode {
    TreeNode {
        key: node.node.encode(),
        label: node.label.clone(),
        children: node.children.iter().map(to_tree_node).collect(),
    }
}

impl FolderTreeSource for RequestSource {
    type Key = NodeRef;
    type Extra = RequestExtra;
    const TEXT: TreeText = TreeText {
        filter_placeholder: "requests.tree.filter_placeholder",
        new_tooltip: "requests.tree.new_tooltip",
        empty_title: "requests.tree.empty.title",
        empty_description: "requests.tree.empty.description",
        no_matches_title: "requests.tree.no_matches.title",
        no_matches_description: "requests.tree.no_matches.description",
        open: "requests.common.open",
        duplicate: "requests.common.duplicate",
        rename: "requests.tree.menu.rename",
        delete: "requests.tree.menu.delete",
        revert_delete: "requests.tree.menu.revert_delete",
        confirm_delete: "requests.tree.menu.confirm_delete",
        update_required: "requests.tree.menu.update_required",
        new_folder: "requests.tree.new_menu.folder",
        new_folder_here: "requests.tree.menu.new_folder_here",
    };

    fn nodes(model: &AppModel, filter: &str) -> Vec<TreeNode> {
        build_explorer(model.config(), filter).iter().map(to_tree_node).collect()
    }

    fn is_empty(model: &AppModel) -> bool {
        let config = model.config();
        config.requests.is_empty() && config.folders.is_empty()
    }

    fn folder_rows(model: &AppModel) -> Vec<FolderRow<'_>> {
        model.config().request_folder_rows()
    }

    fn item_folder(model: &AppModel, key: &NodeRef) -> Option<Option<String>> {
        match key {
            NodeRef::Folder { .. } => None,
            NodeRef::Request { kind, id } => model.config().request(*kind, id).map(|r| r.folder_id.clone()),
        }
    }

    fn is_deleted(model: &AppModel, key: &NodeRef) -> bool {
        is_deleted(model, key)
    }

    fn locked_to(model: &AppModel, key: &NodeRef) -> Option<String> {
        locked_to(model, key)
    }

    fn lead(model: &AppModel, key: &NodeRef) -> Option<Lead> {
        let NodeRef::Request { kind, id } = key else { return None };
        let badge = model.config().request(*kind, id).and_then(|r| r.method.clone());
        Some(Lead::Badge(badge.unwrap_or_else(|| kind.badge().to_owned())))
    }

    fn folder_menu(menu: PopupMenu, folder: &str, tree: &WeakEntity<RequestTree>) -> PopupMenu {
        let extra = |extra| emitter(tree, FolderTreeEvent::Extra(extra));
        menu
            // The engine runs the REST requests of a folder; the other types in it have no runner.
            .item(PopupMenuItem::new(t("requests.tree.menu.open_as_runner")).on_click(extra(RequestExtra::RunFolder(folder.to_owned()))))
            .separator()
            // A new request starts as REST; its type is changed in the breadcrumb until it is saved.
            .item(PopupMenuItem::new(t("requests.tree.menu.new_request_here")).on_click(extra(RequestExtra::NewRequest {
                kind: RequestKind::Rest,
                folder: Some(folder.to_owned()),
            })))
    }

    /// One request of each type, one from a cURL command, then a folder. A folder takes requests of every
    /// type.
    fn new_menu(menu: PopupMenu, folder: Option<String>, tree: &WeakEntity<RequestTree>) -> PopupMenu {
        let menu = RequestKind::ALL.into_iter().fold(menu, |menu, kind| {
            menu.item(
                PopupMenuItem::new(t_with("requests.tree.new_menu.kind_request", &[("kind", &kind.label())]))
                    .on_click(emitter(tree, FolderTreeEvent::Extra(RequestExtra::NewRequest { kind, folder: folder.clone() }))),
            )
        });
        menu.separator()
            .item(PopupMenuItem::new(t(CURL_TAB_TITLE)).on_click(emitter(tree, FolderTreeEvent::Extra(RequestExtra::ImportCurl))))
            .separator()
    }
}
