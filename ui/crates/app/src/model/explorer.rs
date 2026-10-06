//! The explorer tree: every saved request of every type, in the folders the engine keeps for each type.
//!
//! Folders belong to one request type (the engine keeps `requestFolders`, `wsFolders`, ... separately), so a
//! folder only ever contains requests of its own type. The explorer merges them into one tree: folders first,
//! then requests, each sorted by name, with the type shown on the row. Opening a WebSocket from here is the
//! same gesture as opening a REST request.

use std::collections::{HashMap, HashSet};

use super::{
    config::{ConfigSnapshot, RequestStub},
    kind::RequestKind,
};

/// Identity of a tree row. Stable across refreshes, which is what keeps selection and expansion in place.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeRef {
    Folder { kind: RequestKind, id: String },
    Request { kind: RequestKind, id: String },
}

impl NodeRef {
    /// The string form used as a tree item id.
    pub fn encode(&self) -> String {
        let (tag, kind, id) = match self {
            NodeRef::Folder { kind, id } => ("folder", kind, id),
            NodeRef::Request { kind, id } => ("request", kind, id),
        };
        format!("{tag}|{}|{id}", kind.entity_kind())
    }

    pub fn parse(encoded: &str) -> Option<Self> {
        let mut parts = encoded.splitn(3, '|');
        let (tag, entity_kind, id) = (parts.next()?, parts.next()?, parts.next()?);
        let kind = RequestKind::ALL.into_iter().find(|k| k.entity_kind() == entity_kind)?;
        let id = id.to_owned();
        match tag {
            "folder" => Some(NodeRef::Folder { kind, id }),
            "request" => Some(NodeRef::Request { kind, id }),
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

/// Builds the tree. A non-empty `filter` keeps requests whose name or target contains it (case-insensitive)
/// and the folders on the way to them; a folder whose own name matches is kept with everything inside it.
pub fn build_explorer(config: &ConfigSnapshot, filter: &str) -> Vec<ExplorerNode> {
    let needle = filter.trim().to_lowercase();
    let filtering = !needle.is_empty();
    let matches = |text: &str| text.to_lowercase().contains(&needle);

    let mut requests_by_folder: HashMap<(RequestKind, Option<&str>), Vec<&RequestStub>> = HashMap::new();
    for request in &config.requests {
        requests_by_folder.entry((request.kind, request.folder_id.as_deref())).or_default().push(request);
    }
    let known_folders: HashSet<(RequestKind, &str)> = config.folders.iter().map(|f| (f.kind, f.id.as_str())).collect();

    struct Walk<'a> {
        config: &'a ConfigSnapshot,
        requests_by_folder: &'a HashMap<(RequestKind, Option<&'a str>), Vec<&'a RequestStub>>,
        known_folders: &'a HashSet<(RequestKind, &'a str)>,
        filtering: bool,
        matches: &'a dyn Fn(&str) -> bool,
    }

    impl Walk<'_> {
        fn level(
            &self,
            kind: RequestKind,
            parent: Option<&str>,
            keep_all: bool,
            seen: &mut HashSet<String>,
        ) -> Vec<ExplorerNode> {
            let mut folders: Vec<ExplorerNode> = Vec::new();
            for folder in self.config.folders.iter().filter(|f| {
                f.kind == kind
                    && match (&f.parent_id, parent) {
                        (None, None) => true,
                        (Some(p), Some(q)) => p == q,
                        // A folder whose parent no longer exists is shown at the top rather than lost.
                        (Some(p), None) => !self.known_folders.contains(&(kind, p.as_str())),
                        _ => false,
                    }
            }) {
                if !seen.insert(folder.id.clone()) {
                    continue; // a cycle in the stored parents; the first visit wins
                }
                let own_match = self.filtering && (self.matches)(&folder.name);
                let children = self.level(kind, Some(&folder.id), keep_all || own_match || !self.filtering, seen);
                if !self.filtering || keep_all || own_match || !children.is_empty() {
                    folders.push(ExplorerNode {
                        node: NodeRef::Folder { kind, id: folder.id.clone() },
                        label: folder.name.clone(),
                        badge: String::new(),
                        children,
                    });
                }
            }

            let mut requests: Vec<ExplorerNode> = self
                .requests_by_folder
                .get(&(kind, parent))
                .into_iter()
                .flatten()
                .filter(|r| !self.filtering || keep_all || (self.matches)(&r.name) || (self.matches)(&r.target))
                .map(|r| ExplorerNode {
                    node: NodeRef::Request { kind, id: r.id.clone() },
                    label: display_name(r),
                    badge: r.method.clone().unwrap_or_else(|| kind.badge().to_owned()),
                    children: Vec::new(),
                })
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

    // Requests whose folder is gone surface at the top, the same as orphaned folders.
    let mut roots: Vec<ExplorerNode> = Vec::new();
    for kind in RequestKind::ALL {
        let mut seen = HashSet::new();
        roots.extend(walk.level(kind, None, false, &mut seen));
        for ((k, folder), stranded) in &requests_by_folder {
            let Some(folder) = folder else { continue };
            if *k != kind || known_folders.contains(&(kind, folder)) {
                continue;
            }
            roots.extend(stranded.iter().filter(|r| !filtering || matches(&r.name) || matches(&r.target)).map(|r| {
                ExplorerNode {
                    node: NodeRef::Request { kind, id: r.id.clone() },
                    label: display_name(r),
                    badge: r.method.clone().unwrap_or_else(|| kind.badge().to_owned()),
                    children: Vec::new(),
                }
            }));
        }
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
            ],
            "wsConnections": [{"id": "s1", "name": "Echo", "url": "wss://echo.test", "folderId": "wf"}],
            "wsFolders": [{"id": "wf", "name": "Live", "parentId": null}],
            "sseRequests": [{"id": "e1", "name": "Feed", "url": "https://sse.test/events", "folderId": "ef"}],
            "sseRequestFolders": [{"id": "ef", "name": "Streams", "parentId": null}],
        }))
        .unwrap()
    }

    fn labels(nodes: &[ExplorerNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.label.as_str()).collect()
    }

    #[test]
    fn folders_come_first_then_requests_each_sorted_without_regard_to_case() {
        let tree = build_explorer(&config(), "");
        assert_eq!(labels(&tree), ["Live", "Streams", "Users", "Ping", "Stranded"]);
    }

    #[test]
    fn an_sse_request_sits_in_its_own_folders_with_its_own_badge() {
        let tree = build_explorer(&config(), "");
        let streams = tree.iter().find(|n| n.label == "Streams").unwrap();
        assert_eq!(labels(&streams.children), ["Feed"]);
        assert_eq!(streams.children[0].badge, "SSE");
        assert_eq!(streams.children[0].node, NodeRef::Request { kind: RequestKind::Sse, id: "e1".into() });
    }

    #[test]
    fn a_websocket_sits_in_its_own_folders_beside_the_others() {
        let tree = build_explorer(&config(), "");
        let live = tree.iter().find(|n| n.label == "Live").unwrap();
        assert_eq!(labels(&live.children), ["Echo"]);
        assert_eq!(live.children[0].badge, "WS");
        assert_eq!(live.children[0].node, NodeRef::Request { kind: RequestKind::WebSocket, id: "s1".into() });
    }

    #[test]
    fn rest_rows_show_their_method_and_nest_by_parent_folder() {
        let tree = build_explorer(&config(), "");
        let users = tree.iter().find(|n| n.label == "Users").unwrap();
        assert_eq!(labels(&users.children), ["Admin", "list users"]);
        assert_eq!(users.children[1].badge, "GET");
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
        assert_eq!(labels(&users.children), ["Admin", "list users"]);
    }

    #[test]
    fn filtering_also_matches_the_target() {
        assert_eq!(labels(&build_explorer(&config(), "wss://echo")), ["Live"]);
    }

    #[test]
    fn node_ids_round_trip_including_ids_with_separators() {
        let node = NodeRef::Request { kind: RequestKind::WebSocket, id: "a|b|c".into() };
        assert_eq!(NodeRef::parse(&node.encode()), Some(node));
        assert_eq!(NodeRef::parse("garbage"), None);
        assert_eq!(NodeRef::parse("folder|nope|x"), None);
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
