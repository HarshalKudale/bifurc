//! The engine version an entity was last written by, and what that locks.
//!
//! Every request, mock, proxy rule, flow, mapping and environment is stamped with the version of the engine that writes it
//! (`engineVersion` in its file). An entity stamped by a newer engine than this one is locked here: it is
//! listed, by name, and nothing else is done with it. It is not read, changed, moved, switched on or deleted
//! until this install is updated, because this engine cannot know what the newer one meant by its fields and
//! writing it back would lose them. A locked mapping is not applied and a locked environment is not resolved
//! from or selected; the Global environment, when locked, adds no variables and is left as it is.

use serde_json::{Value, json};

use super::tree;
use crate::{
    ENGINE_VERSION,
    protocol::{EngineError, ErrorCode},
};

/// The field of an entity file that holds the stamp.
pub const VERSION_FIELD: &str = "engineVersion";

/// What `details.reason` of the error for a locked entity says.
pub const UPDATE_REQUIRED: &str = "updateRequired";

/// Whether the entities of `kind` (the store's name for it) are stamped.
pub fn is_stamped_kind(kind: &str) -> bool {
    matches!(kind, "rules" | "flows" | "mappings" | "environments") || tree::is_tree_kind(kind)
}

/// `major.minor.patch`, with whatever follows a `-` or `+` left out: a pre-release of a version is read
/// by that version.
fn parse(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Whether a stamp is of an engine newer than `engine`. A stamp that is not a version is taken for one this
/// engine does not know yet.
fn is_newer(stamp: &str, engine: &str) -> bool {
    match (parse(stamp), parse(engine)) {
        (Some(stamp), Some(engine)) => stamp > engine,
        _ => true,
    }
}

/// The version an entity needs when this engine is older than the one that last wrote it: it is locked.
pub fn lock_of(entity: &Value) -> Option<String> {
    locked_stamp(entity.get(VERSION_FIELD)?)
}

fn locked_stamp(stamp: &Value) -> Option<String> {
    match stamp {
        Value::Null => None,
        Value::String(version) => is_newer(version, ENGINE_VERSION).then(|| version.clone()),
        other => Some(other.to_string()),
    }
}

/// The version a stored stamp (as an index keeps it) locks an entity to, if it does.
pub fn lock_of_stamp(stamp: Option<&str>) -> Option<String> {
    stamp.filter(|version| is_newer(version, ENGINE_VERSION)).map(str::to_string)
}

/// The stamp `data` is written with: this engine's version, or the newer one it already carries. An entity
/// brought in from a newer engine (an import) therefore arrives locked instead of being taken for this
/// engine's.
pub fn stamp_for(data: &Value) -> Value {
    match data.get(VERSION_FIELD) {
        Some(stamp) if locked_stamp(stamp).is_some() => stamp.clone(),
        _ => json!(ENGINE_VERSION),
    }
}

/// What a write puts in an entity's file: `new`, or what the file holds already when they differ in nothing
/// but the stamp. Saving an entity as it is does not make it a newer engine's.
pub fn settled(on_disk: Option<Value>, new: Value) -> Value {
    let unstamped = |entity: &Value| {
        let mut fields = entity.as_object().cloned().unwrap_or_default();
        fields.shift_remove(VERSION_FIELD);
        fields
    };
    match on_disk {
        Some(on_disk) if unstamped(&on_disk) == unstamped(&new) => on_disk,
        _ => new,
    }
}

/// The error for anything asked of an entity that is locked to `version`.
pub fn update_required(version: &str) -> EngineError {
    EngineError::with_details(
        ErrorCode::Unsupported,
        format!(
            "It was last saved by Bifurc {version}, which is newer than this one ({ENGINE_VERSION}). Update Bifurc to use it."
        ),
        json!({ "reason": UPDATE_REQUIRED, "requiredVersion": version, "engineVersion": ENGINE_VERSION }),
    )
    .keyed("entity.update_required", json!({ "version": version, "current": ENGINE_VERSION }))
}

/// The error for deleting a folder that holds an entity locked to `version`.
pub fn folder_update_required(version: &str) -> EngineError {
    EngineError::with_details(
        ErrorCode::Unsupported,
        format!(
            "Something in this folder was last saved by Bifurc {version}, which is newer than this one ({ENGINE_VERSION}). Update Bifurc to delete the folder."
        ),
        json!({ "reason": UPDATE_REQUIRED, "requiredVersion": version, "engineVersion": ENGINE_VERSION }),
    )
    .keyed("folder.update_required", json!({ "version": version, "current": ENGINE_VERSION }))
}

/// `update_required` for the functions that answer with an `io::Error`.
pub fn update_required_io(version: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, update_required(version).message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_locks_only_when_it_is_newer_than_the_engine() {
        assert!(is_newer("0.6.0", "0.5.0"));
        assert!(is_newer("0.5.1", "0.5.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.5.0", "0.5.0"));
        assert!(!is_newer("0.4.0", "0.5.0"));
        assert!(!is_newer("v0.5.0", "0.5.0"));
        assert!(!is_newer("0.5.0-beta.2", "0.5.0"), "a pre-release is read by its version");
        assert!(is_newer("next", "0.5.0"), "what is not a version is not known yet");
    }

    #[test]
    fn an_entity_without_a_stamp_or_with_this_engines_is_open() {
        assert_eq!(lock_of(&json!({ "name": "old" })), None);
        assert_eq!(lock_of(&json!({ VERSION_FIELD: null })), None);
        assert_eq!(lock_of(&json!({ VERSION_FIELD: ENGINE_VERSION })), None);
        assert_eq!(lock_of(&json!({ VERSION_FIELD: "0.0.1" })), None);
        assert_eq!(lock_of(&json!({ VERSION_FIELD: "999.0.0" })).as_deref(), Some("999.0.0"));
        assert_eq!(lock_of(&json!({ VERSION_FIELD: 7 })).as_deref(), Some("7"));
    }

    #[test]
    fn a_write_stamps_this_engine_but_keeps_a_newer_stamp() {
        assert_eq!(stamp_for(&json!({})), json!(ENGINE_VERSION));
        assert_eq!(stamp_for(&json!({ VERSION_FIELD: "0.0.1" })), json!(ENGINE_VERSION));
        assert_eq!(stamp_for(&json!({ VERSION_FIELD: "999.0.0" })), json!("999.0.0"));
    }

    #[test]
    fn a_write_that_changes_nothing_keeps_the_stamp_the_file_has() {
        let old = json!({ "type": "rest", "name": "Ping" });
        let same = json!({ "type": "rest", VERSION_FIELD: ENGINE_VERSION, "name": "Ping" });
        assert_eq!(settled(Some(old.clone()), same.clone()), old, "an unstamped file stays unstamped");
        let earlier = json!({ "type": "rest", VERSION_FIELD: "0.0.1", "name": "Ping" });
        assert_eq!(settled(Some(earlier.clone()), same.clone()), earlier);
        let changed = json!({ "type": "rest", VERSION_FIELD: ENGINE_VERSION, "name": "Pong" });
        assert_eq!(settled(Some(earlier), changed.clone()), changed);
        assert_eq!(settled(None, same.clone()), same);
    }

    #[test]
    fn the_error_names_the_version_that_is_needed() {
        let error = update_required("999.0.0");
        assert_eq!(error.code, ErrorCode::Unsupported);
        let details = error.details.unwrap();
        assert_eq!(details["reason"], UPDATE_REQUIRED);
        assert_eq!(details["requiredVersion"], "999.0.0");
    }
}
