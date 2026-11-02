# MCP server

`bifurc --mcp` is the same commands as the CLI, as tools for an AI agent, over stdio. `--mcp` must be the first argument. It talks to the engine that serves the data directory and starts one if none is running.

Register it with an MCP host as a stdio server whose command is `bifurc` with the argument `--mcp`:

```json
{ "mcpServers": { "bifurc": { "command": "bifurc", "args": ["--mcp"] } } }
```

The everyday tools (send a request, list requests, mocks and mappings, create a mock or a mapping) are listed to the agent. It finds the others with `search_tools`, reads one with `describe_tool` and runs it with `call_tool`. There is no tool that deletes anything, reads a secret, signs in or stops the engine. Vault tools are in beta.

```text
Usage: bifurc --mcp [options]

The Bifurc MCP server: the commands of the bifurc command line as tools for an AI agent, over stdio.
It talks to the Bifurc engine that serves a data directory and starts one in the background if none is
running, as bifurc does. Register it with an MCP host as a stdio server whose command is `bifurc` with the argument --mcp

The everyday tools are listed to the agent; it finds the others with search_tools and runs them with
call_tool. There is no tool that deletes, reads a secret, signs in or stops the engine.

Options
  --session <name>     Work in a named session: its own cookies, variables and environment (or BIFURC_SESSION)
  --data-dir <path>    Default: the desktop app's data directory (or BIFURC_DATA_DIR)
  --no-start           Fail instead of starting an engine when none is running
  --all-tools          List every tool instead of the everyday ones and search_tools, for a host that
                       searches tools itself
  -V, --version
  -h, --help
```
