/**
 * The collection runner's **loop** — `plan/19` WI 4.
 *
 * ## What moved, and what this file is
 *
 * `packages/gui/src/lib/collectionRunner.ts`'s `runCollection` executed a folder's requests in the renderer:
 * resolve, pre-script, replay, post-script, test — **a second copy of the same five steps**
 * `useRestActions.handleSend` ran (`plan/19` §1 rows 3 and 4). WI 3 collapsed those five steps into
 * `pipeline/runPipeline.ts`; this file is the *loop around them*, and it is deliberately thin because of that:
 *
 * > **One `registry.invoke("request.send", { workspaceId, id })` per request.**
 *
 * That is the whole per-request body. The pipeline owns resolution, the scripts and the send; the runner owns
 * ordering, the delay, cancellation and the report. The renderer's version could not be this thin because it
 * had to do the five steps itself.
 *
 * ## Cancellation is a **callback**, and that is decision D29
 *
 * The loop does not know what cancels a run — it asks `isCancelled()`. That is what makes D29 testable rather
 * than merely asserted: *the only thing that stops a run is an explicit `runner.cancel`*, because the flag
 * behind this callback is set by that command and by nothing else. **No transport-disconnect hook, and no
 * workspace-switch hook, reaches this loop** — which is precisely what D29 decided, and what §8 trap 5 warns
 * is the default most implementations reach for.
 *
 * ## A failing request is recorded and the run continues
 *
 * `runRequestPipeline` propagates a send rejection, and its header says why: *"the two callers disagree about
 * what a failure means: a tab surfaces it to the user, while the collection runner records it against that
 * request and continues."* This is that second caller, so the catch is here rather than in the pipeline — one
 * request failing must not abandon the other thirty.
 *
 * ## Two config fields the renderer stored but never honoured, kept un-honoured
 *
 * `runner.json` carries `stopOnFailure` and `iterations`, and the renderer's `runCollection` read **neither** —
 * it used `delayMs` and `requestOrder` only, so a folder configured to stop on failure ran to the end anyway.
 * Reproducing that is deliberate: implementing them inside a move would be a behaviour change smuggled in
 * under a refactor, and each is a real decision (does a failed *test* count as a failure, or only a failed
 * *send*?) that this file is not the place to make. Recorded here so the omission reads as one.
 */
import type { CommandContext, CommandRegistry } from "../kernel/registry";

/** One request's outcome — the engine's own copy of what the renderer's `RunnerRequestResult` carried. */
export interface RunnerRequestResult {
  requestId: string;
  requestName: string;
  /** The method and url **as actually sent** — resolved, and rewritten by the pre-script if there was one. */
  method: string;
  url: string;
  status: number | null;
  responseTime: number;
  error?: string;
  tests: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs: string[];
  preScriptError?: string;
  postScriptError?: string;
}

/** What a finished run produces. The field set is the protocol's `RunReport` (`commands/runner.ts`). */
export interface CollectionRunReport {
  folderId: string;
  folderName?: string;
  startedAt: number;
  completedAt: number;
  totalRequests: number;
  totalTests: number;
  passedTests: number;
  failedTests: number;
  results: RunnerRequestResult[];
}

/** The minimum the loop needs to know about a request — it never reads the stored entity itself. */
export interface RunRequestRef {
  id: string;
  name: string;
  method: string;
  url: string;
}

export interface RunCallbacks {
  /** Fired before a request is sent. `index` is 0-based, matching the report's `results` order. */
  onRequestStart?(index: number, request: RunRequestRef): void;
  /** Fired after a request completes, successfully or not. */
  onRequestDone?(index: number, result: RunnerRequestResult): void;
}

export interface RunCollectionOptions {
  registry: CommandRegistry;
  ctx: CommandContext;
  workspaceId: string;
  folderId: string;
  folderName?: string;
  /** Already ordered — the loop does not sort, so the caller owns `runner.json`'s `requestOrder`. */
  requests: RunRequestRef[];
  delayMs?: number;
  isCancelled: () => boolean;
  callbacks?: RunCallbacks;
}

/** `request.send`'s result, as far as this loop reads it. */
interface SendOutcome {
  status: number;
  durationMs: number;
  request: { method: string; url: string };
  tests: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs: string[];
  preScriptError?: string;
  postScriptError?: string;
}

export async function runCollection(opts: RunCollectionOptions): Promise<CollectionRunReport> {
  const startedAt = Date.now();
  const results: RunnerRequestResult[] = [];

  for (let i = 0; i < opts.requests.length; i++) {
    if (opts.isCancelled()) break;

    if (i > 0 && opts.delayMs && opts.delayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, opts.delayMs));
    }

    const request = opts.requests[i];
    opts.callbacks?.onRequestStart?.(i, request);

    let result: RunnerRequestResult;
    try {
      // **The whole per-request body.** The pipeline resolves, runs the scripts and sends; this loop only
      // decides *which* request and *when*.
      const sent = (await opts.registry.invoke(
        "request.send",
        { workspaceId: opts.workspaceId, id: request.id },
        opts.ctx,
      )) as SendOutcome;

      result = {
        requestId: request.id,
        requestName: request.name,
        // From `sent.request`, not from `request`: what the run *did* is what the report has to say, and
        // after engine-side resolution the caller genuinely does not know what was sent.
        method: sent.request.method,
        url: sent.request.url,
        status: sent.status,
        responseTime: sent.durationMs,
        tests: sent.tests ?? [],
        testLogs: sent.testLogs ?? [],
        preScriptError: sent.preScriptError,
        postScriptError: sent.postScriptError,
      };
    } catch (e) {
      // Recorded, not rethrown — see the header. `status: null` is what the report has always used for
      // "no response", and it is why the field is nullable.
      result = {
        requestId: request.id,
        requestName: request.name,
        method: request.method,
        url: request.url,
        status: null,
        responseTime: 0,
        error: e instanceof Error ? e.message : String(e),
        tests: [],
        testLogs: [],
      };
    }

    results.push(result);
    opts.callbacks?.onRequestDone?.(i, result);
  }

  const allTests = results.flatMap((r) => r.tests);
  return {
    folderId: opts.folderId,
    folderName: opts.folderName,
    startedAt,
    completedAt: Date.now(),
    totalRequests: results.length,
    totalTests: allTests.length,
    passedTests: allTests.filter((t) => t.passed).length,
    failedTests: allTests.filter((t) => !t.passed).length,
    results,
  };
}
