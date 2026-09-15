# 09 — Phase 8: CLI

**Goal:** a command-line client that can start, stop and inspect engines, and (scope permitting) drive
the full engine surface — including against a remote Docker engine.

**Effort:** **~3 weeks.** **Depends on:** P5. **Parallel with:** P7, P9.

> ## Scope set by D7 — v1 is lifecycle + read-only
>
> *"**(a) for v1**, then expand based on real usage."*
>
> ```
> IN SCOPE (v1)
>   engine start | stop | status | logs
>   remote add | list | use | test
>   config get
>   workspace list | use          env list | use
>   mocks list   requests list    rules list    mappings list
>   tls generate | status | export
>   logs tail [-f]                audit list
>
> DEFERRED (v2+, on evidence of real usage)
>   every mutating command:  add / rm / update / send
>   import · tls trust · requests send · config save
>   collection runner · gRPC / GraphQL / SOAP execute
> ```
>
> The deferred set is **not wasted work** — it is the same client calls against a proven protocol, added
> when there is demand. Shipping the read surface first also validates the protocol, the transports and
> the auth model with a much smaller command tree.

---

## Preconditions

- P5 gate green: `packages/client` works over `stdio` and `ws`.
- D7 answered — ✅ (a), lifecycle + read-only.

---

## Work item 1 — Command tree

Marked **[v1]** or **[deferred]** per D7.

```
bifurc engine start [--data-dir <path>] [--port <n>] [--detach]     [v1]
bifurc engine stop  [--all]                                        [v1]
bifurc engine status [--json]                                      [v1]
bifurc engine logs  [-f] [--level <lvl>]                           [v1]

bifurc remote add <name> <url> [--token <t>]                       [v1]
bifurc remote list                                                 [v1]
bifurc remote use <name>                                           [v1]
bifurc remote test <name>            # handshake + version check   [v1]

bifurc config get [--json]                                         [v1]
bifurc config save <file>                                          [deferred]

bifurc workspace list | use <id>                                   [v1]
bifurc env list | use <id>                                         [v1]

bifurc mocks list                                                  [v1]
bifurc requests list                                               [v1]
bifurc rules list                                                  [v1]
bifurc mappings list                                               [v1]
bifurc mocks add | rm · rules add | rm · mappings add | rm         [deferred]
bifurc requests send <id>                                          [deferred]

bifurc export --kind <k> --format <f> -o <file>                    [deferred]
bifurc import <file> --kind <k> --format <f> [--strategy …]        [deferred]

bifurc tls generate | status | export -o <file>                    [v1]
bifurc tls trust                                                   [deferred]

bifurc logs tail [-f] [--kind <k>]                                 [v1]
bifurc audit list                                                  [v1]
bifurc audit export                                                [deferred]
```

**Note:** `export` moves to deferred under D7, which means the CLI is no longer a consumer of the blob
layer in v1. The blob work in P3 is still required — the **web UI** needs it, and it is what replaces the
native dialogs. The CLI picks it up when export/import are added in v2.

**Naming rule:** the CLI command tree mirrors the protocol namespaces exactly (`config.get` →
`bifurc config get`). No separate vocabulary — it makes both easier to learn and to document.

---

## Work item 2 — Transport selection

```
1. If --remote <name> given        → ws transport to that engine
2. Else if --engine <url> given    → ws transport
3. Else if a local engine is running (socket/port probe)  → attach to it
4. Else                            → spawn a local engine over stdio, run the command, exit
```

**Case 4 is what makes the CLI feel like a normal CLI.** `bifurc mocks list` should just work without the
user having to start anything. Spawn-on-demand over `stdio`, run, shut down.

**Case 3 matters for correctness.** If a desktop shell already has an engine running, the CLI should
attach rather than start a second one. Two engines on one data dir is a corruption risk.

---

## Work item 3 — Output formats

Every command supports:
- **Human** (default) — aligned tables, colour, truncated long values.
- **`--json`** — the raw protocol result. **This is the contract.** Scripts depend on it, so it must not
  be prettified or reshaped.

```bash
$ bifurc mocks list
ID          METHOD  PATTERN              ENABLED
mk_7f3a21   GET     /api/users/*         yes
mk_7f3a22   POST    /api/orders          yes

$ bifurc mocks list --json
{"mocks":[{"id":"mk_7f3a21","method":"GET","urlPattern":"/api/users/*","enabled":true}]}
```

Exit codes: `0` success, `1` command error, `2` usage error, `3` connection/auth error. Scripts need
these to be stable.

---

## Work item 4 — `engine logs -f` and event streaming

The CLI is the best consumer of the event/replay work from P4, because a terminal can display volume the
GUI cannot.

- `-f` subscribes to `event.log.entry` and streams.
- Use the `seq`/replay mechanism on reconnect — a `tail -f` that silently stops after a network blip is
  worse than one that errors.
- Respect the coalescing policy; a CLI showing 10k req/s does not need per-request lines. Add
  `--rate` to summarise.

---

## Work item 5 — The blob layer in a CLI

Imports and exports are natural CLI operations and must use the blob layer, not paths.

```bash
$ bifurc export --kind mocks --format postman -o ./mocks.json
# → export.create → blob.read → write ./mocks.json locally

$ bifurc import ./postman-collection.json --kind requests --format postman
# → read locally → blob.put → import.preflight → (prompt on collisions) → import.commit → blob.release
```

Note this is exactly the same three-step flow as the GUI. **The blob design pays off here** — a CLI has
no dialogs at all, and under a path-based design it could not import a file into a remote Docker engine.

Collision handling: `--strategy` for non-interactive use; an interactive prompt when a TTY is present.

---

## Work item 6 — TLS and the CA in a CLI

The CLI is the ideal place to expose the certificate workflow, and it can do what the browser cannot.

```bash
$ bifurc tls generate
Generated CA  fingerprint: sha256:ab12cd34…
Saved to: <data-dir>/ca-cert.pem

$ bifurc tls export -o ./bifurc-ca.pem
$ bifurc tls status
Engine:  generated, fingerprint sha256:ab12cd34…
Local:   not trusted

$ bifurc tls trust        # CLI performs the OS install (same commands as the desktop shell)
```

`bifurc tls trust` uses the platform commands from `File_Ops_Protocol.md` §6.2. On Linux it should
prefer `trust anchor` and fall back to `pkexec`. Print the exact command before running it so users can
do it manually if they prefer.

**The CLI should also warn about Firefox** (`File_Ops_Protocol.md` §6.3) — it is a natural place to
detect it and explain.

---

## Work item 7 — Distribution

- Single binary, same mechanism as the engine (P10).
- `npm i -g` as a secondary path for Node users.
- Homebrew / Scoop / winget are nice-to-have, not v1.
- Version-lockstep with the engine (D9), and the CLI must refuse to talk to an incompatible engine with
  a clear message rather than failing obscurely.

---

## How to start — the first three things

1. **`bifurc engine start|stop|status|logs`.** These validate the stdio transport, the spawn/supervise
   logic and the event stream, with no domain logic at all. Highest signal per line of code.
2. **`bifurc remote add|list|test`.** Validates the `ws` transport and the handshake/version check.
3. **Then `config get --json` and `mocks list`.** First real domain commands; proves the client shims work
   from a non-Electron consumer.

---

## Acceptance criteria

- [ ] Engine lifecycle commands work: start, stop, status, logs `-f`.
- [ ] Remote engine management works against a Docker engine.
- [ ] `--json` output is the raw protocol result, stable and script-safe.
- [ ] Exit codes are stable and documented.
- [ ] Attaches to an existing engine rather than starting a second one on the same data dir.
- [ ] Spawn-on-demand works with no running engine.
- [ ] Import/export use the blob layer and work against a remote engine.
- [ ] `tls trust` installs the CA and verifies the fingerprint; warns about Firefox.
- [ ] `engine logs -f` survives a reconnect without silently stopping.
- [ ] Incompatible engine version produces a clear error.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Scope creep toward full parity (D7) | **High** | Ship lifecycle + read-only first; expand on evidence |
| CLI starts a second engine on a live data dir | Medium | Explicit attach-before-spawn probe; test it |
| `--json` output drifts from the protocol | Medium | Emit the raw result; never reshape |
| `tls trust` silently fails on Linux | Medium | Print the command, check the exit code, verify after |
| `tail -f` stops silently after a blip | Medium | Replay via `seq`; surface disconnects on stderr |
