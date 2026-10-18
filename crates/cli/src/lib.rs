//! The Bifurc command line as a library: `main.rs` only calls [`cli::run`].
//!
//! The CLI holds no application logic. It finds the engine that serves a data directory through
//! `<data-dir>/runtime.json`, opens a session over its socket, and turns one command line into one or a few
//! engine commands and a printed answer. Argument parsing, formatting and engine location are pure modules.

pub mod agent;
pub mod approvals;
pub mod args;
#[cfg(test)]
mod budget;
pub mod capture;
pub mod changes;
pub mod claims;
pub mod cli;
pub mod commands;
pub mod contract;
pub mod curl;
pub mod devservices;
pub mod direct;
pub mod discovery;
pub mod entities;
pub mod error;
pub mod extras;
pub mod flows;
pub mod flowsteps;
pub mod format;
pub mod health;
pub mod infer;
pub mod jwt;
pub mod locate;
pub mod logtrace;
pub mod mail;
pub mod mcp;
pub mod named;
pub mod openapi;
pub mod policy;
pub mod prompt;
pub mod protocol;
pub mod record;
pub mod replay;
pub mod requests;
pub mod response;
pub mod select;
pub mod session;
pub mod setup;
pub mod shape;
pub mod socketio;
pub mod steps;
pub mod stream;
pub mod supervisor;
#[cfg(test)]
pub(crate) mod testing;
pub mod types;
pub mod update;
pub mod usage;
pub mod vault;
pub mod wait;
pub mod webhooks;
