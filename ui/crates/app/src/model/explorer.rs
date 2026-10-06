//! The explorer tree: every saved request of every type, in the folders of the request tree.
//!
//! The engine keeps one tree of folders for all requests (a folder is a directory of the workspace, and a
//! folder in a folder a directory in a directory), so a folder holds requests of any type side by side. The
//! explorer shows that tree as it is: folders first, then requests, each sorted by name, with the type shown
//! on the row. Opening a WebSocket from here is the same gesture as opening a REST request.

use std::collections::{HashMap, HashSet};

use super::{
    config::{ConfigSnapshot, RequestStub},
    kind::RequestKind,
};

/// Identity of a tree row. Stable across refreshes, which is what keeps selection and expansion in place.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeRef {
    Folder { id: String },
    Request { kind: RequestKind, id: String },
}

/// What stands where a request's type does in the id of a folder row: a folder has no type.
const FOLDER_KIND: &str = "-";

impl NodeRef {
    /// The string form used as a tree item id.
    pub fn encode(&self) -> String {
        match self {
            NodeRef::Folder { id } => format!("folder|{FOLDER_KIND}|{id}"),
            NodeRef::Request { kind, id } => format!("request|{}|{id}", kind.entity_kind()),
        }
    }

    pub fn parse(encoded: &str) -> Option<Self> {
        let mut parts = encoded.splitn(3, '|');
        let (tag, entity_kind, id) = (parts.next()?, parts.next()?, parts.next()?);
        let id = id.to_owned();
        match tag {
            "folder" if entity_kind == FOLDER_KIND => Some(NodeRef::Folder { id }),
            "request" => {
                let kind = RequestKind::ALL.into_iter().find(|k| k.entity_kind() == entity_kind)?;
                Some(NodeRef::Request { kind, id })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplorerNode {
    pub node: NodeRef,
    pub label: String,
    /// The method for REST, the type tag for everything else; empty for folders.
    pub badge: String,
    pub children: Vec<ExplorerNode>,
}

impl ExplorerNode {
    pub fn is_folder(&self) -> bool {
        matches!(self.node, NodeRef::Folder { .. })
    }
}

fn display_name(stub: &RequestStub) -> String {
    if !stub.name.is_empty() {
        stub.name.clone()
    } else if !stub.target.is_empty() {
        stub.target.clone()
    } else {
        "Untitled".to_owned()
    }
}

fn sort_key(label: &str) -> String {
    label.to_lowercase()
}

fn request_node(request: &RequestStub) -> ExplorerNode {
    ExplorerNode {
        node: NodeRef::Request { kind: request.kind, id: request.id.clone() },
        label: display_name(request),
        badge: request.method.clone().unwrap_or_else(|| request.kind.badge().to_owned()),
        children: Vec::new(),
    }
}

/// Builds the tree. A non-empty `filter` keeps requests whose name or target contains it (case-insensitive)
/// and the folders on the way to them; a folder whose own name matches is kept with everything inside it.
pub fn build_explorer(config: &ConfigSnapshot, filter: &str) -> Vec<ExplorerNode> {
    let needle = filter.trim().to_lowercase();
    let filtering = !needle.is_empty();
    let matches = |text: &str| text.to_lowercase().contains(&needle);

    let mut requests_by_folder: HashMap<Option<&str>, Vec<&RequestStub>> = HashMap::new();
    for request in &config.requests {
        requests_by_folder.entry(request.folder_id.as_deref()).or_default().push(request);
    }
    let known_folders: HashSet<&str> = config.folders.iter().map(|f| f.id.as_str()).collect();

    struct Walk<'a> {
        config: &'a ConfigSnapshot,
        requests_by_folder: &'a HashMap<Option<&'a str>, Vec<&'a RequestStub>>,
        known_folders: &'a HashSet<&'a str>,
        filtering: bool,
        matches: &'a dyn Fn(&str) -> bool,
    }

    impl Walk<'_> {
        fn level(&self, parent: Option<&str>, keep_all: bool, seen: &mut HashSet<String>) -> Vec<ExplorerNode> {
            let mut folders: Vec<ExplorerNode> = Vec::new();
            for folder in self.config.folders.iter().filter(|f| match (&f.parent_id, parent) {
                (None, None) => true,
                (Some(p), Some(q)) => p == q,
                // A folder whose parent no longer exists is shown at the top rather than lost.
                (Some(p), None) => !self.known_folders.contains(p.as_str()),
                _ => false,
            }) {
                if !seen.insert(folder.id.clone()) {
                    continue; // a cycle in the stored parents; the first visit wins
                }
                let own_match = self.filtering && (self.matches)(&folder.name);
                let children = self.level(Some(&folder.id), keep_all || own_match || !self.filtering, seen);
                if !self.filtering || keep_all || own_match || !children.is_empty() {
                    folders.push(ExplorerNode {
                        node: NodeRef::Folder { id: folder.id.clone() },
                        label: folder.name.clone(),
                        badge: String::new(),
                        children,
                    });
                }
            }

            let mut requests: Vec<ExplorerNode> = self
                .requests_by_folder
                .get(&parent)
                .into_iter()
                .flatten()
                .filter(|r| !self.filtering || keep_all || (self.matches)(&r.name) || (self.matches)(&r.target))
                .map(|r| request_node(r))
                .collect();

            folders.sort_by_key(|n| sort_key(&n.label));
            requests.sort_by_key(|n| sort_key(&n.label));
            folders.extend(requests);
            folders
        }
    }

    let walk = Walk {
        config,
        requests_by_folder: &requests_by_folder,
        known_folders: &known_folders,
        filtering,
        matches: &matches,
    };

    let mut seen = HashSet::new();
    let mut roots = walk.level(None, false, &mut seen);
    // Requests whose folder is gone surface at the top, the same as orphaned folders.
    for (folder, stranded) in &requests_by_folder {
        let Some(folder) = folder else { continue };
        if known_folders.contains(folder) {
            continue;
        }
        roots.extend(
            stranded.iter().filter(|r| !filtering || matches(&r.name) || matches(&r.target)).map(|r| request_node(r)),
        );
    }
    let (mut folders, mut requests): (Vec<_>, Vec<_>) = roots.into_iter().partition(ExplorerNode::is_folder);
    folders.sort_by_key(|n| sort_key(&n.label));
    requests.sort_by_key(|n| sort_key(&n.label));
    folders.extend(requests);
    folders
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> ConfigSnapshot {
        ConfigSnapshot::from_value(&json!({
            "activeWorkspaceId": "w",
            "requests": [
                {"id": "r1", "name": "list users", "method": "GET", "url": "/users", "folderId": "f1"},
                {"id": "r2", "name": "Create user", "method": "POST", "url": "/users", "folderId": "f2"},
                {"id": "r3", "name": "Ping", "method": "GET", "url": "/ping"},
                {"id": "r4", "name": "Stranded", "method": "GET", "url": "/x", "folderId": "gone"},
            ],
            "requestFolders": [
                {"id": "f1", "name": "Users", "parentId": null},
                {"id": "f2", "name": "Admin", "parentId": "f1"},
                {"id": "lf", "name": "Live", "parentId": null},
            ],
            "wsConnections": [{"id": "s1", "name": "Echo", "url": "wss://echo.test", "folderId": "lf"}],
            "sseRequests": [{"id": "e1", "name": "Feed", "url": "https://sse.test/events", "folderId": "lf"}],
            "socketioRequests": [{"id": "i1", "name": "Chat", "url": "https://io.test", "folderId": "f1"}],
        }))
        .unwrap()
    }

    fn labels(nodes: &[ExplorerNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.label.as_str()).collect()
    }

    #[test]
    fn folders_come_first_then_requests_each_sorted_without_regard_to_case() {
        let tree = build_explorer(&config(), "");
        assert_eq!(labels(&tree), ["Live", "Users", "Ping", "Stranded"]);
    }

    #[test]
    fn a_folder_holds_requests_of_every_type_side_by_side() {
        let tree = build_explorer(&config(), "");
        let live = tree.iter().find(|n| n.label == "Live").unwrap();
        assert_eq!(labels(&live.children), ["Echo", "Feed"]);
        assert_eq!(live.children[0].badge, "WS");
        assert_eq!(live.children[0].node, NodeRef::Request { kind: RequestKind::WebSocket, id: "s1".into() });
        assert_eq!(live.children[1].badge, "SSE");
        assert_eq!(live.children[1].node, NodeRef::Request { kind: RequestKind::Sse, id: "e1".into() });

        // A Socket.IO request beside REST requests, in a folder that also holds a folder.
        let users = tree.iter().find(|n| n.label == "Users").unwrap();
        assert_eq!(labels(&users.children), ["Admin", "Chat", "list users"]);
        assert_eq!(users.children[1].badge, "SIO");
        assert_eq!(users.children[1].node, NodeRef::Request { kind: RequestKind::SocketIo, id: "i1".into() });
    }

    #[test]
    fn rest_rows_show_their_method_and_nest_by_parent_folder() {
        let tree = build_explorer(&config(), "");
        let users = tree.iter().find(|n| n.label == "Users").unwrap();
        assert_eq!(users.children[2].badge, "GET");
        assert_eq!(labels(&users.children[0].children), ["Create user"]);
    }

    #[test]
    fn a_request_in_a_missing_folder_is_not_lost() {
        assert!(labels(&build_explorer(&config(), "")).contains(&"Stranded"));
    }

    #[test]
    fn filtering_keeps_matches_and_the_folders_that_lead_to_them() {
        let tree = build_explorer(&config(), "create");
        assert_eq!(labels(&tree), ["Users"]);
        assert_eq!(labels(&tree[0].children), ["Admin"]);
        assert_eq!(labels(&tree[0].children[0].children), ["Create user"]);
    }

    #[test]
    fn a_folder_whose_name_matches_keeps_everything_in_it() {
        let tree = build_explorer(&config(), "users");
        let users = tree.iter().find(|n| n.label == "Users").unwrap();
        assert_eq!(labels(&users.children), ["Admin", "Chat", "list users"]);
    }

    #[test]
    fn filtering_also_matches_the_target() {
        let tree = build_explorer(&config(), "wss://echo");
        assert_eq!(labels(&tree), ["Live"]);
        assert_eq!(labels(&tree[0].children), ["Echo"]);
    }

    #[test]
    fn node_ids_round_trip_including_ids_with_separators() {
        let node = NodeRef::Request { kind: RequestKind::WebSocket, id: "a|b|c".into() };
        assert_eq!(NodeRef::parse(&node.encode()), Some(node));
        let folder = NodeRef::Folder { id: "f|1".into() };
        assert_eq!(NodeRef::parse(&folder.encode()), Some(folder));
        assert_eq!(NodeRef::parse("garbage"), None);
        assert_eq!(NodeRef::parse("folder|nope|x"), None);
        assert_eq!(NodeRef::parse("request|nope|x"), None);
    }

    #[test]
    fn a_cycle_in_stored_folder_parents_cannot_hang_the_tree() {
        let c = ConfigSnapshot::from_value(&json!({
            "activeWorkspaceId": "w",
            "requestFolders": [
                {"id": "a", "name": "A", "parentId": "b"},
                {"id": "b", "name": "B", "parentId": "a"},
            ],
        }))
        .unwrap();
        // Neither folder is reachable from the top; the point is that this returns.
        let _ = build_explorer(&c, "");
    }
}
