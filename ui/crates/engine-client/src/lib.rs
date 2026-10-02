//! A Rust client for the Bifurc engine: the socket transport, the handshake, and the sidecar rendezvous.
//!
//! The engine is a separate process (`bifurc-engine`) that every client — this UI, the `bifurc` CLI, the
//! Electron shell — reaches over the same wire. This crate holds no application logic and no GPUI: it moves
//! JSON frames and finds or starts the engine. What the frames mean is the protocol's business
//! (`packages/protocol`), and the typed views the UI needs live in the UI crate.

mod connection;
mod error;
mod frame;
mod sidecar;

pub use connection::{CLIENT_NAME, EngineClient, EngineEvent, Hello, Incoming, PROTOCOL_VERSION};
pub use error::{ClientError, EngineError, Result};
pub use sidecar::{
    RuntimeInfo, Sidecar, SidecarConfig, attach_or_start, default_data_dir, locate_engine_entry, read_runtime_info,
    reconnect,
};
