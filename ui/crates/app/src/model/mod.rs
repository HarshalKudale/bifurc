//! What the UI knows, with no GPUI in it: typed views of engine data and the pure logic over them.
//!
//! Everything in this module is testable without a window. The engine is the source of truth; these types
//! are snapshots the views render from.

pub mod config;
pub mod explorer;
pub mod http;
pub mod kind;
pub mod socket;
pub mod traffic;

pub use config::ConfigSnapshot;
pub use explorer::{ExplorerNode, NodeRef, build_explorer};
pub use kind::RequestKind;
