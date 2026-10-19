//! The MCP client request type: the engine connects to a Model Context Protocol server (a program it starts
//! and speaks to over stdio, or an HTTP endpoint), reads the tools, prompts and resources the server declares,
//! calls them, keeps the log of what was asked and answered and reports status and log lines as `mcp.status` /
//! `mcp.message` events. The protocol itself is the official SDK's (`rmcp`).

mod answer;
mod arguments;
pub(crate) mod client;
mod commands;
mod registry;

use crate::kernel::CommandRegistry;

pub use registry::{MAX_MCP_CONNECTIONS, MAX_MCP_MESSAGES};

pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
}
