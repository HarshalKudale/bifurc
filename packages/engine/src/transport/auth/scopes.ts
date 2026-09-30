/**
 * P4 work item 4 — the per-session authorisation scope.
 *
 * `plan/05`'s table is the intent; **this file is the resolution**, because the plan's rules overlap
 * and two of them do not match the real protocol:
 *
 * - The plan says `applications.*`; the namespace is **`application.*`** (singular).
 * - The plan says `runner.*` -> `execute`, but **no runner-execution command exists** — the six
 *   `runner.*` commands are `getHistory`/`listFolderIds` (read) and
 *   `saveReport`/`exportReport`/`saveConfig`/`loadConfig` (write). Collection execution must happen
 *   outside the registry.
 * - `*.list` -> read and `applications.*` -> admin overlap on `application.list`. Resolved by the
 *   narrower rule: **pure reads inside an admin namespace are reads**, so a client can still see what
 *   it is allowed to touch while `start`/`stop`/`delete` stay admin-only.
 *
 * ## Why an explicit table and not pattern rules
 *
 * This gates `tls.*`, `server.*` and every `*.execute`. A derived-by-pattern table with overlapping
 * rules is exactly the thing that produces a silent mis-gate — a command that reads as "obviously
 * read" to whoever wrote the regex and as "execute" to whoever reads the plan. 93 explicit lines are
 * auditable; a clever rule is not. Same reasoning as `COMMAND_FIXTURES`: an explicit table plus a
 * totality assertion beats a rule that is *probably* right.
 *
 * ## Fail closed, and loudly
 *
 * `assertScopesAreTotal()` throws **at import time** if the table and `COMMANDS` disagree in either
 * direction. So adding a command to the protocol cannot produce an accidentally ungated command: the
 * engine refuses to load until someone assigns it a scope. And `requiredScopeFor()` returns
 * `undefined` for an unknown command rather than defaulting — `commandIsAllowed()` then returns
 * **false**. Unknown means denied, never allowed.
 *
 * ## No implicit hierarchy
 *
 * `admin` does **not** imply `read`. Scopes are an explicit set, so "which commands can this session
 * reach?" is a lookup rather than a derivation — which is the question a security review actually
 * asks. The presets below exist so callers do not hand-roll the sets.
 */
import { COMMANDS, type CommandAction } from "@bifurc/protocol";
import type { Scope } from "../../kernel/registry";

/**
 * Re-exported, **not declared here**. `Scope` is the vocabulary a handler reads off
 * `CommandContext.session`, so it is declared beside `CommandContext` in `commands/registry.ts`:
 * `transport/` imports `commands/` and never the reverse, and a type both layers share has to live on
 * the lower side or the dependency inverts. Still re-exported from this module so `./scopes` remains
 * the single import a consumer needs for both the vocabulary and the table.
 */
export type { Scope };

/** Ordered least- to most-privileged, for display and stable iteration. Not a hierarchy. */
export const SCOPES = ["read", "write", "execute", "admin"] as const satisfies readonly Scope[];

/**
 * The table. Every command in `@bifurc/protocol`'s `COMMANDS`, assigned exactly one scope.
 *
 * `read`    — observes; changes nothing. Safe to expose broadly.
 * `write`   — mutates engine or workspace state, and writes files the user asked for.
 * `execute` — runs something the engine does not control: a script, a GraphQL/gRPC/SOAP call, a
 *             replayed request. The class `plan/05` singles out for rate limiting (work item 5).
 * `admin`   — TLS trust, server and application lifecycle. Can affect the whole machine rather than
 *             just the workspace, which is why generating a CA and untrusting one live here.
 */
export const SCOPE_BY_COMMAND: Readonly<Record<CommandAction, Scope>> = {
  "entity.create": "write",
  "entity.update": "write",
  "entity.delete": "write",
  "entity.load": "read",
  "entity.list": "read",
  "entity.setEnabled": "write",
  "folder.add": "write",
  "folder.rename": "write",
  "folder.move": "write",
  "folder.delete": "write",
  "folder.publish": "write",
  "config.get": "read",
  "config.save": "write",
  "workspace.add": "write",
  "workspace.rename": "write",
  "workspace.delete": "write",
  "workspace.setActive": "write",
  "env.setActive": "write",
  // `plan/19` WI 5, item 4 — the resolution *display* path. `read`: it observes and writes nothing. The
  // `{{random.*}}` values it mints are not a side effect (nothing is stored), but they are why a caller
  // must not treat the result as stable — see `config.ts`.
  "env.resolveText": "read",
  "server.status": "read",
  "server.start": "admin",
  "server.stop": "admin",
  "server.restart": "admin",
  "proxy.status": "read",
  "services.discover": "read",
  "graphql.introspect": "read",
  "graphql.execute": "execute",
  "soap.fetchWsdl": "read",
  "soap.execute": "execute",
  "grpc.execute": "execute",
  "grpc.reflect": "read",
  "application.list": "read",
  "application.save": "write",
  "application.delete": "admin",
  "application.start": "admin",
  "application.stop": "admin",
  "application.getState": "read",
  "application.getAllStates": "read",
  "application.getLogs": "read",
  "application.checkPort": "read",
  "application.killPort": "admin",
  "webhook.registerActive": "write",
  "webhook.unregisterActive": "write",
  "webhookServer.start": "admin",
  "webhookServer.stop": "admin",
  "webhookServer.status": "read",
  "runner.saveReport": "write",
  "runner.exportReport": "write",
  "runner.getHistory": "read",
  "runner.saveConfig": "write",
  "runner.loadConfig": "write",
  "runner.listFolderIds": "read",
  "sync.setRemote": "write",
  "sync.disconnect": "write",
  "sync.push": "write",
  "sync.pull": "write",
  "sync.getState": "read",
  "sync.setAutoSync": "write",
  "sync.getEntityStatus": "read",
  "git.diff": "read",
  "git.discard": "write",
  "git.sync": "write",
  "git.history": "read",
  "entity.publish": "write",
  "entity.restore": "write",
  "audit.list": "read",
  "audit.diff": "read",
  "audit.export": "write",
  "history.list": "read",
  "history.diff": "read",
  "tls.generate": "admin",
  "tls.certStatus": "read",
  "tls.removeCert": "admin",
  "tls.exportCert": "write",
  "tls.importCert": "write",
  "tls.importKey": "write",
  "export.formats": "read",
  "export.create": "write",
  "import.preflight": "write",
  "import.commit": "write",
  "blob.put": "write",
  "blob.stat": "read",
  "blob.read": "read",
  "blob.release": "write",
  "script.execute": "execute",
  "request.replay": "execute",
  // `plan/19` WI 5 — the cURL parser. `read` rather than `execute`, and the contrast with the line above is
  // the point: parsing a string touches nothing outside the process and reaches no upstream, so it is the
  // "observes; changes nothing" class. A `read`-scoped session may import a cURL command.
  "request.parseCurl": "read",
  "healthbar.getServices": "read",
  "healthbar.saveServices": "write",
  "healthbar.checkUrl": "read",
  "capture.shareJson": "write",
  // `plan/19` WI 5, item 2 — capture → entity construction. **`read`, and the contrast with the line above
  // is the point:** `capture.shareJson` stages a blob, so it writes. These two build a *payload* and touch
  // nothing — the storing is `entity.create`'s, which is `write`. A `read`-scoped session may prepare an
  // entity it is not allowed to save, which is the correct amount of authority for a pure function.
  "capture.buildRequest": "read",
  "capture.buildMock": "read",
  // The four saved-entity counterparts (`packages/protocol/src/commands/saved.ts`). `execute` for the
  // same reason as the commands they delegate to: each one's effect is an **outbound request the engine
  // does not control**, which is the class this scope exists to single out for rate limiting. Loading
  // the entity is a read, but the command as a whole is not, and a scope is per command.
  //
  // The `execute` choice is also what keeps the pair consistent rather than privileged: a counterpart
  // delegates through `registry.invoke()`, which does **not** re-check scopes (the check lives in the
  // transport decorator), so a counterpart carrying a *weaker* scope than its target would be a
  // widening — a `read`-scoped session could reach `graphql.execute` by the saved-entity door. Same
  // scope, no widening.
  // `request.send` replaced `request.replaySaved` (2026-09-30, `plan/19` D24/D27). Same `execute` class
  // as the `request.replay` it invokes — the pipeline resolves and runs scripts, and a `read` scope for
  // that would be a widening, not a reclassification.
  "request.send": "execute",
  // `graphql.send` replaced `graphql.executeSaved` (2026-09-30, `plan/19` WI 3's second protocol). Same
  // `execute` class as the `graphql.execute` it invokes — it resolves the request now, and a `read` scope
  // for that would be a widening rather than a reclassification.
  "graphql.send": "execute",
  // `soap.send` replaced `soap.executeSaved` (2026-09-30, `plan/19` WI 3's third protocol). Same `execute`
  // class as the `soap.execute` it invokes — it resolves the request now, and a `read` scope would be a
  // widening rather than a reclassification.
  "soap.send": "execute",
  // `grpc.send` replaced `grpc.executeSaved` (2026-09-30, `plan/19` WI 3's fourth and last protocol). Same
  // `execute` class as the `grpc.execute` it invokes.
  "grpc.send": "execute",
  // The run's own commands (`plan/19` WI 4). **Two different scopes, and the split is the interesting part:**
  // starting and cancelling a run both *make something happen*, so they are `execute`; `runner.status` only
  // reads the engine's own run state, so it is `read` — which is what lets a reconnecting client ask what
  // happened (D29) without also being granted the right to start or stop anything.
  "runner.start": "execute",
  "runner.cancel": "execute",
  "runner.status": "read",
  // The socket registry (`plan/19` WI 6), and the split is `runner.*`'s one namespace over —
  // `Protocol_Module_Contract.md` §4 requires it to be *chosen* rather than inherited:
  //
  //   `ws.connect` / `ws.disconnect` / `ws.send` are `execute`, because each reaches a peer the engine does
  //   not control — opening a connection, writing a frame to it. `ws.status` / `ws.list` are `read`, which is
  //   what lets a reconnecting client ask *what happened to my socket* (D29) without also being granted the
  //   right to open or close one.
  //
  //   `ws.clearMessages` is `write`: it mutates engine state (the retained log) and reaches no upstream, so it
  //   is neither of the two classes above. Note it is deliberately *not* `read` — a `read`-scoped session that
  //   could erase the log a `status` call reports would make `ws.status` unreliable for everyone else.
  "ws.connect": "execute",
  "ws.disconnect": "execute",
  "ws.send": "execute",
  "ws.status": "read",
  "ws.list": "read",
  "ws.clearMessages": "write",
  // `plan/19` WI 6 — the read that replaced `App.tsx`'s `mergeEnvVars`. `read`: it merges two entity lists the
  // caller could have read itself and stores nothing. It is **not** a resolver — `env.resolveText` is — so it
  // is not `execute`, and the distinction matters because this one returns a list to choose from.
  "env.getMerged": "read",
};

/**
 * The scope a command requires, or `undefined` if the command is not in the table.
 *
 * `undefined` means **deny** — see `commandIsAllowed()`. It is not a "no scope needed" sentinel.
 */
export function requiredScopeFor(command: string): Scope | undefined {
  return (SCOPE_BY_COMMAND as Record<string, Scope | undefined>)[command];
}

/**
 * Throw unless the table keys `COMMANDS` exactly. Called at module scope below.
 *
 * Both directions matter. A missing key is a command that would be denied by accident; a surplus key
 * is a scope assigned to a command that no longer exists, which reads like coverage while covering
 * nothing.
 */
export function assertScopesAreTotal(commands: readonly string[] = Object.keys(COMMANDS)): void {
  const missing = commands.filter((c) => !(c in SCOPE_BY_COMMAND));
  const surplus = Object.keys(SCOPE_BY_COMMAND).filter((c) => !commands.includes(c));

  if (missing.length > 0 || surplus.length > 0) {
    throw new Error(
      "The authorisation scope table and @bifurc/protocol's COMMANDS disagree. " +
        (missing.length > 0 ? "Unscoped (would be DENIED): " + missing.join(", ") + ". " : "") +
        (surplus.length > 0 ? "Scoped but not a command: " + surplus.join(", ") + ". " : "") +
        "Assign a scope in packages/engine/src/transport/auth/scopes.ts — an unscoped command is " +
        "denied, so this must be fixed before the engine can serve it.",
    );
  }
}

assertScopesAreTotal();

/** Every scope. What the shell's own session gets: it is the user. */
export const SHELL_SCOPES: ReadonlySet<Scope> = new Set(SCOPES);

/**
 * Read + write, nothing that executes or administers.
 *
 * `ALLOWED_ACTIONS`' `{mock:add, request:add, folder:add, config:get}` generalised: three additive
 * writes and one read. `plan/05` calls it a "write-only, non-destructive scope" — it needs `read` as
 * well as `write`, because `config:get` is a read and the extension must be able to check state
 * before adding to it.
 */
export const COMPANION_SCOPES: ReadonlySet<Scope> = new Set<Scope>(["read", "write"]);

/** A session that may only look. */
export const READ_ONLY_SCOPES: ReadonlySet<Scope> = new Set<Scope>(["read"]);

/**
 * Is `command` reachable with `granted`?
 *
 * **Fails closed.** A command with no entry in the table is denied, not allowed — the table is
 * asserted total at import, so this branch should be unreachable in production, and if it is ever
 * reached the safe answer is the one that returns.
 */
export function commandIsAllowed(granted: ReadonlySet<Scope>, command: string): boolean {
  const required = requiredScopeFor(command);
  if (required === undefined) return false;
  return granted.has(required);
}
