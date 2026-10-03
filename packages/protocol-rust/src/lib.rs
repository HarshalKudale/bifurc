//! `bifurc-protocol` — the versioned engine<->client wire contract, shared by the engine, the CLI and the UI.
//!
//! Mirrors `packages/protocol` (version, errors, envelope, events, command schemas). The command schemas are
//! `schemas/commands.json`; `schemas/fixtures.json` holds one valid and one invalid payload per command, and
//! the tests run every one through the validator.

pub mod commands;
pub mod envelope;
pub mod errors;
pub mod events;
pub mod version;

pub use envelope::{
    CLIENT_NAMES, Capability, ClientName, HELLO_ACTION, HelloRequest, HelloResponse, RESERVED_ACTIONS, RpcRequest,
    RpcResponse, SubscribeRequest, UnsubscribeRequest, capability,
};
pub use errors::{EngineError, ErrorCode, Result, RpcError};
pub use events::{
    EVENT_NAMES, EventEnvelope, ResyncReason, ResyncRequiredEvent, bus_name_for, is_wire_event, wire_name_for,
};
pub use version::{PROTOCOL_VERSION, ParsedVersion, VersionCompatibility, check_version_compatibility, parse_version};

/// The companion extension's commands: a frozen public API (it speaks the legacy wire shape directly).
pub const EXTENSION_FROZEN_COMMANDS: [&str; 4] = ["mock:add", "request:add", "folder:add", "config:get"];
