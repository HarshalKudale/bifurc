//! The text of `bifurc --help` and of each command group's `bifurc <group> --help`.

use crate::direct;

/// The usage text, printed for `--help`, `help` and no command, and appended to an unknown-command error.
pub const USAGE: &str = "Usage: bifurc <group> <verb> [arguments] [options]

Talks to the Bifurc engine that serves a data directory, over its socket (a named pipe on Windows).
The engine is found through <data-dir>/runtime.json. A command that needs the engine starts one in the
background if none is running; it keeps running with a tray icon after this terminal closes.

Everything the desktop app does once its first-run setup is done can be done here. Every group of saved
items has the same verbs: list, show, create, update, patch (set any field from JSON), duplicate, delete,
and enable|disable or the verb that runs it (send, run) where that applies. An item is named by its id or
by its name.

Groups (bifurc <group> --help shows the arguments and options of each verb)
  workspaces    list | show | create | update | use | sync | publish | import | delete
  env           list | show | use | create | set | unset | patch | duplicate | delete
  mappings      list | show | create | update | patch | enable | disable | duplicate | delete
                                                                      <sub>.localhost → target
  rules         list | show | create | update | patch | enable | disable | duplicate | delete
                                                                      Forward-proxy rules
  mocks         list | show | create | update | patch | enable | disable | duplicate | delete
                                                                      Mocked responses
  requests      list | show | create | update | patch | send | reflect | duplicate | delete
                                                                      REST, GraphQL, gRPC, SOAP, WebSocket, SSE, Socket.IO, MCP
  folders       list | create | rename | move | delete | confirm | restore
                                                                      Folders of the request, mock, flow and rule trees
  runner        run | status | cancel | config | configure             Send every request of a folder
  flows         list | show | create | update | validate | run | loadtest | loadstatus | loadcancel | status | cancel | duplicate | delete
                                                                      Saved chains of requests
  health        list | show | create | update | run | delete          Health checks
  capture       list | show | watch | mock | request | secrets | import | remove | clear | pause | resume | cache
                                                                      Traffic the proxies captured
  cookies       list | clear | configure                              The cookie jar of the workspace's requests
  sync          status | changes | pull | push | commit | diff | discard | confirm | restore
                                                                      Git: what changed, and the remote
  history       log | file | show | export                            The audit log and a file's commits
  export        formats | create                                      A workspace's items as a file
  import        check | run                                           A file of Postman, Insomnia, OpenAPI, …
  vault         list | create | set | unset | unseal | seal | rename | passphrase | delete | run
                                                                      Encrypted secrets
  account       list | login | repos | logout                         GitHub sign-in
  proxy         status | start | stop | restart                       The forward and the reverse proxy
  settings      show | update                                         Ports, HTTPS, telemetry
  tls           status | generate | export | importcert | importkey | remove
                                                                      The root certificate of HTTPS interception
  services      list                                                  Local processes listening on a port

Engine
  engine start [--transport socket|ws] [--no-tray]   Start a standalone engine in the background
  engine stop                            Stop it gracefully; the interfaces using it close too
  engine restart                         Stop it and start it again; the interfaces reconnect
  engine status                          Where it runs, without opening a session
  status                                 Engine, proxy and active workspace
  update [check | skip <version>]        Whether a newer Bifurc exists and how this install gets it; check asks now

Anything else
  curl <curl arguments> [--save <name>]  Run a curl command through the engine: shared cookie jar, environments, one summary line
  call <command> [<json-payload>]        Any engine command, e.g. call server.status
  commands                               List every engine command
  --mcp                                  First argument: serve these commands as tools for an AI agent (MCP, stdio)
  help [<group>]

Options
  --data-dir <path>    Default: the desktop app's data directory (or BIFURC_DATA_DIR)
  -w, --workspace <id> Default: the active workspace
  --no-start           Fail instead of starting an engine when none is running
  --json               Machine-readable output
  -h, --help

Exit codes
  0 succeeded    1 ran, but a test, expectation, script or check failed (a flow or folder run with a failure,
                 a health check that does not answer)
  2 usage error (unknown command, verb or option; missing or bad argument; ambiguous name)
  3 a named thing was not found    4 network error of what was sent (refused, DNS, timeout)
  5 the engine is not running and could not be started or reached
  6 needs a person: a sealed vault and no terminal to ask on; vault run exits with the program's own code";

const CURL: &str = "Usage: bifurc curl <curl arguments> [options]

curl's own arguments, run by the engine (its cookie jar, its environments), printing the summary line of
requests send. A leading `curl` word is ignored, and the whole command may be one quoted string.

  bifurc curl -X POST https://api.localhost/users -H 'Content-Type: application/json'
      -d '{\"email\":\"a@b.co\"}' --save \"Create user\" --folder auth   (all on one line)
  bifurc curl -s https://api.localhost/users/1 --select .email

curl options  -X -H -d/--data --data-raw --data-binary --data-ascii --data-urlencode --json -F --form-string
              -G -u -A -e -b --url --oauth2-bearer, -k (no certificate check), -L (follow redirects),
              -m/--max-time <seconds>. Accepted and ignored: -s -S -v --compressed --connect-timeout.
              -c/--cookie-jar <file> is ignored too: the engine's jar keeps the cookies. -d @file, --json @file
              and -F name=@file are read here. -i shows the headers, -I sends HEAD, -o <file> is --out,
              -f exits 1 on a status of 400 or more. Any other curl option is refused (exit 2).
Save          --save <name>     Save the request, then send the saved one
              --save-only       Save without sending
              --folder <path>   A folder of the request tree, e.g. auth/admin; missing ones are created
              --update          Replace the request of that name (its id, folder, scripts and tests stay)
              --no-templatize   Save as written. Otherwise a base URL, a credential or a value of the
                                environment becomes {{variable}}; a credential nobody holds becomes {{token}}
Output        --body --headers --select <path> --out <file> --max-body <bytes> --raw --quiet
              --expect-status <code> --expect '<path> <op> <value>'
              --bifurc-json     The machine-readable result (--json is curl's: data to send)
Run           --var KEY=VALUE   A variable for this run (repeatable)    --env <name>   Another environment
              --no-jar          Send without the workspace's cookie jar";

const WORKSPACES: &str = "Usage: bifurc workspaces <verb>

  list
  show [<id|name>]                       The active workspace when none is named
  create <name> [--account <login>]      A new local workspace, made active; with --account that account owns it
  update <id|name> --name <new name>
  use <id|name>                          Make it the active workspace
  sync [--message <text>]                Sync everything that was never synced, one commit per item, then push
                                         when there is a remote; --message is the commit message (bifurc sync
                                         --help has the finer verbs)";

const ENV: &str = "Usage: bifurc env <verb>

Global is always applied; the selected environment overrides it; a lone extra environment is selected
without being asked for.

  list
  show [--reveal]                        The merged variables; secret-looking names are masked
  use <id|name>
  set <id|name> <KEY=VALUE>...           Set variables of one environment (Global is named Global), adding
                                         the ones it does not have
  unset <id|name> <KEY>...               Remove variables of one environment";

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
  enable|disable <id|name>               Enabling a rule disables another enabled rule with the same pattern";

const MOCKS: &str = "Usage: bifurc mocks <verb>

A mock created here is the whole response: nothing is asked of the real server.

  list
  show <id|name>
  create <url-pattern> [--method GET] [--name <text>] [--status 200] [-H \"k: v\"]...
                       [-d, --data <text> | --body-file <f>] [--delay <ms>] [--regex] [--disabled]
  create --curl <command> [--name <text>] [--disabled]
                                         A mock of the call a cURL command makes, answering an empty 200
  update <id|name> [--url <pattern>] [--method <m>] [--name <text>] [--status <code>] [-H \"k: v\"]...
                   [-d, --data <text> | --body-file <f>] [--delay <ms>] [--regex | --no-regex]
                                         Only what is given changes; -H \"k:\" removes a header
  enable|disable <id|name>               Enabling a mock disables another enabled mock of the same call

--method takes * for any method.";

const REQUESTS: &str = "Usage: bifurc requests <verb>

Saved requests of every type: rest (the default), graphql, grpc, soap, websocket, sse, socketio, mcp.
A request is named by its id, its name, a unique start of its name (get -> Get order) or of its id (4+).

  list [--type <type>] [--folder <path>] [--grep <text>] [--ids]
                                         One line each: folder, method, URL, name
  show <name> [--curl] [--reveal]        Summary of a request; --curl prints it as a curl command (REST)
  create [--type <type>] --url <url> [--name <text>] [-H \"k: v\"]... <options of the type>
  create --curl <command> [--name <text>]   The request a cURL command describes, of the type it turns out to be
  update <id|name> [--type <type>] <the options of create>
                                         Only what is given changes; -H \"k:\" removes a header
  send <id|name> [--type <type>]         Send a saved request; the engine resolves it and runs its scripts
  send [--type <type>] --url <url> <the options of create>
                                         Send a request that is not saved
  --var KEY=VALUE                        Send with this variable ({{KEY}}) over the environment's; repeatable
  --env <id|name>                        Send with this environment instead of the selected one
  --data <text|@file>                    On a saved send: replaces its body (graphql: variables)
  --patch '<path>=<value>'               On a saved send: sets a field of its JSON body, e.g. '.qty=2'; repeatable
  reflect <address>                      Services and methods a gRPC server reports through reflection

Output of send (rest, graphql, soap, grpc): one line such as \"201 Created · 142ms · 4.2KB json · tests 3/3 ✓\";
a failure adds a reason line (\"✗ …\") and exits 1, and an error status shows up to 512 bytes of the body
  --body                                 The body, up to --max-body <bytes> (default 2048; 0 = all); text only
  --headers                              The response headers; secrets are masked (--reveal shows them on a terminal)
  --select <path>                        Only what a jq-style path picks from the JSON body, one value per line
  --out <file>                           Save the whole body to a file
  --raw                                  Exactly the body bytes, nothing else
  --quiet                                Nothing printed; the exit code says it
  --expect-status <code>                 Exit 1 unless the status is this
  --expect '<path> <op> <value>'         Exit 1 unless it holds, e.g. 'status < 300' or '.items | length > 0'; repeatable
  --json                                 {schemaVersion, status, statusText, durationMs, size, contentType, tests, body, …}

Options of each type
  rest        [-X, --method GET] [-d, --data <text|@file> | --body-file <f>]
  graphql     (--query <text> | --query-file <f>) [--variables <json>] [--operation <name>]
  grpc        --method <pkg.Service/Method> [-d, --data <json|@file> | --body-file <f>]
              [--proto <protoFileId> | --reflection]
              --url is the server address; -H is metadata; a client-streaming method takes a JSON array of
              messages; without --proto the method is found through reflection
  soap        [--action <SOAPAction>] [-d, --data <xml|@file> | --body-file <f>]
  websocket   send takes [-d, --data <message>] [--wait <seconds, default 2>]: it connects, sends the
              message, prints what was sent and received in that time, and disconnects
  sse         [-X, --method GET] [-d, --data <text|@file> | --body-file <f>]; send takes [--wait <seconds, default 5>]:
              it opens the stream, prints the events that arrive in that time (or until the server ends it),
              and closes it
  socketio    [--namespace </ns>] [--path </socket.io>] [--auth <json object>] [--listen <event>]...;
              only the events named with --listen are printed (every event when none is named); send takes
              [--event <name> [-d, --data <arguments>] [--ack]] [--wait <seconds, default 2>]: it connects,
              emits the event, prints what was emitted and received in that time, and disconnects. The
              arguments are a JSON array (one per element), any other JSON value or plain text (one argument);
              --ack asks the server to acknowledge the event
  mcp         [--transport stdio|http]; --url is the command that starts the server (stdio) or the address of
              its endpoint (http), and says which when --transport does not; -H is a header of an endpoint
              and an environment variable of a program. send connects and prints the tools, prompts and
              resources the server declares, or takes one of [--tool <name> [--arg <name=value>]...],
              [--prompt <name> [--arg <name=value>]...] and [--resource <uri>] and prints the answer, waiting
              [--wait <seconds, default 60>] for it; a value is read as the type the tool declares for it

--type is only needed to tell apart two requests of different types with the same name. send runs the
request's scripts and tests; they are written with patch, as is anything else create and update have no
option for (bifurc requests show <request> --json prints every field).";

const HEALTH: &str = "Usage: bifurc health <verb>

A health check is a URL, with {{variables}}, that answers or does not.

  list
  show <id|name>
  create <name> <url> [--auto]           --auto: the desktop app re-checks it every 30 seconds
  update <id|name> [--name <text>] [--url <url>] [--auto | --no-auto]
  run [<id|name>]                        Check one, or all of them; exits 1 when one does not answer
  delete <id|name>";

const FLOWS: &str = "Usage: bifurc flows <verb>

A flow is a saved graph of nodes: each node's output is the next one's input. It is written as JSON
(create, update) and checked with validate, which names every problem and the ports of each node.

The JSON of a flow
  {\"name\": \"Login then list\", \"folderId\": \"<folder of the flow tree, optional>\",
   \"inputs\": [{\"name\": \"user\", \"value\": \"default\"}],
   \"settings\": {\"timeoutMs\": 60000, \"persistVariables\": false},
   \"nodes\": [{\"id\": \"start\", \"type\": \"start\", \"name\": \"\", \"x\": 60, \"y\": 120}, …],
   \"edges\": [{\"id\": \"e1\", \"from\": \"start\", \"port\": \"out\", \"to\": \"login\"}, …]}

  Every node has id, type, name and x, y (where the desktop app draws it). By type, with its output ports:
  start      (one per flow, where a run begins)                                          out
  request    requestKind, requestId: the saved request it sends (requestKind is the kind
             requests list --json shows: requests, graphqlRequests, grpcRequests,
             soapRequests, sseRequests, wsConnections, socketioRequests); executeScripts,
             executeTests (default true), ignoreTestErrors, failOnStatus (default true: a
             non-2xx answer leaves by error), retries, retryDelayMs                      success, error
  stream     requestKind, requestId of an sse, websocket, socketio or grpc request;
             message (what is sent), event (socketio), maxMessages, idleTimeoutMs,
             listenMs                                                                    message, closed, error
  mcp        requestKind mcpRequests, requestId; call: tool|prompt|resource; target: its
             name or URI; arguments: [{name, value}]                                     success, error
  condition  branches: [{id, name, match: all|any, rules: [{path, op, value}]}]          <branch id>…, else
  if         expression: JavaScript over input and vars                                  true, false
  validate   rules: [{path, op, value}]                                                  pass, error
  evaluate   expression: JavaScript over input and vars; its value is the output         out, error
  delay      delayMs                                                                     out
  or         (goes on when any one of its inputs arrives)                                out
  repeat     count                                                                       each, done
  forEach    path: the list in its input to go through                                   each, done
  data       value: any JSON, a constant                                                 out
  sample     value: any JSON, handed on as the output                                    out
  display    (shows its input in the desktop app)                                        out

  op is one of equals, notEquals, contains, notContains, startsWith, endsWith, matches, greaterThan,
  greaterOrEqual, lessThan, lessOrEqual, exists, notExists, isEmpty, notEmpty, isType.
  An edge with no input is a run connection: when `from` leaves by `port`, `to` runs with that output as
  its input. An edge with input is a wire that fills one value of `to`: input is var:<name> (a variable of
  the node or of its request), arg:<name> (an mcp argument) or message (a stream's message); path picks a
  part of the source's output, e.g. [\"body\", \"items\", 0, \"id\"].
  Text fields may hold {{input.body.id}} (what the node before produced), {{nodes.<id>.headers.location}}
  (what an earlier node did), {{flow.<input>}} (an input of the run) and {{variables}} of the environment.

  list
  show <id|name>                         Inputs with their defaults, the nodes with the request each runs,
                                         and what the engine finds wrong with the flow
  run <id|name> [--input <name=value>]... [--timeout <seconds>] [--max-events <n>]
                                         Run it and print each node as it finishes, then a summary. Exits 1
                                         when the run does not succeed or a test fails.
                                         --input: a value for an input the flow declares (repeatable)
                                         --timeout: stop the run after that long (0: no limit)
                                         --max-events: stop after that many stream messages
                                         With --json nothing is printed until the end, then the run's report
  loadtest <id|name> --instances <n> [--input <name=value>]... [--timeout <seconds>] [--max-events <n>]
                                         Run it <n> times at once (1 to 10000) and print how many passed and
                                         failed, how long an instance and each node took, and what failed how
                                         often. Exits 1 when an instance fails. The other options are those of
                                         run, for every instance alike. An instance is a user of its own: it
                                         has its own cookies and keeps the variables its scripts set. It fails
                                         as a run does (a failed test fails it unless its node ignores tests).
                                         A flow with an MCP node is refused.";

const VAULT: &str = "Usage: bifurc vault <verb>

Encrypted secrets, used as {{vault.SECRET}}. A vault is sealed or unsealed. Unsealing takes its passphrase
and lasts until the vault is sealed or the engine stops, whether it was unsealed here or in the desktop app.
A command that needs a sealed vault asks for the passphrase and unseals it; an unsealed one is used as it is.

  list                                   Vaults, whether each is sealed, and their secret names (never values)
  create <name>                          Asks for a new passphrase (twice); the new vault is unsealed
  set <vault> <SECRET>                   Asks for the value (hidden; or --value-stdin)
  unseal <vault>                         Asks for the passphrase
  seal <vault>
  passphrase <vault>                     Asks for the passphrase and for a new one (twice)
  run <vault> -- <command…>              Run the command with the vault's secrets as environment variables,
                                         e.g. bifurc vault run api_keys -- npm run dev
                                         Exits with the command's exit code

BIFURC_VAULT_PASSPHRASE or --passphrase-stdin give the passphrase without a prompt, and
BIFURC_VAULT_NEW_PASSPHRASE the new one of passphrase.";

const ACCOUNT: &str = "Usage: bifurc account <verb>

GitHub sign-in, per workspace; only needed to sync with a remote.

  list
  login                                  Shows a code to enter on GitHub, then waits
  repos <login>                          Repositories the account owns or was given access to";

const FOLDERS: &str = "Usage: bifurc folders <verb>

Requests, mocks, flows and rules are each kept in a tree of folders; --tree says which. A folder is named
by its id. An item is moved to a folder by setting its folderId: bifurc mocks patch <mock> --data
'{\"folderId\": \"<id>\"}'.

  list --tree request|mock|flow|rule     The folders of one tree of the active workspace";

const RUNNER: &str = "Usage: bifurc runner <verb>

Sends every request of a folder of the request tree, in order, with its scripts and tests.

  run <folder>                           Run it and print how each request did; exits 1 when a request
                                         fails or a test does";

const CAPTURE: &str = "Usage: bifurc capture <verb>

What went through the proxies: each exchange with its request and its response.

  list [--last N] [--status 4xx|422|!2xx] [--method M] [--host H] [--path P*] [--since 2m|1h|#id]
       [--origin real|mock|mapped|rule] [--grep text]
                                         One line per entry, newest first (default 20): #id time method
                                         host/path status duration origin. Times are UTC.
  show <entry> [--body] [--headers] [--req-body] [--req-headers] [--select <path>] [--max-body N]
                                         The response as `requests send` prints it (--out, --raw, --quiet,
                                         --expect* too); --req-* show the request (secrets masked)
  watch [filters] [--count N] [--timeout S]
                                         One JSON line per new matching entry until stopped
  An <entry> is a full id, #tail or tail (the end of the id, as `list` prints it).
  mock <entry>                           Save a mock that answers as the captured exchange did
  request <entry>                        Save a request like the captured one";

const COOKIES: &str = "Usage: bifurc cookies <verb>

The cookies the workspace's requests were given and send back. Values print masked.

  list [--domain <d>] [--reveal]         One line per cookie; --reveal shows values on a terminal
  clear [--domain <d>]                   Forget all cookies, or one domain's
  configure --clear true|false           Forget cookies when the engine restarts";

const SYNC: &str = "Usage: bifurc sync <verb>

A workspace is a git repository: every saved item is a file, a sync is a commit and a push. bifurc
workspaces sync commits and pushes everything at once.";

const HISTORY: &str = "Usage: bifurc history <verb>

Who changed what: the audit log of the workspace, and the commits of one file.";

const EXPORT: &str = "Usage: bifurc export <verb>

An export is Bifurc JSON; bifurc import run reads it back.";

const IMPORT: &str = "Usage: bifurc import <verb>

Reads a file of another tool (Postman, Insomnia, OpenAPI, HAR, …) or a Bifurc export; bifurc export formats
lists the kinds and the formats of each.";

const PROXY: &str = "Usage: bifurc proxy <verb>

The forward proxy (rules, mocks, capture) and the reverse proxy (<sub>.localhost mappings).";

const SETTINGS: &str = "Usage: bifurc settings <verb>

  show                                   The ports, and whether HTTPS, telemetry and the update check are on";

const TLS: &str = "Usage: bifurc tls <verb>

The root certificate HTTPS interception signs with. HTTPS itself is switched with bifurc settings update
--https true|false.";

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
        "folder" => "folders",
        "cookie" => "cookies",
        "git" => "sync",
        "audit" => "history",
        "setting" | "config" => "settings",
        "cert" | "certificate" | "https" => "tls",
        other => other,
    }
}

/// The help of one command group, when it has its own: what is written here, then the verbs that are rows
/// of `direct.rs`.
pub fn group_usage(group: &str) -> Option<String> {
    let group = canonical_group(group);
    let own = match group {
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
        "folders" => FOLDERS,
        "runner" => RUNNER,
        "capture" => CAPTURE,
        "cookies" => COOKIES,
        "sync" => SYNC,
        "history" => HISTORY,
        "export" => EXPORT,
        "import" => IMPORT,
        "proxy" => PROXY,
        "settings" => SETTINGS,
        "tls" => TLS,
        "curl" => CURL,
        _ => return None,
    };
    let rows = direct::usage(group);
    Some(if rows.is_empty() { own.to_string() } else { format!("{own}\n\n{rows}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_group_of_the_overview_has_its_own_help() {
        let mut groups = vec!["health", "services"];
        groups.extend(direct::verbs().iter().map(|verb| verb.words[0]).filter(|group| *group != "update"));
        for group in groups {
            assert!(USAGE.contains(&format!("\n  {group} ")), "{group} is not in the overview");
            let help = group_usage(group).unwrap_or_else(|| panic!("{group} has no help"));
            assert!(help.starts_with(&format!("Usage: bifurc {group} ")), "{group}");
        }
        assert_eq!(group_usage("status"), None);
    }

    #[test]
    fn the_overview_names_every_verb_of_the_tables() {
        for verb in direct::verbs() {
            let (group, name) = (verb.words[0], verb.words[1]);
            let line = USAGE
                .lines()
                .find(|line| line.starts_with(&format!("  {group} ")))
                .unwrap_or_else(|| panic!("{group}"));
            assert!(line.split([' ', '|', '[', ']']).any(|word| word == name), "{group} {name} is not in the overview");
            assert!(group_usage(group).unwrap().contains(&format!("\n  {name}")), "{group} {name} is not in its help");
        }
    }

    #[test]
    fn aliases_reach_the_same_help() {
        assert_eq!(group_usage("mock"), group_usage("mocks"));
        assert_eq!(group_usage("proxy-rules"), group_usage("rules"));
        assert_eq!(group_usage("healthchecks"), group_usage("health"));
        assert_eq!(group_usage("vaults"), group_usage("vault"));
    }
}
