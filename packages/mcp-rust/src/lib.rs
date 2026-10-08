//! The Bifurc MCP server as a library: `main.rs` only reads its options and calls [`server::serve`].
//!
//! It is the `bifurc` command line with another front: a tool is a command (`tools.rs`), a call of it is that
//! command's line (`invoke.rs`) run by the CLI's own code, against the engine the CLI would find or start. The
//! server holds no application logic and no state between calls.

pub mod invoke;
pub mod search;
pub mod server;
pub mod tools;

/// What `bifurc-mcp --help` prints.
pub const USAGE: &str = "Usage: bifurc-mcp [options]

The Bifurc MCP server: the commands of the bifurc command line as tools for an AI agent, over stdio.
It talks to the Bifurc engine that serves a data directory and starts one in the background if none is
running, as bifurc does. Register it with an MCP host as a stdio server whose command is `bifurc` with the single argument --mcp

The everyday tools are listed to the agent; it finds the others with search_tools and runs them with
call_tool. There is no tool that deletes, reads a secret, signs in or stops the engine.

Options
  --data-dir <path>    Default: the desktop app's data directory (or BIFURC_DATA_DIR)
  --no-start           Fail instead of starting an engine when none is running
  --all-tools          List every tool instead of the everyday ones and search_tools, for a host that
                       searches tools itself
  -V, --version
  -h, --help";
