//! Import and export of workspace data: `import.preflight`, `import.commit`, `export.formats` and
//! `export.create`. An export is Bifurc JSON; an import also reads the formats of other tools.

mod commands;
mod convert;
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
