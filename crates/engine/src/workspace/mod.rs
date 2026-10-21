//! The workspace store: settings, the on-disk entity layout, the config snapshot and the git repository
//! behind each workspace.

pub mod accounts;
pub mod blob;
pub mod changes;
pub mod claims;
pub mod config;
pub mod contracts;
pub mod deletions;
pub mod dispatch;
pub mod entities;
pub mod entity_paths;
pub mod fs;
pub mod git_store;
pub mod kinds;
pub mod models;
pub mod paths;
pub mod policy;
pub mod response_history;
pub mod settings;
pub mod stamp;
pub mod status_cache;
pub mod tree;
pub mod versions;
