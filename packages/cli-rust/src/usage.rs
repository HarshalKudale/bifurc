//! The text of `bifurc --help` and of each command group's `bifurc <group> --help`.

/// The usage text, printed for `--help`, `help` and no command, and appended to an unknown-command error.
pub const USAGE: &str = "Usage: bifurc <group> <verb> [arguments] [options]

Talks to the Bifurc engine that serves a data directory, over its socket (a named pipe on Windows).
The engine is found through <data-dir>/runtime.json. A command that needs the engine starts one in the
background if none is running; it keeps running with a tray icon after this terminal closes.

Every group of saved items has the same verbs: list, show, create, update, and enable|disable or the verb
that runs it (send, run) where that applies. Deleting, syncing one item, reverting and scripts are done in
the desktop app. An item is named by its id or by its name.

Groups (bifurc <group> --help shows the arguments and options of each)
  workspaces    list | show | create | update | use | sync
  env           list | show | use
  mappings      list | show | create | update | enable | disable      <sub>.localhost → target
  rules         list | show | create | update | enable | disable      Forward-proxy rules
  mocks         list | show | create | update | enable | disable      Mocked responses
  requests      list | show | create | update | send | reflect        REST, GraphQL, gRPC, SOAP, WebSocket, SSE, Socket.IO, MCP
  health        list | show | create | update | run                   Health checks
  flows         list | show | run                                    Saved chains of requests
  vault         list | create | set | unseal | seal | run            Encrypted secrets
  account       list | login | repos                                  GitHub sign-in
  services      list                                                  Local processes listening on a port

Engine
  engine start [--transport socket|ws] [--no-tray]   Start a standalone engine in the background
  engine stop                            Stop it gracefully; the interfaces using it close too
  engine restart                         Stop it and start it again; the interfaces reconnect
  engine status                          Where it runs, without opening a session
  status                                 Engine, proxy and active workspace
  update [check]                         Whether a newer Bifurc exists and how this install gets it; check asks now

Anything else
  call <command> [<json-payload>]        Any engine command, e.g. call server.status
  commands                               List every engine command
  --mcp                                  Alone, no other argument: serve these commands as tools for an AI agent (MCP, stdio)
  help [<group>]

Options
  --data-dir <path>    Default: the desktop app's data directory (or BIFURC_DATA_DIR)
  -w, --workspace <id> Default: the active workspace
  --no-start           Fail instead of starting an engine when none is running
  --json               Machine-readable output
  -h, --help

Exit code: 0 when the command succeeded, 1 when it failed or what it ran failed (a request's failed test,
a failed gRPC call, a health check that does not answer).";

const WORKSPACES: &str = "Usage: bifurc workspaces <verb>

  list
  show [<id|name>]                       The active workspace when none is named
  create <name> [--account <login>]      A new local workspace, made active; with --account that account owns it
  update <id|name> --name <new name>
  use <id|name>                          Make it the active workspace
  sync                                   Sync everything that was never synced, one commit per item, then push
                                         when there is a remote. Deletions are left to the desktop app, where
                                         each one is confirmed or reverted

Publishing to a new repository, importing one and deleting a workspace are done in the desktop app.";

const ENV: &str = "Usage: bifurc env <verb>

Global is always applied; the selected environment overrides it; a lone extra environment is selected
without being asked for.

  list
  show                                   The merged variables a request resolves against
  use <id|name>";

const MAPPINGS: &str = "Usage: bifurc mappings <verb>

A mapping serves <sub>.localhost from a target (host:port or an http(s):// URL).

  list
  show <id|domain>
  create <sub|domain> <target> [--label <text>] [--disabled]
  update <id|domain> [--domain <sub|domain>] [--target <target>] [--label <text>]
  enable|disable <id|domain>";

const RULES: &str = "Usage: bifurc rules <verb>

A rule sends the forward proxy's requests whose URL matches a pattern to a mapping or to another server.

  list
  show <id|name>
  create <pattern> (--mapping <id|domain> | --target <url>) [--name <text>] [--regex] [--disabled]
  update <id|name> [--pattern <text>] [--regex | --no-regex] [--mapping <id|domain> | --target <url>]
                   [--name <text>]
  enable|disable <id|name>               Enabling a rule disables another enabled rule with the same pattern

A rule's request and response scripts are written in the desktop app.";

const MOCKS: &str = "Usage: bifurc mocks <verb>

A mock created here is the whole response: nothing is asked of the real server.

  list
  show <id|name>
  create <url-pattern> [--method GET] [--name <text>] [--status 200] [-H \"k: v\"]...
                       [--body <text> | --body-file <f>] [--delay <ms>] [--regex] [--disabled]
  create --curl <command> [--name <text>] [--disabled]
                                         A mock of the call a cURL command makes, answering an empty 200
  update <id|name> [--url <pattern>] [--method <m>] [--name <text>] [--status <code>] [-H \"k: v\"]...
                   [--body <text> | --body-file <f>] [--delay <ms>] [--regex | --no-regex]
                                         Only what is given changes; -H \"k:\" removes a header
  enable|disable <id|name>               Enabling a mock disables another enabled mock of the same call

--method takes * for any method.";

const REQUESTS: &str = "Usage: bifurc requests <verb>

Saved requests of every type: rest (the default), graphql, grpc, soap, websocket, sse, socketio, mcp.

  list [--type <type>]
  show <id|name> [--type <type>]
  create [--type <type>] --url <url> [--name <text>] [-H \"k: v\"]... <options of the type>
  create --curl <command> [--name <text>]   The request a cURL command describes, of the type it turns out to be
  update <id|name> [--type <type>] <the options of create>
                                         Only what is given changes; -H \"k:\" removes a header
  send <id|name> [--type <type>]         Send a saved request; the engine resolves it and runs its scripts
  send [--type <type>] --url <url> <the options of create>
                                         Send a request that is not saved
  reflect <address>                      Services and methods a gRPC server reports through reflection

Options of each type
  rest        [-X, --method GET] [-d, --body <text> | --body-file <f>]
  graphql     (--query <text> | --query-file <f>) [--variables <json>] [--operation <name>]
  grpc        --method <pkg.Service/Method> [-d, --body <json> | --body-file <f>]
              [--proto <protoFileId> | --reflection]
              --url is the server address; -H is metadata; a client-streaming method takes a JSON array of
              messages; without --proto the method is found through reflection
  soap        [--action <SOAPAction>] [-d, --body <xml> | --body-file <f>]
  websocket   send takes [-d, --body <message>] [--wait <seconds, default 2>]: it connects, sends the
              message, prints what was sent and received in that time, and disconnects
  sse         [-X, --method GET] [-d, --body <text> | --body-file <f>]; send takes [--wait <seconds, default 5>]:
              it opens the stream, prints the events that arrive in that time (or until the server ends it),
              and closes it
  socketio    [--namespace </ns>] [--path </socket.io>] [--auth <json object>] [--listen <event>]...;
              only the events named with --listen are printed (every event when none is named); send takes
              [--event <name> [-d, --body <arguments>] [--ack]] [--wait <seconds, default 2>]: it connects,
              emits the event, prints what was emitted and received in that time, and disconnects. The
              arguments are a JSON array (one per element), any other JSON value or plain text (one argument);
              --ack asks the server to acknowledge the event
  mcp         [--transport stdio|http]; --url is the command that starts the server (stdio) or the address of
              its endpoint (http), and says which when --transport does not; -H is a header of an endpoint
              and an environment variable of a program. send connects and prints the tools, prompts and
              resources the server declares, or takes one of [--tool <name> [--arg <name=value>]...],
              [--prompt <name> [--arg <name=value>]...] and [--resource <uri>] and prints the answer, waiting
              [--wait <seconds, default 60>] for it; a value is read as the type the tool declares for it

--type is only needed to tell apart two requests of different types with the same name. Scripts and tests
are written in the desktop app; send runs them.";

const HEALTH: &str = "Usage: bifurc health <verb>

A health check is a URL, with {{variables}}, that answers or does not.

  list
  show <id|name>
  create <name> <url> [--auto]           --auto: the desktop app re-checks it every 30 seconds
  update <id|name> [--name <text>] [--url <url>] [--auto | --no-auto]
  run [<id|name>]                        Check one, or all of them; exits 1 when one does not answer";

const FLOWS: &str = "Usage: bifurc flows <verb>

A flow is a saved graph of requests: each node's output is the next one's input. Flows are made and edited
in the desktop app.

  list
  show <id|name>                         Inputs with their defaults, the nodes with the request each runs,
                                         and what the engine finds wrong with the flow
  run <id|name> [--input <name=value>]... [--timeout <seconds>] [--max-events <n>]
                                         Run it and print each node as it finishes, then a summary. Exits 1
                                         when the run does not succeed or a test fails.
                                         --input: a value for an input the flow declares (repeatable)
                                         --timeout: stop the run after that long (0: no limit)
                                         --max-events: stop after that many stream messages
                                         With --json nothing is printed until the end, then the run's report";

const VAULT: &str = "Usage: bifurc vault <verb>

Encrypted secrets, used as {{vault.SECRET}}. A vault is sealed or unsealed. Unsealing takes its passphrase
and lasts until the vault is sealed or the engine stops, whether it was unsealed here or in the desktop app.
A command that needs a sealed vault asks for the passphrase and unseals it; an unsealed one is used as it is.

  list                                   Vaults, whether each is sealed, and their secret names (never values)
  create <name>                          Asks for a new passphrase (twice); the new vault is unsealed
  set <vault> <SECRET>                   Asks for the value (hidden; or --value-stdin)
  unseal <vault>                         Asks for the passphrase
  seal <vault>
  run <vault> -- <command…>              Run the command with the vault's secrets as environment variables,
                                         e.g. bifurc vault run api_keys -- npm run dev
                                         Exits with the command's exit code

BIFURC_VAULT_PASSPHRASE or --passphrase-stdin give the passphrase without a prompt.

Removing a secret or a vault is done in the desktop app.";

const ACCOUNT: &str = "Usage: bifurc account <verb>

GitHub sign-in, per workspace; only needed to sync with a remote.

  list
  login                                  Shows a code to enter on GitHub, then waits
  repos <login>                          Repositories the account owns or was given access to

Signing out removes the account's workspaces from this device; it is done in the desktop app.";

const SERVICES: &str = "Usage: bifurc services list

Local processes listening on a port.";

const ENGINE: &str = "Usage: bifurc engine <verb>

  start [--transport socket|ws] [--no-tray]   Start a standalone engine in the background
  stop                                   Stop it gracefully; the interfaces using it close too
  restart                                Stop it and start it again; the interfaces reconnect
  status                                 Where it runs, without opening a session";

const UPDATE: &str = "Usage: bifurc update [<verb>]

The engine reads the list of releases once a day and knows how this copy was installed. Nothing is installed
from here: a Store install is updated by the Store, a package manager's by its own command, and any other by
the file this names.

  status                                 What the engine last found (the default)
  check                                  Ask now";

/// The name a command group is dispatched and documented under, whichever way it was spelled.
pub fn canonical_group(group: &str) -> &str {
    match group {
        "workspace" => "workspaces",
        "environment" | "environments" => "env",
        "mapping" => "mappings",
        "rule" | "proxy-rules" | "proxy-rule" => "rules",
        "mock" => "mocks",
        "request" => "requests",
        "healthcheck" | "healthchecks" | "health-check" | "health-checks" => "health",
        "flow" => "flows",
        "vaults" => "vault",
        "accounts" => "account",
        "service" => "services",
        other => other,
    }
}

/// The help of one command group, when it has its own.
pub fn group_usage(group: &str) -> Option<&'static str> {
    Some(match canonical_group(group) {
        "workspaces" => WORKSPACES,
        "env" => ENV,
        "mappings" => MAPPINGS,
        "rules" => RULES,
        "mocks" => MOCKS,
        "requests" => REQUESTS,
        "health" => HEALTH,
        "flows" => FLOWS,
        "vault" => VAULT,
        "account" => ACCOUNT,
        "services" => SERVICES,
        "engine" => ENGINE,
        "update" => UPDATE,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_group_of_the_overview_has_its_own_help() {
        for group in
            ["workspaces", "env", "mappings", "rules", "mocks", "requests", "health", "flows", "vault", "account", "services"]
        {
            assert!(USAGE.contains(&format!("\n  {group} ")), "{group} is not in the overview");
            let help = group_usage(group).unwrap_or_else(|| panic!("{group} has no help"));
            assert!(help.starts_with(&format!("Usage: bifurc {group} ")), "{group}");
        }
        assert_eq!(group_usage("status"), None);
    }

    #[test]
    fn aliases_reach_the_same_help() {
        assert_eq!(group_usage("mock"), group_usage("mocks"));
        assert_eq!(group_usage("proxy-rules"), group_usage("rules"));
        assert_eq!(group_usage("healthchecks"), group_usage("health"));
        assert_eq!(group_usage("vaults"), group_usage("vault"));
    }
}
