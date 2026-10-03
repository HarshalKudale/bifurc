//! The Bifurc command line as a library: `main.rs` only calls [`cli::run`].
//!
//! The CLI holds no application logic. It finds the engine that serves a data directory through
//! `<data-dir>/runtime.json`, opens a session over its socket, and turns one command line into one or a few
//! engine commands and a printed answer. Argument parsing, formatting and engine location are pure modules.

pub mod args;
pub mod cli;
pub mod commands;
pub mod discovery;
pub mod error;
pub mod format;
pub mod locate;
pub mod prompt;
pub mod protocol;
pub mod session;
pub mod supervisor;
pub mod unlock;
pub mod usage;
