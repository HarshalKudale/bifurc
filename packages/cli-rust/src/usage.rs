//! The text of `bifurc --help`.

/// The usage text, printed for `--help`, `help` and no command, and appended to an unknown-command error.
pub const USAGE: &str = "Usage: bifurc <command> [options]

Talks to the Bifurc engine that serves a data directory, over its socket (a named pipe on Windows).
The engine is found through <data-dir>/runtime.json. A command that needs the engine starts one in the
background if none is running; it keeps running with a tray icon after this terminal closes.

Engine
  engine start [--transport socket|ws] [--no-tray]   Start a standalone engine in the background
  engine stop                            Stop it gracefully; the interfaces using it close too
  engine restart                         Stop it and start it again; the interfaces reconnect
  engine status                          Where it runs, without opening a session
  status                                 Engine, proxy and active workspace

Workspaces
  workspaces list
  workspaces use <id>

Environments (Global is always applied; the selected one overrides it; one extra env is auto-selected)
  env list
  env use <id|name>
  env show                               The merged variables a request resolves against

Mappings (<sub>.localhost → target)
  mappings list
  mappings add <sub|domain> <target> [--label <text>]
  mappings remove <id>
  mappings enable|disable <id>

Services
  services list                          Local processes listening on a port

Requests
  requests list
  requests show <id>
  requests add --url <url> [--method GET] [--name <n>] [-H \"k: v\"]... [--body <text> | --body-file <f>]
  requests send <id>                     Send a saved request by id (engine resolves and runs its scripts)
  requests send --url <url> [--method] [-H \"k: v\"]... [--body …]   Send an unsaved request

Anything else
  call <command> [<json-payload>]        Any engine command, e.g. call server.status
  commands                               List every engine command

Options
  --data-dir <path>    Default: the desktop app's data directory (or BIFURC_DATA_DIR)
  -w, --workspace <id> Default: the active workspace
  --no-start           Fail instead of starting an engine when none is running
  --json               Machine-readable output
  -h, --help";
