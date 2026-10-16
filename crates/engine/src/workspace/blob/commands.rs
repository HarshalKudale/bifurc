//! `blob.put`, `blob.stat`, `blob.read` and `blob.release`, bound to the store.
//!
//! The namespace is always the `workspaceId` of the call, never the active workspace.

use serde_json::Value;

use super::store::{put_blob, read_blob, release_blob, stat_blob};
use crate::{
    kernel::CommandRegistry,
    utils::{
        id::now_ms,
        json::{opt_str, str_of},
    },
};

fn whole_number(params: &Value, key: &str) -> Option<i64> {
    let value = params.get(key)?;
    value.as_i64().or_else(|| value.as_f64().map(|f| f as i64))
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("blob.put", |params, _| {
        let size = whole_number(&params, "size").unwrap_or(0);
        Ok(put_blob(
            str_of(&params, "filename"),
            str_of(&params, "mimeType"),
            size,
            str_of(&params, "data"),
            now_ms(),
            opt_str(&params, "workspaceId"),
        )?)
    });

    registry.register_sync("blob.stat", |params, _| {
        Ok(stat_blob(str_of(&params, "blobId"), now_ms(), opt_str(&params, "workspaceId"))?)
    });

    registry.register_sync("blob.read", |params, _| {
        let offset = whole_number(&params, "offset").map(|n| n.max(0) as u64);
        let length = whole_number(&params, "length").map(|n| n.max(0) as u64);
        Ok(read_blob(str_of(&params, "blobId"), offset, length, now_ms(), opt_str(&params, "workspaceId"))?)
    });

    registry.register_sync("blob.release", |params, _| {
        Ok(release_blob(str_of(&params, "blobId"), opt_str(&params, "workspaceId"))?)
    });
}
