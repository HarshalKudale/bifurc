//! What the UI knows, with no GPUI in it: typed views of engine data and the pure logic over them.
//!
//! Everything in this module is testable without a window. The engine is the source of truth; these types
//! are snapshots the views render from.

pub mod agent;
pub mod config;
pub mod cookies;
pub mod explorer;
pub mod flow;
pub mod git;
pub mod graphql;
pub mod history;
pub mod http;
pub mod kind;
pub mod logs;
pub mod mcp;
pub mod media;
pub mod snippet;
pub mod socket;
pub mod socketio;
pub mod stream;
pub mod traffic;
pub mod tree;
pub mod trust;
pub mod update;
pub mod variables;
pub mod vault;
pub mod wsdl;

pub use config::ConfigSnapshot;
pub use explorer::{ExplorerNode, NodeRef, build_explorer};
pub use kind::RequestKind;
