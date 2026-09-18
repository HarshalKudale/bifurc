/**
 * Retry policy — P5 work item 5, and the one place the protocol's own answer is not enough.
 *
 * ## The problem
 *
 * `@bifurc/protocol`'s `isRetryable(code)` is **code-based only**:
 *
 * ```
 * RETRYABLE_CODES = { UPSTREAM_FAILED, TIMEOUT, ENGINE_ERROR }
 * ```
 *
 * That is correct as far as it goes, but it cannot see *which command* failed, and the two
 * questions are independent:
 *
 * - A `TIMEOUT` on `server.status` is safe to retry — the call is a read.
 * - A `TIMEOUT` on `entity.create` is **not** safe to retry. A timeout means *we do not know
 *   whether the mutation landed*. Retrying duplicates the entity, and the duplicate is not
 *   hypothetical: the engine has already `unshift`ed the new entity and won routing for it, so
 *   the user ends up with two mocks, the second shadowing the first.
 *
 * So the effective policy is `isRetryable(code) && isIdempotent(command)`, and this module owns
 * the second half. It is the *conjunction* that is load-bearing: either half alone is wrong,
 * and each is wrong in a different direction.
 *
 * ## Why the sets are written out rather than derived
 *
 * The sets are exhaustive and asserted to **partition** `COMMANDS` (`assertRetryPolicyIsTotal()`).
 * A derived rule ("anything starting with `get`/`list`/`status` is a read") would be shorter and
 * would silently mis-classify the next command someone adds — `application.getLogs` reads, but
 * `application.killPort` does not, and both match a `get`/`kill` prefix heuristic only by luck.
 *
 * The failure mode of getting this wrong is asymmetric, which is why the *default* for an
 * unlisted command is **not retryable**:
 *
 * - wrongly retryable → a duplicated mutation, visible only to the user, some time later
 * - wrongly non-retryable → a request the caller retries by hand, or a slightly worse UX
 *
 * The partition assertion means there is no third option ("unclassified"), so a new command
 * forces the decision at the point it is added rather than at the point it misbehaves.
 */

import { COMMANDS, EngineError, isRetryable, type ErrorCodeValue } from "@bifurc/protocol";

/**
 * Commands that may be replayed after a retryable failure **without changing the outcome**.
 *
 * Reads, and writes whose effect is a function of the arguments alone (a set, a rename, a
 * delete, a start/stop). Note what is deliberately *absent*: everything that allocates.
 */
export const IDEMPOTENT_COMMANDS: ReadonlySet<string> = new Set([
  // ── reads ────────────────────────────────────────────────────────────────
  "entity.load",
  "entity.list",
  "config.get",
  "server.status",
  "proxy.status",
  "services.discover",
  "graphql.introspect",
  "soap.fetchWsdl",
  "grpc.reflect",
  "grpc.mockServerStatus",
  "application.list",
  "application.getState",
  "application.getAllStates",
  "application.getLogs",
  "application.checkPort",
  "webhookServer.status",
  "runner.getHistory",
  "runner.loadConfig",
  "runner.listFolderIds",
  "sync.getState",
  "sync.getEntityStatus",
  "git.diff",
  "git.history",
  "audit.list",
  "audit.diff",
  "history.list",
  "history.diff",
  "tls.certStatus",
  "export.formats",
  "import.preflight",
  "blob.stat",
  "blob.read",
  "app.checkUpdate",
  "healthbar.getServices",
  "healthbar.checkUrl",

  // ── writes that are a function of their arguments alone ──────────────────
  "entity.update",
  "entity.delete",
  "entity.setEnabled",
  "config.save",
  "folder.rename",
  "folder.move",
  "folder.delete",
  "folder.publish",
  "workspace.rename",
  "workspace.delete",
  "workspace.setActive",
  "env.setActive",
  "server.start",
  "server.stop",
  "server.restart",
  "webhook.registerActive",
  "webhook.unregisterActive",
  "webhookServer.start",
  "webhookServer.stop",
  "sync.disconnect",
  "sync.setAutoSync",
  "git.discard",
  "tls.removeCert",
  "tls.importCert",
  "tls.importKey",
  "blob.release",
]);

/**
 * Commands that must **never** be auto-retried, with the reason recorded next to each.
 *
 * Most allocate an entity or commit content, so a retry after an unknown-outcome failure
 * produces a duplicate. The rest are here because their effect is not a pure function of their
 * arguments (`sync.push` sends whatever is dirty *now*, which a retry changes).
 */
export const NON_IDEMPOTENT_COMMANDS: ReadonlySet<string> = new Set([
  "entity.create", // allocates an id; a retry is a second entity
  "folder.add", // allocates an id
  "workspace.add", // allocates an id
  "application.save", // allocates an id
  "application.delete",
  "application.start",
  "application.stop",
  "application.killPort",
  "runner.saveReport",
  "runner.exportReport",
  "runner.saveConfig",
  "sync.setRemote",
  "sync.push", // sends whatever is dirty at call time
  "sync.pull",
  "git.sync", // commits; a retry re-commits
  "entity.publish",
  "entity.restore",
  "audit.export",
  "tls.generate",
  "tls.exportCert",
  "export.create", // allocates a blob id
  "import.commit", // applies a plan
  "blob.put", // allocates a blob id
  "script.execute", // arbitrary user code; side effects are its own
  "request.replay", // an outbound HTTP request, not a read of local state
  "graphql.execute",
  "soap.execute",
  "grpc.execute",
  "grpc.startMockServer",
  "grpc.stopMockServer",
  "capture.shareJson",
  "healthbar.saveServices",
]);

/** The frozen command list, so the partition can be asserted against it. */
export const ALL_COMMANDS: readonly string[] = Object.keys(COMMANDS);

/**
 * Throw unless the two sets exactly partition `COMMANDS`.
 *
 * Same shape and same reasoning as `assertBridgeIsTotal()` (engine) and
 * `assertSurfaceIsTotal()` (this package): a classification that is merely written down drifts,
 * and this one drifts in the direction of duplicated mutations.
 */
export function assertRetryPolicyIsTotal(): void {
  const overlap = [...IDEMPOTENT_COMMANDS].filter((c) => NON_IDEMPOTENT_COMMANDS.has(c));
  const known = new Set([...IDEMPOTENT_COMMANDS, ...NON_IDEMPOTENT_COMMANDS]);
  const unclassified = ALL_COMMANDS.filter((c) => !known.has(c));
  const phantom = [...known].filter((c) => !ALL_COMMANDS.includes(c));

  const problems: string[] = [];
  if (overlap.length > 0) problems.push(`in both sets: ${overlap.sort().join(", ")}`);
  if (unclassified.length > 0) problems.push(`unclassified: ${unclassified.sort().join(", ")}`);
  if (phantom.length > 0) problems.push(`not a real command: ${phantom.sort().join(", ")}`);

  if (problems.length > 0) {
    throw new Error(
      `packages/client/src/retry.ts does not partition the protocol's commands — ${problems.join("; ")}. ` +
        `Every command needs an idempotency decision; see the file header for why the safe default ` +
        `is "not retryable".`,
    );
  }
}

/** Is replaying `command` after an unknown-outcome failure safe? */
export function isIdempotentCommand(command: string): boolean {
  return IDEMPOTENT_COMMANDS.has(command);
}

/**
 * The effective policy: retry only when the **error code** is retryable *and* the **command**
 * is idempotent. Either half alone is a bug — see the header.
 */
export function isSafeToRetry(command: string, code: ErrorCodeValue): boolean {
  return isRetryable(code) && isIdempotentCommand(command);
}

// ── applying the policy ──────────────────────────────────────────────────────

/**
 * How the client retries. Injectable because the alternative is tests that sleep: the default
 * `sleep` is real wall-clock time, and a test asserting "attempts 3, delays 50ms then 100ms" would
 * otherwise take 150ms per case and be flaky under load.
 */
export interface RetryPolicy {
  /** **Total attempts**, not retries. `1` disables retrying entirely. */
  readonly attempts: number;
  readonly baseDelayMs: number;
  readonly maxDelayMs: number;
  readonly sleep: (ms: number) => Promise<void>;
}

/**
 * Three attempts, exponential backoff from 50ms capped at 1s — so a full retry sequence adds at
 * most 150ms before the error surfaces.
 *
 * Deliberately small. These transports are loopback or a local socket: a failure that persists
 * across three attempts is not transient, and a longer backoff would only delay the error while
 * the user watches a spinner. The engine's own reconnect logic is where patience belongs, not here.
 */
export const DEFAULT_RETRY_POLICY: RetryPolicy = {
  attempts: 3,
  baseDelayMs: 50,
  maxDelayMs: 1_000,
  sleep: (ms) => new Promise<void>((resolve) => setTimeout(resolve, ms)),
};

/** `base * 2^(attempt-1)`, capped. `attempt` is 1-based, i.e. the attempt that just failed. */
export function backoffDelayMs(attempt: number, policy: RetryPolicy): number {
  return Math.min(policy.maxDelayMs, policy.baseDelayMs * 2 ** (attempt - 1));
}

/**
 * Run `command`'s request, retrying it while `isSafeToRetry` says so.
 *
 * `run` is a thunk rather than a payload so that a retry is a **fresh** `transport.request()` call —
 * a retry that reused a settled promise would not retry anything, which is the classic way this
 * kind of wrapper silently becomes a no-op.
 *
 * Only an `EngineError` is considered for retry. That is exact rather than a duck-type guess: all
 * four transports reject with a real `EngineError` — `inProcess` throws one directly, and `stdio`,
 * `ws` and `socket` rebuild one through `remoteErrorToEngineError` — so anything else is a bug in a
 * transport, and retrying it would be guessing.
 *
 * ## `isTransportClosed` — the third term, and why the policy needs one
 *
 * `isSafeToRetry` is two terms, and they are not quite enough. Every transport rejects a request on
 * a **closed** transport with `ENGINE_ERROR` (`inProcess.ts`'s `assertOpen`, `stdio.ts`'s
 * `teardown`, `ws.ts`'s, `socket.ts`'s), and the protocol's `RETRYABLE_CODES` includes
 * `ENGINE_ERROR`. So the two-term policy happily retries a transport that is permanently dead —
 * three attempts, 150ms, two guaranteed-identical failures.
 *
 * The plan's own rule for this case is "connection loss mid-request: reject with a typed error; the
 * caller decides whether to retry" — i.e. *do not* auto-retry. So this hook is the client telling
 * the loop "I know this transport is gone", which is information the error code cannot carry.
 *
 * It is a guard, not a fix. The fix is a distinct non-retryable code for a closed transport
 * (`ENGINE_ERROR` means "unhandled internal", which is a *transient* claim, and the closed case is
 * the opposite); that is a protocol change plus seven call sites across four transports, and it
 * belongs with `plan/05`'s item 5 hardening rather than here. Recorded in `plan/06`.
 */
export async function withRetry<T>(
  command: string,
  run: () => Promise<T>,
  policy: RetryPolicy = DEFAULT_RETRY_POLICY,
  isTransportClosed: () => boolean = () => false,
): Promise<T> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await run();
    } catch (err) {
      const lastAttempt = attempt >= policy.attempts;
      if (lastAttempt || isTransportClosed()) throw err;
      if (!(err instanceof EngineError)) throw err;
      if (!isSafeToRetry(command, err.code)) throw err;
      await policy.sleep(backoffDelayMs(attempt, policy));
    }
  }
}
