//! The commands that read and write saved entities, folders, workspaces and settings.

pub mod audit_commands;
pub mod config_commands;
pub mod core_commands;
pub mod entity_crud;
pub mod folder_commands;

use crate::kernel::CommandRegistry;

pub fn register(registry: &mut CommandRegistry) {
    config_commands::register(registry);
    core_commands::register(registry);
    entity_crud::register(registry);
    folder_commands::register(registry);
    audit_commands::register(registry);
}
