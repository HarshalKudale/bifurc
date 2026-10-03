//! Import and export of workspace data in other tools' formats: `import.preflight`, `import.commit`,
//! `export.formats` and `export.create`.

mod commands;
mod exporters;
mod formats;
mod importers;
mod registry;
mod support;
mod types;

use crate::kernel::CommandRegistry;

/// Register the import and export commands.
pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
}
