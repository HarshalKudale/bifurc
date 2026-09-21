/**
 * P14 work item 7 — how a workspace-scoped command **drives** a runtime's servers, once
 * `runtimeRegistry.ts` has resolved which runtime that is.
 *
 * Split from the registry deliberately: the registry answers *"whose server is this?"* and this answers
 * *"and what may a command do to it?"*. They are different questions with different failure modes, and the
 * second one is where the subtlety lives.
 *
 * ## The rule: the host owns a runtime's lifecycle, so a command never allocates a port
 *
 * A runtime is created by the host with ports the host allocated (D16, via `portAllocator.ts`, which knows
 * what every other runtime on the machine already holds and which ports the RPC endpoint is about to
 * want). So for a runtime-backed workspace:
 *
 * - **`start` does not bind.** If the server is running, `{ ok: true }` — it is already serving, which is
 *   what the caller asked for. If it is *not* running, `{ ok: false }`, because the only way to honour
 *   `start` would be to pick a port, and a port picked here bypasses the allocator's `reserved` set. That
 *   is the same class of mistake D16 names for `checkPortInUse`.
 * - **`stop` stops.** Genuinely — a user may want a workspace to stop serving while the host keeps the
 *   others up. That is the D10 behaviour.
 * - **`restart` re-binds only a port it has *observed*.** See below.
 *
 * ## Why `restart` keys off `wasRunning` rather than a port sentinel
 *
 * The obvious guard is "re-bind `getPort()` if it is greater than zero" — and **it does not transfer
 * between the two servers.** `proxy/server.ts` seeds `let currentPort = 0`, so a never-started proxy is
 * detectable. `webhookServer.ts` seeds `let currentPort = DEFAULT_WEBHOOK_PORT` (**9101**), so a
 * never-started webhook reports a *plausible* port rather than an obvious sentinel, and a guard written
 * for the proxy would happily re-bind 9101 — which on a two-workspace host may well be **another
 * workspace's** port. That is precisely the bug class this work item exists to remove.
 *
 * So the rule is sentinel-independent: **re-bind only if we observed the server running.** If it was
 * running, its `getPort()` was really bound and is therefore real; if it was not, we refuse rather than
 * guess. `currentPort` is retained across `stop()` in both servers (`proxy/server.ts` records this as
 * pre-existing behaviour), which is what makes the observed port still valid after the stop.
 *
 * ## Structural types, not imports
 *
 * These interfaces are structural so this module needs no *value* import from `proxy/`. `workspaceRuntime.ts`
 * already imports `proxy/server.ts`, so a value edge back would close a module cycle — the same reasoning
 * `runtimeRegistry.ts` records for its type-only import.
 */

/** The lifecycle subset shared by the proxy and the webhook receiver. */
export interface ManagedServer {
    isRunning(): boolean;
    getPort(): number;
    start(port: number): void;
    stop(): void;
}

/**
 * Ensure a runtime-backed server is serving.
 *
 * - **Already running** → `{ ok: true }`. It is serving, which is what the caller asked for; this is
 *   idempotent rather than a re-bind, because the host's `start()` already happened.
 * - **Not running, and the caller named a port** → bind that port. An explicit port is the caller's
 *   choice and honouring it is the whole point of the parameter existing; ignoring it would be a schema
 *   the implementation contradicts — the same defect `webhookCommands.ts`'s header records for the
 *   pre-P14 `webhookServer.start`.
 * - **Not running, no port** → `{ ok: false }`. **This is the case that matters.** The only way to honour
 *   `start` would be to pick a port, and a port picked here bypasses the allocator's `reserved` set — the
 *   same class of mistake D16 names for `checkPortInUse`. `ServerLifecycleResult` is `{ ok }` only, so
 *   `false` is the whole failure vocabulary available, and an honest failure beats a success that quietly
 *   took a port the allocator was protecting.
 */
export function startManagedServer(server: ManagedServer, requestedPort?: number): { ok: boolean } {
    if (server.isRunning()) return { ok: true };
    if (requestedPort === undefined) return { ok: false };
    server.start(requestedPort);
    return { ok: true };
}

/**
 * Stop then re-start a runtime-backed server on the port it was actually serving.
 *
 * Returns `{ ok: false }` when the server was **not** running, for the reason in the header: there is no
 * observed port to re-bind, and inventing one is the allocator's job.
 */
export function restartManagedServer(server: ManagedServer): { ok: boolean } {
    const wasRunning = server.isRunning();
    const port = server.getPort();
    server.stop();
    if (!wasRunning) return { ok: false };
    server.start(port);
    return { ok: true };
}
