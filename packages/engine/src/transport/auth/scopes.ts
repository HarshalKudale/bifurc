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
import type { Scope } from "../../commands/registry";

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
  "grpc.mockServerStatus": "read",
  "grpc.startMockServer": "execute",
  "grpc.stopMockServer": "execute",
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
  "healthbar.getServices": "read",
  "healthbar.saveServices": "write",
  "healthbar.checkUrl": "read",
  "app.checkUpdate": "read",
  "capture.shareJson": "write",
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
