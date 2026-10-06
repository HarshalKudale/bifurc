//! Where every entity and folder of the active workspace is in its repository, by id.

use indexmap::IndexMap;

use super::{
    config::load_config,
    fs::{entity_rel_path, flat_entity_rel_path, get_pending_deletions, sanitize_dir_name},
    kinds::crud_kind,
};
use crate::utils::json::{array_of, opt_str, str_of};

/// The order kinds are listed in: clients receive these maps, and their key order is part of the result.
const KIND_ORDER: [&str; 11] = [
    "soapRequests",
    "graphqlRequests",
    "grpcRequests",
    "sseRequests",
    "socketioRequests",
    "mappings",
    "rules",
    "mocks",
    "requests",
    "sockets",
    "environments",
];

#[derive(Debug, Default)]
pub struct EntityPathMaps {
    /// entity id -> `kind/[folder/]id.json`
    pub entities: IndexMap<String, String>,
    /// folder id -> `kind/folder/`
    pub folders: IndexMap<String, String>,
}

pub fn load_entity_path_maps() -> EntityPathMaps {
    let cfg = load_config();
    let mut maps = EntityPathMaps::default();
    for kind in KIND_ORDER.iter().filter_map(|name| crud_kind(name)) {
        let folder_list = kind.folder_config_key.map(|key| array_of(&cfg, key)).unwrap_or(&[]);
        for folder in folder_list {
            maps.folders.insert(
                str_of(folder, "id").to_string(),
                format!("{}/{}/", kind.kind, sanitize_dir_name(str_of(folder, "name"))),
            );
        }
        for entity in array_of(&cfg, kind.config_key) {
            let id = str_of(entity, "id");
            let path = if kind.is_flat {
                flat_entity_rel_path(kind.kind, id)
            } else {
                let folder_name = opt_str(entity, "folderId")
                    .filter(|f| !f.is_empty())
                    .and_then(|folder_id| folder_list.iter().find(|f| str_of(f, "id") == folder_id))
                    .map(|f| str_of(f, "name"));
                entity_rel_path(kind.kind, id, folder_name)
            };
            maps.entities.insert(id.to_string(), path);
        }
        // A deleted entity is where it was when it was deleted, whatever became of its folder since.
        for record in get_pending_deletions(str_of(&cfg, "activeWorkspaceId"), kind.kind) {
            if let Some(path) = opt_str(&record, "path") {
                maps.entities.insert(str_of(&record, "id").to_string(), path.to_string());
            }
        }
    }
    maps
}
