//! The Bifurc command line as a library: `main.rs` only calls [`cli::run`].
//!
//! The CLI holds no application logic. It finds the engine that serves a data directory through
//! `<data-dir>/runtime.json`, opens a session over its socket, and turns one command line into one or a few
//! engine commands and a printed answer. Argument parsing, formatting and engine location are pure modules.

pub mod args;
pub mod cli;
pub mod commands;
pub mod direct;
pub mod discovery;
pub mod entities;
pub mod error;
pub mod extras;
pub mod flows;
pub mod format;
pub mod health;
pub mod locate;
pub mod mcp;
pub mod prompt;
pub mod protocol;
pub mod requests;
pub mod response;
pub mod select;
pub mod session;
pub mod socketio;
pub mod stream;
pub mod supervisor;
#[cfg(test)]
pub(crate) mod testing;
pub mod update;
pub mod usage;
pub mod vault;
