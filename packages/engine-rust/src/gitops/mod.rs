//! Everything the engine does with a workspace's git repository beyond single commits: remotes, push and
//! pull, auto-sync, per-file status, publishing, discarding, diffs and history.

mod auto_sync;
mod git_lock;
mod git_ops;
mod git_state;
mod git_sync_ops;
mod publish_service;
mod status_tracker;
mod sync_commands;
mod sync_manager;
mod sync_state;

#[cfg(test)]
mod tests;

pub use auto_sync::{start_auto_sync, stop_all_auto_sync};
pub use git_state::get_workspace_git_state;
pub use status_tracker::get_workspace_sync_status;
pub use sync_commands::register;
pub use sync_state::get_sync_config;
