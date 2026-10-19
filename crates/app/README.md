# Bifurc desktop UI (GPUI Kit)

A native GPUI app that runs the Bifurc engine as a **sidecar** and talks to it over the engine's socket
transport — the same transport, handshake and commands the `bifurc` CLI uses. The UI holds no workspace logic and
no entity state: every change is an engine command, and the engine's events say when to re-read.

```
ui/
├── crates/engine-client   socket framing, hello, request/response + events, sidecar attach-or-start (no GPUI)
├── crates/tray            the engine's tray icon (bifurc-tray): a helper the engine runs, no GPUI
├── crates/bifurc          the `bifurc` command: the Rust CLI (crates/cli), or the MCP server (crates/mcp) with `--mcp`
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

Prerequisites: Rust (stable, 2024 edition). The engine and the CLI are Rust too (`crates/engine`,
`crates/cli`); all of them are members of the one cargo workspace at the repository root.

```sh
cargo dev:ui           # from the repository root: builds the engine and tray helper, runs the UI (joins a running engine or starts one)
# the engine and the CLI it starts come from the repository workspace:
cargo build -p bifurc-engine -p bifurc     # at the repository root
```

On start the UI reads `<data dir>/runtime.json`. If an engine is already running for that data directory (for
example one started with `bifurc engine start`) it **joins** it, whichever engine that is. Otherwise it starts
`bifurc-engine --transport socket`, detached: the one beside its executable (an install), else the newest build
in a `target/release` or `target/debug` above the executable or the working directory. Either way the CLI and the UI see
the same workspace live: a request created with `bifurc call entity.create …` shows up in the request tree
without a refresh.

The engine's lifetime is its own, whichever client started it:

- The engine shows a **tray icon** (`crates/tray`, the `bifurc-tray` helper it runs as a child; building the app builds it too). Closing the
  window closes this app only; the engine keeps running behind the icon. The icon opens the app again (or
  brings an open one to the front), restarts the engine and quits.
- **Quit** in the tray, or `bifurc engine stop`, stops the engine and every interface attached to it closes.
- A **restart** (the tray, Settings → Servers, or `bifurc engine restart`) starts a new engine with the same
  command line; attached interfaces wait for it and reconnect.
- Only with no tray icon at all (the platform has none, or on Linux no StatusNotifierWatcher is running, as on
  stock GNOME without the AppIndicator extension) does the engine stop when its last interface
  closes, so it is never left running out of reach.

| Variable               | Meaning                                                                                 |
| ---------------------- | --------------------------------------------------------------------------------------- |
| `BIFURC_DATA_DIR`      | Data directory (default: the desktop app's, e.g. `%LOCALAPPDATA%\Bifurc` on Windows)    |
| `BIFURC_ENGINE`        | The `bifurc-engine` executable to start (default: beside this executable, else the newest `target/` build above it) |
| `BIFURC_TRAY`          | The tray helper (default: `bifurc-tray` beside this executable, else the engine's search) |
| `BIFURC_UI`            | The interface the tray icon opens (default: this executable)                            |

On Windows the executable is linked with an 8 MB main-thread stack (`crates/app/build.rs`). Windows gives the
main thread 1 MB, and GPUI lays out and paints by recursion, so a deep view tree overflows it in a debug build.
The same build script compiles `build/icon.ico` into the executable, which is the icon the taskbar shows.

## Package

```sh
cargo xtask package                        # everything this platform has: packages and store manifests
cargo xtask package --target nsis          # one or more of the targets below, or dir
cargo xtask package --skip-build           # package what is already built
cargo xtask package --release-tag v1.2.3   # the GitHub release the manifests download from (default: v<version>)
```

`xtask` (at the repository root) builds the engine and the release executables, lays out one folder
(`release/desktop/<platform>-unpacked`) and wraps it. Every package installs the same things: the interface, the
Rust engine, the tray helper (Windows), and the `bifurc` command. No Node is shipped.

```
bifurc-ui[.exe]      the interface
bifurc-engine[.exe]  the engine (crates/engine)
bifurc-tray.exe      the engine's tray icon (Windows)
bin/bifurc[.exe]     the bifurc command (crates/bifurc): the CLI, or the MCP server with --mcp; finds the engine above its bin/
```

| Package | File | `bifurc` on `PATH` |
| ------- | ---- | ------------------ |
| Installer (NSIS, `packaging/windows/bifurc.nsi`) | `Bifurc-<version>-setup.exe` | Adds `<install>\bin` to the user's `PATH`; removed on uninstall. Per-user, no elevation, installs to `%LOCALAPPDATA%\Programs\bifurc-desktop`. |
| Store (`packaging/windows/AppxManifest.xml`) | `Bifurc-<version>.msix` | An app execution alias, `bifurc.exe`. Unsigned: the Store signs it; sign it yourself to sideload. |
| Zip (`zip`) | `Bifurc-<version>-win-x64.zip` | The staged folder as it is; nothing is put on `PATH`. What Scoop installs. |
| AppImage (`packaging/linux/AppRun`) | `Bifurc-<version>-x86_64.AppImage` | The first run links `~/.local/bin/bifurc`; `Bifurc.AppImage cli …` works too. |
| Debian package (`packaging/linux/deb`) | `bifurc_<version>_amd64.deb` | `/usr/bin/bifurc`, a link into `/usr/lib/bifurc`. Its `postinst` gives the engine `cap_net_bind_service`, so the reverse proxy can listen on port 80 without root. |
| Tarball (`tarball`) | `Bifurc-<version>-linux-x86_64.tar.gz` | The staged folder as `bifurc-<version>/`, with a launcher entry. What the AUR package installs. |

The store manifests are written to `release/desktop/manifests`, each naming the file it downloads from the GitHub
release (`--release-tag`, in `GITHUB_REPOSITORY`) and that file's SHA-256:

| Manifest | Target | Files | Publishing it |
| -------- | ------ | ----- | ------------- |
| winget | `winget` | `winget/manifests/h/HarshalKudale/Bifurc/<version>/*.yaml` | A pull request to `microsoft/winget-pkgs` with that folder. |
| Scoop | `scoop` | `scoop/bifurc.json` | A file of a bucket; `scoop install <url of the file>` works without one. |
| AUR | `aur` | `aur/PKGBUILD`, `.SRCINFO`, `bifurc.install` | A commit to the `bifurc-bin` AUR repository. |

A manifest's target builds the file it describes too (`winget` the installer, `scoop` the zip, `aur` the
tarball). None of them is published by a workflow: a release attaches them, and submitting one is done by hand.

The AppImage does not run the app from inside the image: the engine outlives the window, and the image's files are
only there while the process it launched runs. Its first run copies the app to `~/.local/share/bifurc/app/<build>`
and adds a launcher entry; everything runs from that copy.

The installer needs NSIS (`makensis` on `PATH`, in `Program Files`, or `MAKENSIS`), the Store package needs the
Windows SDK (`makeappx`, `makepri`), the zip Windows' own `tar.exe`, the `.deb` `dpkg-deb` (and `dpkg-shlibdeps`,
which reads its dependencies from the executables). `.github/workflows/package-desktop.yml` runs
`cargo xtask package` once per platform and builds all of it; `release.yml` calls that workflow and publishes
what it built.

## Using it

- **Window**: a title bar, a left sidebar, a top bar, the screen in the middle, an optional right sidebar, and a
  footer.
  - The title bar has search, which opens the command palette, and the environment selector. Variables resolve
    against the selected environment over the global one.
  - The left sidebar has the workspace switcher at the top (switch, create, rename, delete), the screens in
    groups, and Settings at the foot. Applications: Services, Health bar. Routing: Mappings, Proxy rules,
    Capture. API: Requests, Mocks, Flows. Workspace: Environments, Vaults, Config, Audit log. It collapses to icons
    (`Ctrl+B`).
  - The top bar has the two sidebar triggers and the breadcrumb: workspace, screen, the folders the open
    request or mock is filed under, and what is open.
  - The right sidebar (`Ctrl+Shift+B`) is reference only: **History** (the commits that touched the open
    entity's file), **Snippet** (the open request as cURL, `fetch`, axios or Python) and **Variables** (what
    `{{name}}` resolves against).
  - The footer shows the workspace's git state with a Publish/Sync button for the current screen's changes, the
    current screen's status, the forward proxy (select it to start or stop it) and the engine connection.
- **Screens** are kept alive while another one is in front, so coming back to Requests finds its tabs as they were.
  Switch with the sidebar or `Ctrl+1` to `Ctrl+9` (Services, Health bar, Mappings, Proxy rules, Capture,
  Requests, Mocks, Flows, Environments); `Ctrl+,` opens Settings. Vaults, Config and Audit log have no key.
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
    the engine, so it goes on if the tab is closed and is picked up again when the tab is reopened. **Export
    report** saves a finished run as an HTML page.
  - **Mappings**, **Services** and **Health bar** are lists. **Proxy rules** and **Environments** are a list with
    a detail pane.
  - **Capture** works like a browser's network tab. The list shows each exchange's name, method, status, type,
    how the proxy handled it, size and time. Selecting one fetches it from the engine and shows its headers,
    its payload (query parameters and request body) and its response; an image is shown as an image, and other
    data can be saved to a file. **Save as request** and **Save as mock** create the entity and open it.
    **Pause** stops the engine recording for this workspace until **Resume**: what passes meanwhile is not kept.
  - **Settings**: Appearance (theme, language, text size; kept by this client only), Accounts, Servers,
    Requests (the defaults every request starts from: certificate checks, redirects, timeout), HTTPS
    (interception and the root certificate: generate, import, install, remove) and About (versions, whether a
    newer one exists and how this install gets it).
  - **Config** is the workspace's own settings: its name, publishing it to a git remote and how syncing goes,
    export and import, cookies, and deleting it.
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
- **Response history**: a saved request keeps its last five responses on this computer, outside the workspace's
  git folder. **History** in the response pane shows an earlier one and goes back to the latest.
- **Command palette**: `Ctrl+K` / `Ctrl+Shift+P`. It opens any saved request or mock in a tab, switches the
  environment, and lists every command: new request of each type, new flow, folder, mock, mapping, proxy rule,
  health check and environment, the cURL imports, capture and forward-proxy controls, Save, Sync, Revert,
  Delete, Duplicate, Rename and Close for the open item, tab steps, zoom and Quit. Commands show the keys they
  are bound to. **Finder** (`Ctrl+P`) searches requests, mocks, mappings and environments by name; `Ctrl+P` in
  the palette switches to the finder and `Ctrl+K` in the finder switches back.

`Ctrl` is `Cmd` on macOS.

| Key                          | Command                                          |
| ---------------------------- | ------------------------------------------------ |
| `Ctrl+K`, `Ctrl+Shift+P`     | Command palette                                  |
| `Ctrl+P`                     | Finder                                           |
| `Ctrl+N`, `Ctrl+T`           | New REST request                                 |
| `Alt+1` to `Alt+9`           | New REST, GraphQL, gRPC, SOAP, WebSocket, SSE, Socket.IO or MCP request; `Alt+9` imports cURL (`Cmd+Option+1`–`9` on macOS) |
| `Ctrl+Shift+N`               | New WebSocket request                            |
| `Ctrl+Shift+M`               | New mock                                         |
| `Ctrl+Shift+G`               | New flow                                         |
| `Ctrl+S`                     | Save (creates a draft; writes now if saved)      |
| `Ctrl+Enter`                 | Send, or connect                                 |
| `Esc`                        | Cancel the request being sent                    |
| `Ctrl+W`                     | Close the open tab                               |
| `Ctrl+Shift+T`               | Reopen the last closed tab                       |
| `Ctrl+D`                     | Duplicate the open item (on the flow canvas: the selected node) |
| `F2`                         | Rename the open item                             |
| `Ctrl+L`                     | Go to the open request's address                 |
| `Ctrl+PageDown`, `Ctrl+PageUp` | Next or previous tab                           |
| `Ctrl+Tab`, `Ctrl+Shift+Tab` | Next or previous item (tabs, or the list on Proxy rules and Flows) |
| `Ctrl+B`                     | Show or hide the left sidebar                    |
| `Ctrl+Shift+B`               | Show or hide the right sidebar                   |
| `Ctrl+Shift+E`               | Show or hide the folder tree                     |
| `Ctrl+1` to `9`              | Go to a screen (see above)                       |
| `Ctrl+=`, `Ctrl+-`, `Ctrl+0` | Larger text, smaller text, default size          |
| `Ctrl+,`                     | Settings                                         |
| `F5`                         | Reload the workspace                             |
| `Ctrl+Q`                     | Quit                                             |

The keys that act on the open item do nothing while a dialog or the palette is open.

Everything else the palette lists has a command but no key.

## What is not here yet

- A schema explorer for gRPC (GraphQL has its schema, SOAP its WSDL).
- The runner keeps no history of past runs and runs REST folders only.
- Installing an update: the app says that a newer version exists and how this install gets it (Settings →
  About, and the footer), and leaves the installing to the Store, the package manager or the downloaded file.
- The engine is not stopped if the UI is killed rather than closed; `bifurc engine stop` stops it.

## Tests

```sh
cargo test --workspace   # frame codec, connection, sidecar discovery, config/explorer/response/socket models
cargo clippy --workspace --all-targets
```

`cargo run -p bifurc-engine-client --example probe -- <data-dir>` starts or joins an engine for a data
directory and prints what it says: a quick check of the real wire.
