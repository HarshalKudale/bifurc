# Bifurc
[![Website](https://img.shields.io/badge/Website-Bifurc-4F46E5?logo=googlechrome&logoColor=white)](https://bifurc.app/)
[![Tests](https://github.com/HarshalKudale/bifurc/actions/workflows/test.yml/badge.svg)](https://github.com/HarshalKudale/bifurc/actions/workflows/test.yml)
[![Latest Release](https://img.shields.io/github/v/release/HarshalKudale/bifurc?display_name=tag&sort=semver)](https://github.com/HarshalKudale/bifurc/releases/latest)
[![GitHub Downloads](https://img.shields.io/github/downloads/HarshalKudale/bifurc/total)](https://github.com/HarshalKudale/bifurc/releases)
[![License](https://img.shields.io/badge/license-PolyForm%20Noncommercial%201.0.0-blue)](LICENSE)
[![GitHub stars](https://img.shields.io/github/stars/HarshalKudale/bifurc?style=flat)](https://github.com/HarshalKudale/bifurc/stargazers)

**Bifurc is a desktop API development workspace for localhost routing, capture, mocking, protocol testing, and git-backed configuration.**

It brings together `*.localhost` mappings, proxy rules, capture tooling, REST/GraphQL/SOAP/gRPC request authoring, browser-assisted workflows, TLS interception, and workspace history in one developer-focused app.

## Main capabilities

- map friendly `*.localhost` domains to local services
- intercept, inspect, and replay HTTP traffic
- create mocks for REST, GraphQL, SOAP, and gRPC workflows
- save and organize authored requests across protocols
- use a browser companion extension for DevTools-driven capture and mock creation
- version workspace data with git and sync it between collaborators

## Engine, CLI, MCP server and desktop UI

The engine (`crates/engine`), the `bifurc` command (`crates/bifurc`, over the CLI in `crates/cli` and the MCP server
in `crates/mcp`), the shared wire types and client
(`crates/protocol`, `crates/client`) and the desktop UI (`crates/app`, `crates/tray`) are all Rust. There is no Node runtime.

```bash
cargo dev:engine     # the engine, in this terminal (logs here; Ctrl+C or `cargo dev:cli engine stop` ends it)
cargo dev:ui         # the UI; attaches to a running engine, or starts one from target/ if none is
cargo dev:cli <cmd>  # the `bifurc` CLI, e.g. `cargo dev:cli status`
cargo dev:mcp        # `bifurc --mcp`, the MCP server, on this terminal's stdin and stdout
cargo xtask package  # installer / Store package / AppImage / macOS .dmg
```

All of them find each other through `<data-dir>/runtime.json`; set `BIFURC_DATA_DIR` to use a scratch directory.
On Windows a running engine locks `target/debug/bifurc-engine.exe`, so stop it before rebuilding it.

### MCP server

`bifurc --mcp` gives an AI agent the commands of the `bifurc` CLI as tools, over stdio. `--mcp` must be the first
argument. It talks to the same engine and starts one if none is running. Register it with an MCP host as a stdio
server whose command is `bifurc` with the argument `--mcp`, for example:

```json
{ "mcpServers": { "bifurc": { "command": "bifurc", "args": ["--mcp"] } } }
```

The everyday tools (send a request, list requests, mocks and mappings, create a mock or a mapping) are listed to
the agent; it finds the others with `search_tools`, reads one with `describe_tool` and runs it with `call_tool`.
After `--mcp` the server's own options may follow (`cargo dev:mcp --all-tools` lists every tool instead, for a host that searches tools itself; `--help` lists the rest). Like the CLI it cannot
delete anything, read a secret, sign in or stop the engine.


## Repository structure

```text
Bifurc/
├── crates/
│   ├── protocol/       # Wire types and command schemas shared by the engine and its clients
│   ├── client/         # Synchronous engine client shared by the CLI and MCP server
│   ├── engine/         # The engine: proxies, mocks, requests, workspaces, git
│   ├── cli/            # The CLI (library)
│   ├── mcp/            # The MCP server (library): the CLI's commands as tools
│   ├── engine-client/  # The desktop UI's async connection to the engine
│   ├── app/            # Desktop UI (GPUI)
│   ├── tray/           # Tray helper the engine runs
│   └── bifurc/         # The `bifurc` binary: the CLI, or the MCP server with `--mcp`
├── xtask/              # `cargo xtask package`
├── packaging/          # Installer, Store package and AppImage templates
└── build/              # Icons
```

The browser companion extension (`bifurc-extension`) and the published site (`bifurc-website`) are separate repositories.

## Documentation

The documentation source now lives in the **website repository only** and is rendered from markdown files in `bifurc-website\content\docs`.

- Website repo: `bifurc-website\`
- Docs source: `bifurc-website\content\docs\`
- Website routes: `/docs/*`

## Quick start

```bash
git clone https://github.com/HarshalKudale/bifurc.git
cargo dev:engine   # then, in another terminal:
cargo dev:ui
```

## Browser extension

Load the `bifurc-extension` repository's folder unpacked in Chrome or Edge to:

- toggle browser proxy routing
- review traffic inside a Bifurc DevTools tab
- create saved requests and mocks from live browser traffic

## Testing

```bash
cargo test                # protocol, engine, CLI, MCP server (the default members)
cargo test --workspace    # and the UI crates, which need GPUI's build dependencies
```

## Licensing

Bifurc is free to use for personal, non-commercial purposes under the [PolyForm Noncommercial License 1.0.0](LICENSE).

You may not resell it, repackage it, or use it to build commercial products or generate profit.

See [LICENSE](LICENSE) for the full terms. Questions: contact@bifurc.app.
