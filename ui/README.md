# Bifurc desktop UI (GPUI Kit)

A native GPUI app that runs the Bifurc engine as a **sidecar** and talks to it over the engine's socket
transport — the same transport, handshake and commands the `bifurc` CLI uses. The UI holds no workspace logic and
no entity state: every change is an engine command, and the engine's events say when to re-read.

```
ui/
├── crates/engine-client   socket framing, hello, request/response + events, sidecar attach-or-start (no GPUI)
└── crates/app             the GPUI app (bifurc-ui)
    ├── model/             typed views of engine data and the logic over them (no GPUI; unit-tested)
    ├── session/           one entity per open request: REST, WebSocket, GraphQL/gRPC/SOAP
    ├── panels/            dockable panels: explorer, request tab, response, details, activity
    └── shell/             window: title bar, module rail, dock area, status bar, dialogs, command palette
```

## Run

Prerequisites: Rust (stable, 2024 edition), Node.js, and a built engine.

```sh
npm install            # once, at the repository root
npm run build:packages # builds the engine the UI starts
cd ui
cargo run -p bifurc-ui
```

On start the UI reads `<data dir>/runtime.json`. If an engine is already running for that data directory (for
example one started with `bifurc engine start`) it **joins** it and leaves it running on exit. Otherwise it starts
`node packages/engine/dist/runtime/hostCli.js --transport socket` and stops it when the window closes. Either
way the CLI and the UI see the same workspace live: a request created with `bifurc call entity.create …` shows up
in the explorer without a refresh.

| Variable               | Meaning                                                                                 |
| ---------------------- | --------------------------------------------------------------------------------------- |
| `BIFURC_DATA_DIR`      | Data directory (default: the desktop app's, e.g. `%LOCALAPPDATA%\Bifurc` on Windows)    |
| `BIFURC_NODE`          | The `node` executable to start the engine with (default: `node` on `PATH`)              |
| `BIFURC_ENGINE_ENTRY`  | The engine's `hostCli.js` (default: found above the executable or the working directory) |

## Using it

- **Rail** (left): Requests, Environments, Mappings. It switches what the explorer dock shows.
- **Docks**: explorer on the left, details on the right, activity at the bottom; the editor and response panels
  are in the centre. Drag a tab to split or move it; each dock folds away (`Ctrl+B`, `Ctrl+Shift+B`, `Ctrl+J`).
- **Requests** are one tree. REST, GraphQL, gRPC, SOAP and **WebSocket** requests live in their own folders and
  open as tabs. A WebSocket is just another request type: Save, Revert and the tree behave the same, and its
  stream shows in the Response panel. The connection belongs to the engine, so it survives closing the tab.
- **Command palette**: `Ctrl+K` / `Ctrl+Shift+P`. Commands show the keys they are bound to; requests and
  environments are searchable. `Ctrl+N` new request, `Ctrl+Shift+N` new WebSocket, `Ctrl+S` save,
  `Ctrl+Enter` send (or connect), `F5` reload.
- A tab with unsaved edits is marked and cannot be closed until you save or revert.

## What is not here yet

- Editing GraphQL, gRPC and SOAP requests (they open, show their stored fields and send by id).
- Editing environment variables, mock rules, proxy rules, services, settings, sync/git, import/export.
- Saving the dock layout between runs.
- A packaged build that bundles Node and the engine.
- The engine is not stopped if the UI is killed rather than closed; `bifurc engine stop` stops it.

## Tests

```sh
cargo test --workspace   # frame codec, connection, sidecar discovery, config/explorer/response/socket models
cargo clippy --workspace --all-targets
```

`cargo run -p bifurc-engine-client --example probe -- <data-dir>` starts or joins an engine for a data
directory and prints what it says: a quick check of the real wire.
