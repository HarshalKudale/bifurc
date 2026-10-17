//! The text of `bifurc --help` and of each command group's `bifurc <group> --help`.

use crate::direct;

/// The short overview, printed for `--help`, `help` and no command, and appended to an unknown-command error.
pub const OVERVIEW: &str = "Usage: bifurc <group> <verb> [arguments] [options]

Runs requests, mocks, flows and the proxies of a Bifurc workspace; starts an engine when none runs.

  agent     init [--skill]                                 Teach an AI agent this CLI: a cheat sheet in AGENTS.md / CLAUDE.md
  curl      <curl arguments> [--save <name>]               A curl command through the engine, one summary line
  requests  list | show | send | create | update | delete  Saved requests of every type
  capture   list | show | diff | watch | pin | mock | request | clear
  mocks     list | show | create | update | enable | disable | scenario | status | hits | serve
  flows     list | show | run | validate | loadtest         Saved chains of requests
  runner    run <folder> [--bail]                          Run a folder's requests
  contract  check | update-mock | snapshot                 Do the mocks still match the real API? Sends requests
  env       list | show | use | set | unset                Environments and their variables
  cookies   list | set | clear                             The cookie jar (yours in a --session)
  session   show | reset · vars list | set | clear         Your own cookies, variables and env (--session <name>)
  changes   [--since <seq|2m|1h>] [--mine]                 What others changed since you last asked
  wait      <url|check> | --port <n> | --capture           Block until something is ready
  status                                                   Engine, proxy, mocks, capture and cookies, two lines
  Also: setup, workspaces, mappings, rules, folders, health, sync, history, export, import, vault, account, proxy,
  settings, tls, services, engine, update, call, commands

Options
  --json  --data-dir <path>  -w, --workspace <id>  --session <name> (or BIFURC_SESSION)  --no-start  -h, --help

  bifurc help <group>             The verbs and options of a group
  bifurc help <group> --examples  Copy-paste examples
  bifurc help --all               Every group and verb
Exit codes: 0 ok, 1 a check failed, 2 usage, 3 not found, 4 network, 5 no engine, 6 needs a person";

/// The full usage text, printed for `help --all`.
pub const USAGE: &str = "Usage: bifurc <group> <verb> [arguments] [options]

Talks to the Bifurc engine that serves a data directory, over its socket (a named pipe on Windows).
The engine is found through <data-dir>/runtime.json. A command that needs the engine starts one in the
background if none is running; it keeps running with a tray icon after this terminal closes.

Everything the desktop app does once its first-run setup is done can be done here. Every group of saved
items has the same verbs: list, show, create, update, patch (set any field from JSON), duplicate, delete,
and enable|disable or the verb that runs it (send, run) where that applies. An item is named by its id or
by its name.

Groups (bifurc <group> --help shows the arguments and options of each verb)
  agent         init                                                  A cheat sheet of this CLI for AI agents, in the repo
  workspaces    list | show | create | update | use | sync | publish | import | delete
  env           list | show | use | create | set | unset | patch | duplicate | delete
  mappings      list | show | create | update | patch | enable | disable | duplicate | delete
                                                                      <sub>.localhost → target
  rules         list | show | create | update | patch | enable | disable | duplicate | delete
                                                                      Forward-proxy rules
  mocks         list | show | create | update | patch | enable | disable | scenario | status | hits | serve | duplicate | delete
                                                                      Mocked responses
  requests      list | show | create | update | patch | send | reflect | duplicate | delete
                                                                      REST, GraphQL, gRPC, SOAP, WebSocket, SSE, Socket.IO, MCP
  folders       list | create | rename | move | delete | confirm | restore
                                                                      Folders of the request, mock, flow and rule trees
  runner        run | status | cancel | config | configure             Run a folder's requests, a line each
  contract      check | update-mock | snapshot                        Compare mocks with what the real API answers
  flows         list | show | create | update | validate | run | loadtest | loadstatus | loadcancel | status | cancel | duplicate | delete
                                                                      Saved chains of requests
  health        list | show | create | update | run | delete          Health checks
  capture       list | show | diff | watch | pin | unpin | mock | request | secrets | import | remove | clear | pause | resume | cache
                                                                      Traffic the proxies captured
  cookies       list | set | clear | configure                       The cookie jar of the workspace's requests
  session       show | reset                                          Your named session: cookies, variables, environment
  changes       [--since <seq|time>] [--mine]                         What others changed since you last asked
  claim         [--release] <request|mock|flow|env> <name>            Mark an item you are editing (30 minutes)
  claims                                                              The live claims
  vars          list | set | clear                                    The runtime variables of the session
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
  services      list | define | up | down | restart | remove | ps | show   Dev servers the engine runs; local ports
  logs          <service> [--errors] [--since 2m] [--grep x] [--last 50]   A dev server's output, filtered
  port          free <port> [--kill]                                  What holds a port, and stopping it

Engine
  engine start [--transport socket|ws] [--no-tray]   Start a standalone engine in the background
  engine stop                            Stop it gracefully; the interfaces using it close too
  engine restart                         Stop it and start it again; the interfaces reconnect
  engine status                          Where it runs, without opening a session
  status                                 Engine, proxy, workspace, environment, mocks, capture and cookies in two lines
  wait <url|check> | --port <n> | --capture   Block until a URL answers, a port accepts, or a request is captured
  setup [--accept-terms --accept-privacy] [--workspace <name>]   First-run setup without the app (see: bifurc help setup)
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
  --session <name>     Work in a named session: its own cookie jar, runtime variables and environment (or
                       BIFURC_SESSION). Without one you share the desktop app's jar and selected environment
  --no-start           Fail instead of starting an engine when none is running
  --json               Machine-readable output
  -h, --help

Exit codes
  0 succeeded    1 ran, but a test, expectation, script or check failed (a flow or folder run with a failure,
                 a health check that does not answer)
  2 usage error (unknown command, verb or option; missing or bad argument; ambiguous name), or an update
    refused because another session changed the item since you showed it (--force overwrites)
  3 a named thing was not found    4 network error of what was sent (refused, DNS, timeout)
  5 the engine is not running and could not be started or reached
  6 needs a person: a sealed vault and no terminal to ask on; vault run exits with the program's own code";

const AGENT: &str = "Usage: bifurc agent init [--dir <repo>] [--target agents|claude|both] [--skill]

Teaches an AI agent this CLI in one read: a short cheat sheet, then the names in the workspace (environments,
request folders, flows), written between <!-- bifurc:begin --> and <!-- bifurc:end -->. Run it again to refresh
the block in place; the rest of the file is not touched.

  init                 Write the block into the repo's AGENTS.md and/or CLAUDE.md
  --dir <repo>         The repo (default: the current directory)
  --target <files>     agents, claude or both. Default: the one that exists, both when both do, AGENTS.md when none
  --skill              Also write .claude/skills/bifurc/SKILL.md (overwritten): the same sheet as a skill
With --no-start and no engine running, the workspace names are left out.";

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
              --no-jar          Send without the workspace's cookie jar   --fresh-session   An empty throwaway jar
              --extract name=<path>   Keep a value of the response for later {{name}} (see: bifurc session --help)";

const WAIT: &str = "Usage: bifurc wait <url|health check> [--status <code>] [--timeout 30s] [--interval 500ms]
       bifurc wait --port <n> [--host 127.0.0.1] [--timeout 30s] [--interval 500ms]
       bifurc wait --capture [--method M] [--host H] [--path P*] [--status 4xx] [--origin real|mock|mapped|rule|error|server]
                   [--grep text] [--timeout 30s] [--interval 500ms]

Polls until the thing is ready, then prints one line (`ready · http://api.localhost/health 200 · 2.4s`) and
exits 0. After --timeout it exits 4 and says why the last attempt failed.
  <url>       Ready when the URL answers with any status, or with --status. A health check's name means its URL;
              {{variables}} are resolved. http:// is added to an address without a scheme.
  --port      Ready when a TCP connection is accepted. Needs no engine.
  --capture   Ready when a new request that matches the filters of capture list is captured.
Durations are 500ms, 30s, 2m or a number of seconds.";

const SETUP: &str =
    "Usage: bifurc setup [--accept-terms --accept-privacy] [--workspace <name>] [--port <n> --proxy-port <n>]

The app's first-run setup, for a machine that has no app (a container, CI). Every other command is refused
until it is done. Never run implicitly.
  (no flags)        Print the state and the URLs of the terms and the privacy policy; exit 0 when the setup is
                    finished, 6 when it is not.
  --accept-terms --accept-privacy
                    Record that YOU accept the terms of service and the privacy policy at the URLs it prints.
                    Both are required; with one, that one is recorded and the other is named (exit 6).
  --workspace       Name of the first workspace (default Default); made only when there is none.
  --port            Reverse proxy port (default 80)    --proxy-port   Forward proxy port (default 8080)
Order, as in the app: accept terms, accept privacy, start the proxies, create the workspace, complete.
Run again when finished, it only prints the state.";

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
  unset <id|name> <KEY>...               Remove variables of one environment
  auth <id|name> --login <request|flow> [--flow <name>] [--retry true|false] | --off
                                         Run that login once on a 401, then send again (`↻ re-authenticated`)";

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
  create --from-request <request> [--name <text>] [--var k=v]... [--env <name>] [--disabled]
                                         Send the saved REST request once and mock the answer it got
  create --from-capture <entry> [--name <text>] [--disabled]
                                         Mock a captured exchange (same as: capture mock <entry>)
  update <id|name> [--url <pattern>] [--method <m>] [--name <text>] [--status <code>] [-H \"k: v\"]...
                   [-d, --data <text> | --body-file <f>] [--delay <ms>] [--regex | --no-regex]
                                         Only what is given changes; -H \"k:\" removes a header
  enable|disable <id|name>               Enabling a mock disables another enabled mock of the same call
  scenario <mock> [<scenario>] [--list] [--delay <ms>]
                                         Switch a call between its answers; no scenario (or --list) lists them
  status                                 A line per call: name, method, pattern, active scenario, on/off, hits
  hits <mock> [--last 5]                 Captured requests this call's mocks answered: did the app hit the mock?
  serve [--port 4010] [--host 127.0.0.1] [--fallthrough <base-url>] [--detach] [--quiet]
                                         The enabled mocks over plain HTTP on a local port, no proxy or
                                         certificate (for Node tests, CI, backends). Scenarios apply. Blocks and
                                         prints a line per request; stops with Ctrl+C or the end of the command;
                                         --detach keeps it running under the engine. The host part of a mock's
                                         pattern is ignored: /orders matches http://api.localhost/orders.
                                         No match: 404 with the closest mocks, or --fallthrough forwards.
  serve --stop [--port N] | --status      Stop the mock server(s) / list the running ones

--method takes * for any method.

Scenarios: the mocks of one call (same method and URL pattern) are the answers of that call, one enabled.
The one named plainly is `default`; a scenario is named \"<default name> · <scenario>\" (Get orders · empty).
A scenario that does not exist and is a preset is created from the default mock, then switched on:
  error500 (500) | error401 (401) | notfound (404) | empty (200 [] or {}) | slow (default answer + --delay,
  3000 ms) | timeout (default answer + a 60000 ms delay, longer than common client timeouts)";

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
  --extract name=<path>                  After the send, keep the value at <path> of the JSON body (or `status`)
                                         as the session's variable {{name}}; repeatable; exit 1 if it is absent
  --fresh-session                        Send (rest) with an empty throwaway cookie jar
  --var KEY=VALUE                        Send with this variable ({{KEY}}) over the environment's; repeatable
  --env <id|name>                        Send with this environment instead of the selected one
  --data <text|@file>                    On a saved send: replaces its body (graphql: variables)
  --patch '<path>=<value>'               On a saved send: sets a field of its JSON body, e.g. '.qty=2'; repeatable
  reflect <address>                      Services and methods a gRPC server reports through reflection

Output of send (rest, graphql, soap, grpc): one line such as \"201 Created · 142ms · 4.2KB json · tests 3/3 ✓\";
a failure adds a reason line (\"✗ …\") and exits 1, and an error status shows up to 384 bytes of the body
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

A flow is a saved graph of nodes: each node's output is the next one's input. A straight line of requests
is written as steps (create --steps); any graph as JSON (create, update). validate names every problem and
the ports of each node.

Steps (YAML or JSON), compiled into the graph; `flows show` prints a line flow back as steps
  name: Checkout happy path
  steps:
    - request: Login as admin        a saved request by name: exact, any case or a unique prefix
    - request: Create cart
      extract: { cartId: .id }       later steps can use {{cartId}} in the URL, headers and body
    - request: Add item
      vars: { qty: 2 }               variables for this step only
      expect: [ status == 201, .items | length == 1 ]   == != < <= > >= contains; a miss fails the run
                                     in [ ], quote one with an index: '.items[0].qty == 2'
    - inline: POST {{baseUrl}}/orders      a request made here, saved as <flow> · <n>; headers:, body:

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
  create --steps <file|-> [--name <text>] [--folder <path>]
                                         Compile a step file into a new flow (`-` reads stdin); prints one line
                                         per step, then `saved flow \"Checkout\" · 4 steps`. A problem (exit 2
                                         for a bad step, 3 for an unknown request) saves nothing.
  update <id|name> --steps <file> [--name <text>]
                                         Compile the file again into that flow, keeping its id and inputs
  show <id|name> [--steps]               Inputs with their defaults, a line flow as steps
                                         (`2 POST /carts  Create cart  extract cartId=.id`) or else the nodes
                                         with the request each runs, and what the engine finds wrong.
                                         --steps prints the step file (exit 2 when the flow is not a line)
  run <id|name> [--input <name=value>]... [--var <k=v>]... [--env <name>] [--fresh-session] [--verbose]
      [--timeout <seconds>] [--max-events <n>] [--max-body <bytes>]
                                         Run it; one line per request (or failed node), then `flow passed · 5
                                         steps · 166ms` or `flow failed at step 3`. Only a failed step shows
                                         its body (384 bytes); exits 1 when the run or a test fails.
                                         --input: a value for an input the flow declares (repeatable)
                                         --var, --env: variables / environment over the environment's, as on a send
                                         --fresh-session: cookies of a jar of its own, dropped at the end
                                         --verbose: every step's body, cut at --max-body (default 2048; 0: all)
                                         --timeout: stop the run after that long (0: no limit)
                                         --max-events: stop after that many stream messages
                                         With --json nothing is printed but the run's report
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

  run <folder> [--bail] [--var <k=v>]... [--env <name>] [--fresh-session] [--verbose] [--max-body <bytes>]
                                         <folder>: id, path (auth/admin) or name. One line per request, then
                                         `5 run · 4 passed · 1 failed · 812ms`; only a failed one shows its
                                         body. Exits 1 when a request fails or a test does.
                                         --bail: stop after the first failure; --verbose: every body";

const CONTRACT: &str = "Usage: bifurc contract check | update-mock <mock> | snapshot [options]

Do the mocks the frontend relies on still match what the backend really answers? Each enabled mock is paired
with a saved REST request of the same method and path ({{var}} segments match any one segment, query and host
are ignored). The request is sent, and status, content type and body shape (keys, types, array elements; never
values or headers) are compared.

  check                                  One line per paired mock, the differences under a drifted one, then a count
  --mock <name>                          Only this mock (also when disabled); its reason is shown when unpaired
  --folder <mock folder>                 Only the mocks in this folder and below
  --all                                  Disabled mocks too
  --ignore <glob>                        Leave out differences whose path matches; * is any run of characters,
                                         e.g. '.meta.*' or '.items[].debug'. Repeatable
  --from-capture                         Compare with the newest captured real answer (not a mock's) instead
  --from-snapshot                        Compare with the stored snapshots only; sends nothing
  --openapi <file>                       Compare with an OpenAPI 3 / Swagger 2 file (JSON or YAML) instead: the
                                         first 2xx JSON response schema of the operation with the mock's method
                                         and path ({id} matches any one segment; the servers[0].url / basePath
                                         prefix is optional). Sends and stores nothing; rows end `(openapi)`;
                                         a mock without an operation is unpaired (`not in the spec`). In an
                                         object with a `required` list only those properties count (a mock may
                                         omit or carry optional ones); without one, all do. Not with the two above
  --var k=v  --env <name>  --fresh-session   As for requests send
  --json  --quiet                        Machine-readable; or nothing but the exit code

check SENDS the paired real requests (POST, DELETE and the like too); --from-capture sends nothing. A send is
never answered by a mock. Left is the mock, right the real answer: `+ .items[].discount (number)` means the real
API has it and the mock lacks it, `- .x` the reverse, `~ status 201 → 200` mock then real.
Mocks with no single saved request (none, several, or a regex pattern) are counted as unpaired; --json lists them.
Exit: 1 any drift, else 4 a real request failed, else 0.
When a real request cannot be reached and a snapshot of it exists, the mock is compared with the snapshot and
the row ends `(snapshot)`; that counts as ok or drifted, not failed.

  update-mock <mock>                     Rewrite the mock's body to the real shape, keeping its values where a key
                                         stays; also its status when it differs. Headers stay. Sends the request
                                         (or --from-capture). Prints the fixed differences, then `updated mock`
  snapshot [--mock] [--folder] [--all]   Store the shape of each paired request's answer in the workspace
                                         (contracts/, committed with it): types only, never a value. Sends each
                                         request once. Exit 4 if a send failed, after saving the rest";

const CAPTURE: &str = "Usage: bifurc capture <verb>

What went through the proxies: each exchange with its request and its response.

  list [--last N] [--status 4xx|422|!2xx] [--method M] [--host H] [--path P*] [--since 2m|1h|#id]
       [--origin real|mock|mapped|rule|error|server] [--grep text] [--pinned]
                                         One line per entry, newest first (default 20): #id time method
                                         host/path status duration origin. Times are UTC; #id* is pinned.
  show <entry> [--body] [--headers] [--req-body] [--req-headers] [--select <path>] [--max-body N]
                                         The response as `requests send` prints it (--out, --raw, --quiet,
                                         --expect* too); --req-* show the request (secrets masked)
  diff <a> [<b>] [--mock <name>] [--request <name>] [--shape] [--var K=V] [--env <name>]
                                         Two sides (entries, a mock, a saved request that is sent now):
                                         + .new (number), - .gone, ~ .a 1 → 2; --shape: no values
  watch [filters] [--count N] [--timeout S]
                                         One JSON line per new matching entry until stopped
  pin <entry>...                         Keep entries beyond the buffer limit (200; settings update
                                         --capturelimit N); pinned ones do not count toward it
  unpin <entry>...                       Let them age out again
  An <entry> is a full id, #tail or tail (the end of the id, as `list` prints it).
  mock <entry>                           Save a mock that answers as the captured exchange did
  request <entry>                        Save a request like the captured one";

const COOKIES: &str = "Usage: bifurc cookies <verb>

The cookies the workspace's requests were given and send back. Values print masked.

  list [--domain <d>] [--reveal]         One line per cookie; --reveal shows values on a terminal
  set <domain|url> <name>=<value>        Seed a cookie (host-only, path /), e.g. a feature flag
  clear [--domain <d>]                   Forget all cookies, or one domain's
  configure --clear true|false           Forget cookies when the engine restarts";

const SESSION: &str = "Usage: bifurc session [show | reset]

A named session (--session <name> or BIFURC_SESSION) keeps its own cookie jar, runtime variables and, with
`env use`, its own environment, so two agents (or an agent and you) never clash. Saved requests, mocks, flows
and environments stay shared. Without a name you are the default session: the app's jar and selected
environment. The jar and the variables live as long as the engine and are never written to disk.

  show                                   `session frontend · 2 variables · env dev · cookies 1 domain`
  reset                                  Empty the session's jar and variables and forget its environment
                                         (the default session: only the variables --extract and vars set kept)
Resolution: --var, then the session's variables, then the environment, then Global. A script's
bc.environment.set in a named session stays in the session and not in the environment's file.";

const CHANGES: &str = "Usage: bifurc changes [--since <seq|2m|1h|time>] [--mine] [--json]

What changed in the workspace since this session last asked, oldest first, one line each: the kind, the name,
what was done (and which fields), who did it (`session:<name>`, `cli`, `app`) and when. Your own changes are
left out (--mine includes them). Held in memory since the engine started (the audit log keeps the full
history; `history log` shows the session of each commit).
Name the session with --session <name> or BIFURC_SESSION; without one the cursor is the command line's own.

  (no option)                            Show what is new and move this session's cursor past it
  --since <seq|2m|1h|2d|time>            Show everything after a sequence number, a time ago or a UTC time;
                                         the cursor stays
  --mine                                 Include the changes this session made itself
  --json                                 {schemaVersion, changes: [{seq, ts, kind, id, name, action, fields, by}], cursor}";

const CLAIM: &str = "Usage: bifurc claim [--release] <request|mock|flow|env> <name> | bifurc claims

Tell the other agents which item you are editing. Advisory only, nothing is blocked: when someone else updates
a claimed item the update goes through and prints `bifurc: warning: \"Get orders\" is claimed by session:frontend
(3m ago)` on stderr (exit code unchanged). Claims live in the engine's memory and lapse after 30 minutes;
claiming again renews. Deleting the item drops its claim. Name the session with --session or BIFURC_SESSION.

  claim <kind> <name>                    `claimed mock \"Get orders\"`, or `… — was held by session:frontend 3m ago`
  claim --release <kind> <name>          `released mock \"Get orders\"` (anyone may release)
  claims                                 One line per claim: `mock  \"Get orders\"  by session:backend  3m ago`
  --json                                 {schemaVersion, claim, previous} / {schemaVersion, claims: [{id, kind, name, by, ts}]}";

const VARS: &str = "Usage: bifurc vars <verb>

The runtime variables of the session, in memory: what --extract name=<path> on requests send and curl stores,
and what you set. In the default session they are the command line's own: the app never sees them.
Name the session with --session <name> or BIFURC_SESSION.

  list [--reveal]                        KEY value lines; secret-looking names print masked
  set <KEY=VALUE>...                     Set variables; {{KEY}} resolves in the session's requests
  clear [<KEY>...]                       Remove the named variables, or all of them";

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

const SERVICES: &str = "Usage: bifurc services <verb>

Dev servers the engine runs in the background (they outlive this command, end with the engine, and are
stopped as a whole process tree). A definition is saved in the workspace (devservices/); the output of each
is kept in memory, last 5000 lines. Read it with: bifurc logs <service> [--errors] [--since 2m] [--grep x] [--last 50]
(--errors: warn/error lines and stack traces, repeats as `line ×42`). Free a port with: bifurc port free <port> [--kill]

  list                                   Local processes listening on a port (any process, not only these)
  define <name> --cmd <command> [--port <n>] [--cwd <dir>] [--vault <vault>] [--env KEY=VALUE]...
                                         Save a service; --cwd is stored as an absolute path. Do not put secrets
                                         in --env (it is a workspace file): use --vault, whose secrets are
                                         injected like `vault run` (sealed and no terminal: exit 6)
  up [<name>|--all] [--wait] [--timeout 30s]   Start (default: all); --wait blocks until the port listens, exit 4 on
                                         timeout with the last lines of output
  down <name>|--all                      Stop the whole process tree
  restart <name> [--wait] [--timeout 30s]   Stop and start again
  remove <name>                          Stop and delete the definition
  ps                                     One line each: name, state, port, pid, uptime, health. A crashed service
                                         shows `crashed (exit 1) 12s ago` and its last 5 error lines
  show <name>                            The definition; secret-looking env values masked
  --passphrase-stdin                     With up, restart: the vault passphrase from stdin";

const LOGS: &str = "Usage: bifurc logs <service> [--errors] [--since 2m] [--grep x] [--last 50]

The recent output of a dev server (bifurc services), plain lines, the last 50 by default.

  --errors                               Only warn/error lines and stack traces; repeated lines as `line ×42`
  --since <30s|2m|1h>                    Only lines from that long ago
  --grep <text>                          Only lines containing the text (any case)
  --last <n>                             At most the last n lines (default 50)";

const PORT: &str = "Usage: bifurc port free <port> [--kill]

Who holds a TCP port on this machine: `3000 held by node (pid 4242)`, or `free` (exit 0).

  free <port>                            Show the process holding the port
  free <port> --kill                     Stop it with everything it started";

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
        "contract" => CONTRACT,
        "requests" => REQUESTS,
        "health" => HEALTH,
        "flows" => FLOWS,
        "vault" => VAULT,
        "account" => ACCOUNT,
        "services" => SERVICES,
        "logs" => LOGS,
        "port" => PORT,
        "engine" => ENGINE,
        "update" => UPDATE,
        "folders" => FOLDERS,
        "runner" => RUNNER,
        "capture" => CAPTURE,
        "cookies" => COOKIES,
        "session" => SESSION,
        "vars" => VARS,
        "changes" => CHANGES,
        "claim" | "claims" => CLAIM,
        "sync" => SYNC,
        "history" => HISTORY,
        "export" => EXPORT,
        "import" => IMPORT,
        "proxy" => PROXY,
        "settings" => SETTINGS,
        "tls" => TLS,
        "agent" => AGENT,
        "curl" => CURL,
        "wait" => WAIT,
        "setup" => SETUP,
        _ => return None,
    };
    let rows = direct::usage(group);
    Some(if rows.is_empty() { own.to_string() } else { format!("{own}\n\n{rows}") })
}

/// Copy-paste examples of the groups agents use most, one command per line.
pub fn examples(group: &str) -> Option<&'static [&'static str]> {
    Some(match canonical_group(group) {
        "agent" => &[
            "bifurc agent init",
            "bifurc agent init --target both --skill",
            "bifurc agent init --dir ../api-repo --target claude",
        ],
        "curl" => &[
            r#"bifurc curl -X POST {{baseUrl}}/users -d '{"email":"a@b.co"}' --save "Create user" --folder auth"#,
            "bifurc curl -s http://api.localhost/users/1 --select .email",
            "bifurc curl http://api.localhost/health --expect-status 200",
            "bifurc curl -H 'Authorization: Bearer {{token}}' http://api.localhost/me --headers",
        ],
        "requests" => &[
            "bifurc requests list --folder auth --grep user",
            r#"bifurc requests send "Create user" --var email=x@y.co --expect-status 201 --select .id"#,
            r#"bifurc requests send "Get order" --env staging --body --max-body 500"#,
            "bifurc requests send --url http://api.localhost/ping --expect 'status < 300'",
            r#"bifurc requests show "Create user" --curl"#,
        ],
        "capture" => &[
            "bifurc capture list --status 4xx --last 5",
            "bifurc capture show #k3f9 --select .error",
            "bifurc capture list --host api.localhost --path /orders* --since 2m",
            "bifurc capture show #k3f9 --req-body --headers",
            r#"bifurc capture diff #k3f9 --mock "Get order" --shape"#,
        ],
        "contract" => &[
            "bifurc contract check",
            r#"bifurc contract check --mock "Get orders" --ignore '.meta.*'"#,
            "bifurc contract check --openapi openapi.yaml",
            r#"bifurc contract update-mock "Get orders""#,
            "bifurc contract snapshot --folder shop",
        ],
        "mocks" => &[
            r#"bifurc mocks create http://api.localhost/users/1 --status 200 -d '{"id":1}' -H "Content-Type: application/json""#,
            r#"bifurc mocks scenario "Get user" error500"#,
            "bifurc mocks serve --port 4010 --detach",
            r#"bifurc mocks hits "Get user" --last 5"#,
            r#"bifurc mocks create --from-request "Get user" --name "Get user""#,
        ],
        "flows" => &[
            "bifurc flows list",
            r#"bifurc flows validate "Login then list""#,
            r#"bifurc flows run "Login then list" --input user=ann --env dev"#,
            r#"bifurc flows run "Login then list" --fresh-session --verbose"#,
        ],
        "runner" => &[
            "bifurc runner run auth",
            "bifurc runner run auth/admin --bail --env staging",
            "bifurc runner run auth --var email=x@y.co --verbose",
        ],
        "env" => &[
            "bifurc env list",
            "bifurc env use dev",
            "bifurc env show",
            "bifurc env set dev baseUrl=http://api.localhost token=abc",
            r#"bifurc env auth dev --login "Login as admin""#,
        ],
        "cookies" => &[
            "bifurc cookies list",
            "bifurc cookies list --domain api.localhost",
            "bifurc cookies clear --domain api.localhost",
            "bifurc cookies clear",
        ],
        "session" => &[
            "bifurc session show --session frontend",
            "bifurc session reset --session frontend",
            "bifurc session show --json",
        ],
        "vars" => &[
            "bifurc vars list --session frontend",
            "bifurc vars set userId=7 --session frontend",
            "bifurc vars clear userId",
        ],
        "changes" => {
            &["bifurc changes --session frontend", "bifurc changes --since 10m", "bifurc changes --mine --json"]
        }
        "claim" | "claims" => &[
            r#"bifurc claim mock "Get orders" --session backend"#,
            r#"bifurc claim --release mock "Get orders" --session backend"#,
            r#"bifurc claim request "Create user" --json"#,
        ],
        "services" => &[
            r#"bifurc services define api --cmd "npm run dev" --port 3000 --cwd ./api --vault dev"#,
            "bifurc services up --all --wait --timeout 60s",
            "bifurc services ps",
            "bifurc services down api",
        ],
        "wait" => &[
            "bifurc wait http://api.localhost/health --timeout 30s",
            "bifurc wait api-health --status 200",
            "bifurc wait --port 3000 --timeout 1m",
            "bifurc wait --capture --method POST --path /orders* --timeout 2m",
        ],
        _ => return None,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn words(text: &str) -> Vec<&str> {
        text.split(|c: char| !c.is_alphanumeric() && c != '-').collect()
    }

    /// A `bifurc …` command line parses, names a verb its group's help has, and only flags that help names.
    pub(crate) fn check_command(line: &str) {
        let argv = crate::curl::shell_words(line);
        assert_eq!(argv[0], "bifurc", "{line}");
        let group = argv[1].as_str();
        let Some(help) = group_usage(group) else { return assert_eq!(group, "status", "{line}") };
        let parsed = crate::args::parse_args(&argv[1..]).unwrap_or_else(|e| panic!("{line}: {e}"));
        let verb = parsed.positional(1).unwrap_or_default();
        assert!(group == "curl" || group == "wait" || help.contains(verb), "{line}: verb {verb}");
        for flag in parsed.names().into_iter().filter(|f| !["json", "help"].contains(f)) {
            let aliased = ["header", "data", "method"].contains(&flag);
            assert!(aliased || help.contains(&format!("--{flag}")), "{line}: --{flag}");
        }
    }

    #[test]
    fn the_short_overview_is_at_most_30_lines_and_names_the_commands_agents_use() {
        assert!(OVERVIEW.lines().count() <= 30, "{} lines", OVERVIEW.lines().count());
        for group in [
            "agent", "curl", "requests", "capture", "mocks", "flows", "runner", "env", "cookies", "session", "wait",
            "status",
        ] {
            assert!(words(OVERVIEW).contains(&group), "{group}");
        }
    }

    #[test]
    fn every_example_is_a_known_group_and_verb_with_flags_its_help_names() {
        for group in [
            "agent", "curl", "requests", "capture", "mocks", "contract", "flows", "runner", "env", "cookies",
            "session", "vars", "changes", "claim", "wait", "services",
        ] {
            let list = examples(group).unwrap_or_else(|| panic!("{group} has no examples"));
            assert!((3..=5).contains(&list.len()), "{group}");
            for line in list {
                assert_eq!(crate::curl::shell_words(line)[1], group, "{line}");
                check_command(line);
            }
        }
        assert_eq!(examples("status"), None);
    }

    #[test]
    fn every_group_of_the_overview_has_its_own_help() {
        let mut groups = vec!["health", "services"];
        groups.extend(direct::verbs().iter().map(|verb| verb.words[0]).filter(|group| *group != "update"));
        for group in groups {
            assert!(USAGE.contains(&format!("\n  {group} ")), "{group} is not in the overview");
            assert!(words(OVERVIEW).contains(&group), "{group} is not in the short overview");
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
