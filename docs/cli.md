# CLI

The `bifurc` command runs requests, mocks, flows and the proxies of a workspace, and starts an engine when none runs. The user documentation with examples is on [bifurc.app/docs](https://bifurc.app/docs).

Run `bifurc help <group>` for the verbs of a group, `bifurc help <group> --examples` for examples and `bifurc help --all` for everything. **Vaults are in beta** (`bifurc vault ...`).

```text
Usage: bifurc <group> <verb> [arguments] [options]

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
  bifurc help <group> --examples  Examples
  bifurc help --all               Every group and verb
Exit codes: 0 ok, 1 failed, 2 usage, 3 not found, 4 network, 5 no engine, 6 needs a person, 7 policy
```
