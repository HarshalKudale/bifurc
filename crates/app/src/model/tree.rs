//! A folder tree over one kind of saved entity: folders first, then items, each sorted by name.
//!
//! The requests explorer has its own builder because it merges five request types; this is the same shape
//! for a screen with a single type, such as mocks.

use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct FolderRow<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub parent_id: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemRow<'a> {
    pub id: &'a str,
    pub label: String,
    pub folder_id: Option<&'a str>,
    /// Extra text a filter may match besides the label.
    pub detail: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    /// `folder:<id>` or `item:<id>`; stable across refreshes.
    pub key: String,
    pub label: String,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    pub fn is_folder(&self) -> bool {
        self.key.starts_with("folder:")
    }
}

/// What a tree key refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeKey {
    Folder(String),
    Item(String),
}

impl TreeKey {
    pub fn parse(key: &str) -> Option<Self> {
        if let Some(id) = key.strip_prefix("folder:") {
            Some(Self::Folder(id.to_owned()))
        } else {
            key.strip_prefix("item:").map(|id| Self::Item(id.to_owned()))
        }
    }

    pub fn encode(&self) -> String {
        match self {
            Self::Folder(id) => format!("folder:{id}"),
            Self::Item(id) => format!("item:{id}"),
        }
    }
}

/// Builds the tree. A non-empty `filter` keeps items whose label or detail contains it (case-insensitive)
/// and the folders above them; a folder whose own name matches is kept with everything inside it. Items and
/// folders whose parent no longer exists surface at the top rather than being lost.
pub fn build_tree(folders: &[FolderRow], items: &[ItemRow], filter: &str) -> Vec<TreeNode> {
    let needle = filter.trim().to_lowercase();
    let known: HashSet<&str> = folders.iter().map(|f| f.id).collect();
    let mut seen = HashSet::new();
    level(folders, items, &known, None, &needle, false, &mut seen)
}

fn level<'a>(
    folders: &[FolderRow<'a>],
    items: &[ItemRow<'a>],
    known: &HashSet<&str>,
    parent: Option<&str>,
    needle: &str,
    keep_all: bool,
    seen: &mut HashSet<&'a str>,
) -> Vec<TreeNode> {
    let filtering = !needle.is_empty();
    let is_here = |candidate: Option<&str>| match (candidate, parent) {
        (None, None) => true,
        (Some(c), Some(p)) => c == p,
        (Some(c), None) => !known.contains(c),
        (None, Some(_)) => false,
    };

    let mut nodes: Vec<TreeNode> = Vec::new();
    for folder in folders.iter().filter(|f| is_here(f.parent_id)) {
        if !seen.insert(folder.id) {
            continue; // a cycle in the stored parents; the first visit wins
        }
        let own_match = filtering && folder.name.to_lowercase().contains(needle);
        let children = level(folders, items, known, Some(folder.id), needle, keep_all || own_match, seen);
        if !filtering || keep_all || own_match || !children.is_empty() {
            nodes.push(TreeNode { key: format!("folder:{}", folder.id), label: folder.name.to_owned(), children });
        }
    }
    nodes.sort_by_key(|n| n.label.to_lowercase());

    let mut leaves: Vec<TreeNode> = items
        .iter()
        .filter(|item| is_here(item.folder_id))
        .filter(|item| {
            !filtering
                || keep_all
                || item.label.to_lowercase().contains(needle)
                || item.detail.to_lowercase().contains(needle)
        })
        .map(|item| TreeNode { key: format!("item:{}", item.id), label: item.label.clone(), children: Vec::new() })
        .collect();
    leaves.sort_by_key(|n| n.label.to_lowercase());
    nodes.extend(leaves);
    nodes
}

/// The names of the folders above an entity, outermost first, ending in the folder with `id`. A folder that
/// no longer exists gives an empty path, which is where the tree shows its contents too: at the top.
pub fn folder_path(folders: &[FolderRow], id: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(id);
    while let Some(current) = next {
        // A cycle in the stored parents ends the walk rather than looping.
        if !seen.insert(current) {
            break;
        }
        let Some(folder) = folders.iter().find(|f| f.id == current) else { break };
        names.push(folder.name.to_owned());
        next = folder.parent_id;
    }
    names.reverse();
    names
}

/// Every folder as a place something can be moved to: its id and its full path (`Users / Admin`), in the
/// order of the paths.
pub fn folder_choices(folders: &[FolderRow]) -> Vec<(String, String)> {
    let mut choices: Vec<(String, String)> =
        folders.iter().map(|f| (f.id.to_owned(), folder_path(folders, f.id).join(" / "))).collect();
    choices.sort_by_key(|(_, path)| path.to_lowercase());
    choices
}

/// What a drag in a tree is moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved<'a> {
    Folder(&'a str),
    /// An entity, and the folder it is in now.
    Item {
        folder: Option<&'a str>,
    },
}

/// Whether `moved` can be put in the folder `target`, or at the top level with `None`. `folders` are the
/// folders something can be kept in. An entity is always in a folder, so only a folder goes to the top; a
/// folder does not go into itself or a folder under it; and a move to where it already is, is not one.
pub fn can_move(folders: &[FolderRow], moved: Moved, target: Option<&str>) -> bool {
    let parent_of = |id: &str| folders.iter().find(|f| f.id == id).map(|f| f.parent_id);
    if target.is_some_and(|target| parent_of(target).is_none()) {
        return false;
    }
    match (moved, target) {
        (Moved::Item { folder }, Some(target)) => folder != Some(target),
        (Moved::Item { .. }, None) => false,
        (Moved::Folder(id), None) => parent_of(id).is_some_and(|parent| parent.is_some()),
        (Moved::Folder(id), Some(target)) => {
            let Some(parent) = parent_of(id) else { return false };
            if parent == Some(target) {
                return false;
            }
            // Walk up from the target: meeting the moved folder means the target is inside it.
            let mut seen = HashSet::new();
            let mut next = Some(target);
            while let Some(current) = next {
                if current == id {
                    return false;
                }
                if !seen.insert(current) {
                    break;
                }
                next = parent_of(current).flatten();
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entity_moves_to_any_other_folder_but_never_to_the_top() {
        let item = Moved::Item { folder: Some("f1") };
        assert!(can_move(&folders(), item, Some("f2")));
        assert!(!can_move(&folders(), item, Some("f1")));
        assert!(!can_move(&folders(), item, None));
        assert!(!can_move(&folders(), item, Some("gone")));
    }

    #[test]
    fn a_folder_moves_anywhere_but_into_itself_or_where_it_is() {
        let mut all = folders();
        all.push(FolderRow { id: "f3", name: "Other", parent_id: None });
        assert!(can_move(&all, Moved::Folder("f2"), Some("f3")));
        assert!(can_move(&all, Moved::Folder("f2"), None));
        assert!(can_move(&all, Moved::Folder("f3"), Some("f2")));
        // Already there.
        assert!(!can_move(&all, Moved::Folder("f2"), Some("f1")));
        assert!(!can_move(&all, Moved::Folder("f1"), None));
        // Into itself, or a folder under it.
        assert!(!can_move(&all, Moved::Folder("f1"), Some("f1")));
        assert!(!can_move(&all, Moved::Folder("f1"), Some("f2")));
        assert!(!can_move(&all, Moved::Folder("gone"), Some("f1")));
    }

    #[test]
    fn a_folder_path_runs_from_the_top_down_to_the_folder() {
        assert_eq!(folder_path(&folders(), "f2"), ["Users", "admin"]);
        assert_eq!(folder_path(&folders(), "f1"), ["Users"]);
        assert!(folder_path(&folders(), "gone").is_empty());
    }

    #[test]
    fn folder_choices_are_full_paths_in_order() {
        let choices = folder_choices(&folders());
        assert_eq!(choices, [("f1".to_owned(), "Users".to_owned()), ("f2".to_owned(), "Users / admin".to_owned())]);
    }

    #[test]
    fn a_parent_cycle_cannot_hang_a_folder_path() {
        let cyclic = vec![
            FolderRow { id: "a", name: "A", parent_id: Some("b") },
            FolderRow { id: "b", name: "B", parent_id: Some("a") },
        ];
        assert_eq!(folder_path(&cyclic, "a"), ["B", "A"]);
    }

    fn folders() -> Vec<FolderRow<'static>> {
        vec![
            FolderRow { id: "f1", name: "Users", parent_id: None },
            FolderRow { id: "f2", name: "admin", parent_id: Some("f1") },
        ]
    }

    fn items() -> Vec<ItemRow<'static>> {
        vec![
            ItemRow { id: "a", label: "list".into(), folder_id: Some("f1"), detail: "/users" },
            ItemRow { id: "b", label: "Create".into(), folder_id: Some("f2"), detail: "/users" },
            ItemRow { id: "c", label: "Ping".into(), folder_id: None, detail: "/ping" },
            ItemRow { id: "d", label: "Stranded".into(), folder_id: Some("gone"), detail: "" },
        ]
    }

    fn labels(nodes: &[TreeNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.label.as_str()).collect()
    }

    #[test]
    fn folders_come_first_and_orphans_surface_at_the_top() {
        let tree = build_tree(&folders(), &items(), "");
        assert_eq!(labels(&tree), ["Users", "Ping", "Stranded"]);
        assert_eq!(labels(&tree[0].children), ["admin", "list"]);
        assert!(tree[0].is_folder() && !tree[1].is_folder());
    }

    #[test]
    fn a_filter_keeps_matches_and_the_way_to_them() {
        let tree = build_tree(&folders(), &items(), "create");
        assert_eq!(labels(&tree), ["Users"]);
        assert_eq!(labels(&tree[0].children[0].children), ["Create"]);
        assert_eq!(labels(&build_tree(&folders(), &items(), "/ping")), ["Ping"]);
    }

    #[test]
    fn a_matching_folder_keeps_its_contents() {
        let tree = build_tree(&folders(), &items(), "users");
        assert_eq!(labels(&tree[0].children), ["admin", "list"]);
    }

    #[test]
    fn keys_round_trip() {
        assert_eq!(TreeKey::parse("item:a:b"), Some(TreeKey::Item("a:b".into())));
        assert_eq!(TreeKey::parse(&TreeKey::Folder("x".into()).encode()), Some(TreeKey::Folder("x".into())));
        assert_eq!(TreeKey::parse("nope"), None);
    }

    #[test]
    fn a_parent_cycle_cannot_hang_the_tree() {
        let cyclic = vec![
            FolderRow { id: "a", name: "A", parent_id: Some("b") },
            FolderRow { id: "b", name: "B", parent_id: Some("a") },
        ];
        let _ = build_tree(&cyclic, &[], "");
    }
}
