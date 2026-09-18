"""One-off generator for packages/engine/src/transport/auth/scopes.ts.

Kept in the repo rather than run from a shell heredoc: the table is 93 lines and the point is that it
keys `COMMANDS` exactly, which a generator can guarantee and hand-typing cannot. Delete this script
once the table is stable if you prefer — the table itself is asserted total at import time by
`assertScopesAreTotal()`, so the file is self-defending from here on.
"""
import json
import subprocess

out = subprocess.run(
    ["node", "-e", "console.log(JSON.stringify(Object.keys(require('./packages/protocol/dist/index.cjs').COMMANDS)))"],
    capture_output=True, text=True, check=True,
)
commands = json.loads(out.stdout)

ASSIGN = {
    "read": """
        app.checkUpdate
        application.list application.getState application.getAllStates application.getLogs
        application.checkPort
        audit.list audit.diff
        blob.stat blob.read
        config.get
        entity.load entity.list
        export.formats
        git.diff git.history
        graphql.introspect
        grpc.mockServerStatus grpc.reflect
        healthbar.getServices healthbar.checkUrl
        history.list history.diff
        proxy.status
        runner.getHistory runner.listFolderIds
        server.status
        services.discover
        soap.fetchWsdl
        sync.getState sync.getEntityStatus
        tls.certStatus
        webhookServer.status
    """,
    "write": """
        application.save
        audit.export
        blob.put blob.release
        capture.shareJson
        config.save
        entity.create entity.update entity.delete entity.setEnabled entity.publish entity.restore
        env.setActive
        export.create
        folder.add folder.rename folder.move folder.delete folder.publish
        git.discard git.sync
        healthbar.saveServices
        import.preflight import.commit
        runner.saveReport runner.exportReport runner.saveConfig runner.loadConfig
        sync.setRemote sync.disconnect sync.push sync.pull sync.setAutoSync
        tls.exportCert tls.importCert tls.importKey
        webhook.registerActive webhook.unregisterActive
        workspace.add workspace.rename workspace.delete workspace.setActive
    """,
    "execute": """
        graphql.execute
        grpc.execute grpc.startMockServer grpc.stopMockServer
        request.replay
        script.execute
        soap.execute
    """,
    "admin": """
        application.delete application.start application.stop application.killPort
        server.start server.stop server.restart
        tls.generate tls.removeCert
        webhookServer.start webhookServer.stop
    """,
}

flat = {}
for scope, blob in ASSIGN.items():
    for cmd in blob.split():
        if cmd in flat:
            raise SystemExit("duplicate assignment for %s" % cmd)
        flat[cmd] = scope

missing = [c for c in commands if c not in flat]
extra = [c for c in flat if c not in commands]
if missing or extra:
    raise SystemExit("partition is not total: missing=%s extra=%s" % (missing, extra))

HEADER = '''/**
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

export type Scope = "read" | "write" | "execute" | "admin";

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
'''

FOOTER = '''};

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
'''

body = "".join('  "%s": "%s",\n' % (cmd, flat[cmd]) for cmd in commands)
path = "packages/engine/src/transport/auth/scopes.ts"
with open(path, "w", encoding="utf-8", newline="\n") as fh:
    fh.write(HEADER + body + FOOTER)
print("wrote %s: %d commands" % (path, len(commands)))
