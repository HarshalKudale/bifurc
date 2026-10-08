//! Whole-workspace exports: a JSON snapshot and a zip of the workspace directory.

use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{
    importexport::{
        support::{in_workspace, pretty},
        types::{ExportFile, ExportOutcome},
    },
    utils::json::{array_of, merged, str_of},
    workspace::{
        config::load_config,
        fs::{load_enabled_set, read_all_entities, ws_dir},
    },
};

fn owned_by(entities: Vec<Value>, ws_id: &str) -> Vec<Value> {
    entities.into_iter().filter(|e| str_of(e, "workspaceId") == ws_id).collect()
}

/// The `lp-workspace-v1` snapshot of workspace `ws_id`.
pub fn json(ws_id: &str) -> ExportOutcome {
    let cfg = load_config();
    let Some(workspace) = array_of(&cfg, "workspaces").iter().find(|w| str_of(w, "id") == ws_id) else {
        return Err("Workspace not found".to_string());
    };

    let enabled_rules = load_enabled_set(ws_id, "rules");
    let proxy_rules: Vec<Value> = owned_by(read_all_entities(ws_id, "rules"), ws_id)
        .iter()
        .map(|r| merged(r, &json!({ "enabled": enabled_rules.contains(str_of(r, "id")) })))
        .collect();

    let snapshot = json!({
        "schema": "lp-workspace-v1",
        "workspace": workspace,
        "data": {
            "mappings": in_workspace(&cfg, "mappings", ws_id),
            "proxyRules": proxy_rules,
            "mocks": in_workspace(&cfg, "mocks", ws_id),
            "requests": owned_by(read_all_entities(ws_id, "requests"), ws_id),
            "mockFolders": in_workspace(&cfg, "mockFolders", ws_id),
            "ruleFolders": in_workspace(&cfg, "ruleFolders", ws_id),
            "requestFolders": in_workspace(&cfg, "requestFolders", ws_id),
            "wsConnections": owned_by(read_all_entities(ws_id, "sockets"), ws_id),
            "environments": in_workspace(&cfg, "environments", ws_id),
        },
    });
    Ok(ExportFile { content: pretty(&snapshot), suggested_name: "workspace-export.json" })
}

/// Every file and directory under `dir` except the top-level `.git`, as `(path inside the archive, path on disk)`.
fn archive_entries(dir: &Path) -> std::io::Result<Vec<(String, PathBuf, bool)>> {
    fn walk(dir: &Path, prefix: &str, top: bool, out: &mut Vec<(String, PathBuf, bool)>) -> std::io::Result<()> {
        let mut children: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for entry in children {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Git's data stays here, and so do the indexes of the trees: they are made from the files.
            if (top && name == ".git") || name.starts_with(crate::workspace::tree::INDEX_FILE) {
                continue;
            }
            let relative = format!("{prefix}{name}");
            if entry.file_type()?.is_dir() {
                out.push((format!("{relative}/"), entry.path(), true));
                walk(&entry.path(), &format!("{relative}/"), false, out)?;
            } else {
                out.push((relative, entry.path(), false));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, "", true, &mut out)?;
    Ok(out)
}

fn write_zip(source: &Path, dest: &Path) -> Result<(), String> {
    let fail = |err: &dyn std::fmt::Display| err.to_string();
    let entries = archive_entries(source).map_err(|e| fail(&e))?;
    let mut writer = ZipWriter::new(File::create(dest).map_err(|e| fail(&e))?);
    let options =
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).compression_level(Some(6));
    for (name, path, is_dir) in entries {
        if is_dir {
            writer.add_directory(name, options).map_err(|e| fail(&e))?;
        } else {
            writer.start_file(name, options).map_err(|e| fail(&e))?;
            let mut file = File::open(path).map_err(|e| fail(&e))?;
            std::io::copy(&mut file, &mut writer).map_err(|e| fail(&e))?;
        }
    }
    writer.finish().map_err(|e| fail(&e))?.flush().map_err(|e| fail(&e))
}

/// A zip archive of workspace `ws_id`'s directory (without its git history) at `dest`.
pub fn zip(ws_id: &str, dest: &Path) -> Result<(), String> {
    let source = ws_dir(ws_id);
    if !source.exists() {
        return Err("Workspace directory not found".to_string());
    }
    write_zip(&source, dest)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::fs::{sync_enabled_set, write_entity},
    };

    #[test]
    fn the_snapshot_holds_the_workspace_in_the_documented_order() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let request = json!({ "id": "r1", "name": "Ping", "method": "GET", "url": "http://a/", "headers": {}, "body": "",
            "createdAt": 1, "folderId": null, "workspaceId": ws });
        write_entity(ws, "requests", "r1", &request, None).unwrap();
        let rule = json!({ "id": "p1", "name": "Rule", "pattern": "x", "workspaceId": ws });
        write_entity(ws, "rules", "p1", &rule, None).unwrap();
        sync_enabled_set(ws, "rules", "p1", true).unwrap();

        let out = json(ws).unwrap();
        assert_eq!(out.suggested_name, "workspace-export.json");
        let snapshot: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(snapshot["schema"], "lp-workspace-v1");
        assert_eq!(snapshot["workspace"]["id"], ws);
        let keys: Vec<_> = snapshot["data"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "mappings",
                "proxyRules",
                "mocks",
                "requests",
                "mockFolders",
                "ruleFolders",
                "requestFolders",
                "wsConnections",
                "environments"
            ]
        );
        assert_eq!(snapshot["data"]["requests"][0]["id"], "r1");
        assert_eq!(snapshot["data"]["proxyRules"][0]["enabled"], true);
        assert_eq!(snapshot["data"]["environments"][0]["id"], "__global__");
        assert_eq!(json("nope").err().unwrap(), "Workspace not found");
    }

    #[test]
    fn the_zip_holds_the_workspace_files_but_not_git() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        std::fs::create_dir_all(ws_dir(ws).join(".git")).unwrap();
        std::fs::write(ws_dir(ws).join(".git").join("HEAD"), "ref").unwrap();
        let dest = root.dir.path().join("out.zip");
        zip(ws, &dest).unwrap();

        let mut archive = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();
        assert!(names.contains(&"workspace.json".to_string()), "{names:?}");
        assert!(names.contains(&".gitignore".to_string()));
        assert!(names.iter().all(|n| !n.starts_with(".git/") && n != ".git/"));
        let mut text = String::new();
        archive.by_name("workspace.json").unwrap().read_to_string(&mut text).unwrap();
        assert!(text.contains("Workspace 1"));
        assert_eq!(zip("missing", &dest).err().unwrap(), "Workspace directory not found");
    }
}
