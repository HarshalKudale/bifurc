//! The engine<->client wire protocol. It lives in `packages/protocol-rust` (`bifurc-protocol`); the engine
//! reaches it through this path.

pub use bifurc_protocol::commands as schema;
pub use bifurc_protocol::*;
