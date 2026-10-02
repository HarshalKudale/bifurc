//! The commands that read and write saved entities, folders, workspaces and settings.

pub mod config_commands;

use crate::kernel::CommandRegistry;

pub fn register(registry: &mut CommandRegistry) {
    config_commands::register(registry);
}
