//! The workspace store: settings, the on-disk entity layout, the config snapshot and the git repository
//! behind each workspace.

pub mod blob;
pub mod config;
pub mod dispatch;
pub mod entities;
pub mod entity_paths;
pub mod fs;
pub mod git_store;
pub mod kinds;
pub mod paths;
pub mod settings;
pub mod status_cache;
