# Architecture

Everything is Rust; there is no Node runtime.

```text
crates/
├── protocol/       wire types and command schemas shared by the engine and its clients
├── client/         synchronous engine client shared by the CLI and the MCP server
├── engine/         the engine: proxies, mocks, requests, workspaces, git
├── cli/            the CLI (library)
├── mcp/            the MCP server (library): the CLI's commands as tools
├── engine-client/  the desktop UI's async connection to the engine
├── app/            the desktop UI (GPUI): bifurc-ui
├── tray/           the tray helper the engine runs
└── bifurc/         the `bifurc` binary: the CLI, or the MCP server with `--mcp`
xtask/              `cargo xtask`: dev runs, licenses, packaging
packaging/          Store package, AppImage, deb and macOS templates
```

## One engine, many clients

The **engine** (`bifurc-engine`) owns all state and logic: the workspace files, git, the forward and reverse
proxies, mocks, requests, capture, TLS interception and updates. The desktop UI, the `bifurc` CLI and the MCP server
are clients. The UI holds no workspace logic: every change is an engine command, and the engine's events say when
to re-read.

A client finds a running engine through `<data-dir>/runtime.json`, which the engine writes when it starts. A client
that finds none starts one beside itself (`--no-start` fails instead). Set `BIFURC_DATA_DIR` to use a scratch data
directory, which is how tests and development runs stay apart from a real install.

## Transports

Clients speak the same commands over:

- a local **socket transport** (the desktop UI and the CLI), with a hello handshake that checks versions, then
  request/response messages and pushed events;
- **stdio**, for the MCP server, which turns the CLI's commands into tools.

The commands and their schemas are in `crates/protocol`; `crates/engine/src/register_all.rs` registers the handlers.

## The installed folder

Every package is the same folder in a different wrapper (`cargo xtask package`; see [building.md](building.md)):

```text
bifurc-ui                the interface
bifurc-engine            the engine
bifurc-tray              tray icon (Windows), menu-bar icon (macOS), StatusNotifierItem (Linux)
bin/bifurc               the `bifurc` command; the only folder on PATH
resources/install.json   names the wrapper, so the engine knows how this copy is updated
```

`crates/engine/src/updates/channel.rs` turns `install.json` into an update channel; a folder without it is a build,
not an install.

## Data and compatibility

Workspaces are plain files in git. A stored-data or settings change needs a migration in
`crates/engine/src/workspace/migrations.rs`, with a test. The minimum version that can upgrade straight to the
current one is 0.5.0.
