//! Managed applications: the user's local dev servers. The engine generates the command line from an
//! application definition, spawns it, captures its output and reports its state.

pub mod command_generator;
mod commands;
pub mod port_utils;
pub mod process_spawner;
pub mod types;

pub use commands::register;

/// Stop every application the engine started.
pub fn stop_all() {
    process_spawner::stop_all();
}
