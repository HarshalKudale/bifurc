//! Everything the engine does with a workspace's git repository beyond single commits: accounts and the
//! remote a workspace syncs with, push and pull, auto-sync, per-file status, publishing, discarding, diffs
//! and history.

mod account_commands;
mod auto_sync;
mod delete_commands;
mod git_lock;
mod git_ops;
mod git_state;
mod git_sync_ops;
mod github;
mod login;
mod publish_service;
mod remote;
mod status_tracker;
mod sync_all;
mod sync_commands;
mod sync_manager;
mod sync_state;

#[cfg(test)]
mod remote_tests;
#[cfg(test)]
mod tests;

pub use auto_sync::{start_auto_sync, stop_all_auto_sync};
pub use delete_commands::reconcile_deletions;
pub use git_state::get_workspace_git_state;
pub use status_tracker::get_workspace_sync_status;
pub use sync_state::get_sync_config;

/// Register the sync, git, account and remote-workspace commands.
pub fn register(registry: &mut crate::kernel::CommandRegistry) {
    sync_commands::register(registry);
    delete_commands::register(registry);
    account_commands::register(registry);
}
