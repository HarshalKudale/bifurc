//! Bifurc's engine: the localhost proxies, mocks, the workspace store, git sync and the request tools,
//! behind one RPC surface that the desktop UI, the CLI and the browser extension share.
//!
//! The engine is single-threaded in the way that matters: commands run on one async thread, so a handler's
//! read-modify-write of a workspace file cannot interleave with another's between awaits. Work that would
//! block that thread for long (a user script) runs on the blocking pool.

pub mod applications;
pub mod capture;
pub mod certificates;
pub mod env;
pub mod fileops;
pub mod gitops;
pub mod graphql;
pub mod grpc;
pub mod importexport;
pub mod kernel;
pub mod mocks;
pub mod protocol;
pub mod proxy;
pub mod register_all;
pub mod rest;
pub mod reverseproxy;
pub mod runner;
pub mod runtime;
pub mod scripting;
pub mod services;
pub mod soap;
pub mod transport;
pub mod utils;
pub mod vault;
pub mod websocket;
pub mod workspace;

/// The engine's version, reported in the handshake and in `runtime.json`.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
pub mod testing;
