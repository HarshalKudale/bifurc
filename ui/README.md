# Bifurc desktop UI (GPUI Kit)

A native GPUI app that runs the Bifurc engine as a **sidecar** and talks to it over the engine's socket
transport — the same transport, handshake and commands the `bifurc` CLI uses. The UI holds no workspace logic and
no entity state: every change is an engine command, and the engine's events say when to re-read.

```
ui/
├── crates/engine-client   socket framing, hello, request/response + events, sidecar attach-or-start (no GPUI)
├── crates/tray            the engine's tray icon (bifurc-tray): a helper the engine runs, no GPUI
└── crates/app             the GPUI app (bifurc-ui); build.rs gives the Windows main thread a larger stack
    └── src/
        ├── model/         typed views of engine data and the logic over them (no GPUI; unit-tested)
        ├── session/       one entity per open request: REST, GraphQL/gRPC/SOAP, WebSocket
        ├── screens/       one view per screen: requests, mocks, mappings, proxy rules, capture, services,
        │                  health bar, environments, settings
        ├── components/    pieces the screens share: entity footer, autosave, dialogs, key/value table,
        │                  variable-aware editor, page furniture
        ├── panels/        the dock panel that follows a screen's active tab
        └── shell/         window: title bar, left sidebar, top bar, right sidebar, footer, command palette
```

Beside those folders, `app_model.rs` holds the workspace as the engine reports it, `commands.rs` the commands and
their keys, `preferences.rs` and `theme.rs` this client's own appearance settings, `engine.rs` the handle on the
engine, and `bootstrap.rs` the root view that is on screen before the engine is.

## Run

Prerequisites: Rust (stable, 2024 edition), Node.js, and a built engine.

```sh
npm install            # once, at the repository root
npm run build:packages # builds the engine the UI starts
cd ui
cargo run -p bifurc-ui
```

On start the UI reads `<data dir>/runtime.json`. If an engine is already running for that data directory (for
example one started with `bifurc engine start`) it **joins** it. Otherwise it starts
`node packages/engine/dist/runtime/hostCli.js --transport socket`, detached. Either way the CLI and the UI see
the same workspace live: a request created with `bifurc call entity.create …` shows up in the request tree
without a refresh.

The engine's lifetime is its own, whichever client started it:

- The engine shows a **tray icon** (`crates/tray`, the `bifurc-tray` helper it runs as a child; building the app builds it too). Closing the
  window closes this app only; the engine keeps running behind the icon. The icon opens the app again (or
  brings an open one to the front), restarts the engine and quits.
- **Quit** in the tray, or `bifurc engine stop`, stops the engine and every interface attached to it closes.
- A **restart** (the tray, Settings → Servers, or `bifurc engine restart`) starts a new engine with the same
  command line; attached interfaces wait for it and reconnect.
- Only with no tray icon at all (the platform has none yet) does the engine stop when its last interface
  closes, so it is never left running out of reach.

| Variable               | Meaning                                                                                 |
| ---------------------- | --------------------------------------------------------------------------------------- |
| `BIFURC_DATA_DIR`      | Data directory (default: the desktop app's, e.g. `%LOCALAPPDATA%\Bifurc` on Windows)    |
| `BIFURC_NODE`          | The `node` executable to start the engine with (default: `node` on `PATH`)              |
| `BIFURC_ENGINE_ENTRY`  | The engine's `hostCli.js` (default: found above the executable or the working directory) |
| `BIFURC_TRAY`          | The tray helper (default: `bifurc-tray` beside this executable, else the engine's search) |
| `BIFURC_UI`            | The interface the tray icon opens (default: this executable)                            |

On Windows the executable is linked with an 8 MB main-thread stack (`crates/app/build.rs`). Windows gives the
main thread 1 MB, and GPUI lays out and paints by recursion, so a deep view tree overflows it in a debug build.

## Using it

- **Window**: a title bar, a left sidebar, a top bar, the screen in the middle, an optional right sidebar, and a
  footer.
  - The title bar has search, which opens the command palette, and the environment selector. Variables resolve
    against the selected environment over the global one.
  - The left sidebar has the workspace switcher at the top (switch, create, rename, delete), the screens in
    groups, and Settings at the foot. Applications: Services, Health bar. Routing: Mappings, Proxy rules,
    Capture. API: Requests, Mocks. Workspace: Environments. It collapses to icons (`Ctrl+B`).
  - The top bar has the two sidebar triggers and the breadcrumb: workspace, screen, the folders the open
    request or mock is filed under, and what is open.
  - The right sidebar (`Ctrl+Shift+B`) is reference only: **History** (the commits that touched the open
    entity's file), **Snippet** (the open request as cURL, `fetch`, axios or Python) and **Variables** (what
    `{{name}}` resolves against).
  - The footer shows the workspace's git state with a Publish/Sync button for the current screen's changes, the
    current screen's status, the forward proxy (select it to start or stop it) and the engine connection.
- **Screens** are kept alive while another one is in front, so coming back to Requests finds its tabs as they were.
  Switch with the sidebar or `Ctrl+1` to `Ctrl+8` (Services, Health bar, Mappings, Proxy rules, Capture,
  Requests, Mocks, Environments); `Ctrl+,` opens Settings.
  - **Requests** and **Mocks** have a folder tree (`Ctrl+Shift+E` shows or hides it), the open items as tabs, and
    the active item's editor and response as docks you can drag to split or move. A tab's right-click menu
    closes it, the others, the ones to its right or all of them, and duplicates what it shows; the mouse wheel
    scrolls the strip, and the arrow at its end lists every open tab.
  - **Duplicate** (a request's or a mock's right-click menu in the tree, or its tab's) asks the engine for a
    copy named "… copy" in the same folder and opens it. A mock's copy starts switched off, so the original
    keeps answering.
  - **Runner**: a folder of REST requests opens as a tab from its right-click menu (**Open as runner**). The tab
    lists the folder's requests in run order, which the arrows change; **Run folder** has the engine send them
    one after another and shows each one's status, time and test results as they arrive. The run belongs to
    the engine, so it goes on if the tab is closed and is picked up again when the tab is reopened.
  - **Mappings**, **Services** and **Health bar** are lists. **Proxy rules** and **Environments** are a list with
    a detail pane.
  - **Capture** works like a browser's network tab. The list shows each exchange's name, method, status, type,
    how the proxy handled it, size and time. Selecting one fetches it from the engine and shows its headers,
    its payload (query parameters and request body) and its response; an image is shown as an image, and other
    data can be saved to a file. **Save as request** and **Save as mock** create the entity and open it.
    **Pause** stops the engine recording for this workspace until **Resume**: what passes meanwhile is not kept.
  - **Settings**: Appearance (theme, text size; kept by this client only), Proxy, HTTPS (interception and the
    root certificate: generate, remove) and About (versions, and a link to the releases page).
- **Requests** are one tree. REST, GraphQL, gRPC, SOAP and **WebSocket** requests live in their own folders,
  are created from the command palette or the keys below, and are edited the same way as REST ones. A WebSocket
  is just another request type: its stream shows in the Response panel, and the connection belongs to the
  engine, so it survives closing the tab.
- **Bodies**: a REST request's body is none, JSON, text, XML, HTML, form fields, **multipart** (fields and
  files) or one **file** sent as it is. A response that is an image is previewed, and any response that is not
  text can be saved to a file.
- **A mock's body follows its response type.** The type is the mock's `Content-Type` header, which a mock made
  from captured traffic takes from the real server. A text type is edited as text; an image or other data type
  is answered with a file, and a file of another type is refused.
- **Save model**: a new request, mock, rule or environment is a **draft**, and Save creates it. A saved one has
  no Save step: its edits are written a moment after they are made, or at once when you leave it. Closing a
  draft you typed into asks first; a saved item is simply closed.
- **Name and type**: the open entity's name is edited in the breadcrumb. For a request the breadcrumb also
  shows its type. A draft's type can be changed there and keeps what the types share (name, address, headers,
  body); a saved request's type is fixed.
- **Footer of an open entity**: every open tab (Requests, Mocks) and every detail pane (Proxy rules,
  Environments) ends in the same bar with Save, Sync, Revert and Delete, and Close for tabs.
  - **Save** creates a draft.
  - **Sync** commits the entity's file, and pushes it if the workspace has a remote.
  - **Revert** puts the entity's file back to the last synced version.
  - **Delete** removes the entity.
  - For a request or a mock the bar also shows its **folder**. Choosing another one moves it: a saved item
    with its next write, a draft when it is created.
- **Command palette**: `Ctrl+K` / `Ctrl+Shift+P`. It opens any saved request or mock in a tab, switches the
  environment, and lists every screen's actions: new request of each type, new folder, new mock, new mapping,
  new proxy rule, new health check, new environment, pause or clear capture, start or stop the forward proxy,
  and Save, Sync, Revert, Delete and Close for the open item. Commands show the keys they are bound to.

`Ctrl` is `Cmd` on macOS.

| Key                | Command                                   |
| ------------------ | ----------------------------------------- |
| `Ctrl+K`, `Ctrl+Shift+P` | Command palette                     |
| `Ctrl+N`, `Ctrl+T` | New REST request                          |
| `Ctrl+Shift+N`     | New WebSocket request                     |
| `Ctrl+Shift+M`     | New mock                                  |
| `Ctrl+S`           | Save (creates a draft; writes now if saved) |
| `Ctrl+Enter`       | Send, or connect                          |
| `Ctrl+W`           | Close the open tab                        |
| `Ctrl+B`           | Show or hide the left sidebar             |
| `Ctrl+Shift+B`     | Show or hide the right sidebar            |
| `Ctrl+Shift+E`     | Show or hide the folder tree              |
| `Ctrl+1` to `8`    | Go to a screen (see above)                |
| `Ctrl+,`           | Settings                                  |
| `F5`               | Reload the workspace                      |
| `Ctrl+Q`           | Quit                                      |

Everything else the palette lists has a command but no key.

## What is not here yet

- Import and export.
- Setting up a git remote (the UI only uses one that exists), and certificate export or import (Settings can
  generate and remove the root certificate).
- The audit log. The runner has no history of past runs, no report export, and runs REST folders only.
- Schema explorers for GraphQL, gRPC and SOAP.
- An updater: Settings only links to the releases page.
- Saving the dock layout between runs (the sidebar state and the theme are kept).
- A packaged build that bundles Node and the engine.
- The engine is not stopped if the UI is killed rather than closed; `bifurc engine stop` stops it.

## Tests

```sh
cargo test --workspace   # frame codec, connection, sidecar discovery, config/explorer/response/socket models
cargo clippy --workspace --all-targets
```

`cargo run -p bifurc-engine-client --example probe -- <data-dir>` starts or joins an engine for a data
directory and prints what it says: a quick check of the real wire.
