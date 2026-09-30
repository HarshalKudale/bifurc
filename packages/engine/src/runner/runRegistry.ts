/**
 * Engine-owned **run state** — `plan/19` WI 4, decisions **D29** and **D30**.
 *
 * ## D29: the state lives as long as the engine does
 *
 * The map below is module-level and there is deliberately **no persistence and no teardown hook**. A run
 * survives a client disconnect and a workspace switch because nothing in this file can observe either — the
 * only thing that stops a run is `cancelRun()`, which `runner.cancel` is the only caller of. A **host restart
 * loses the state**, and that was decided rather than overlooked: persisting it would mean a restart has to
 * decide whether to re-send the requests it had already sent, and that question has no safe default.
 *
 * **§8 trap 5 is the reason this is spelled out**: *"a disconnected client must not cancel the run — so
 * 'cancel on disconnect' is the wrong default, and it is the default most implementations reach for."* A
 * reader looking for where a disconnect is handled will not find one, and this paragraph is why.
 *
 * ## D30: one run per `{workspaceId, folderId}`
 *
 * The key is the pair, so a second run of the **same** folder is refused while one is in flight, and two
 * **different** folders in one workspace run side by side. §8 trap 6: *"concurrency is undefined today because
 * the renderer made it impossible … prevented by there being one browser tab."* The per-folder policy is a
 * superset of that — the old behaviour allowed zero concurrent runs — so nothing regresses.
 *
 * **A *finished* run does not block a new one.** The guard tests `running`, not mere presence, because the
 * snapshot has to outlive the run for `runner.status` to report a completed one (an acceptance criterion in
 * §9). The new run replaces the old snapshot, which is the same thing the renderer did when it reset its
 * `report` state.
 *
 * ## What is NOT here yet
 *
 * The wire: `runner.start` / `runner.cancel` / `runner.status` and the progress events are the next slice.
 * `startRun` therefore takes an `onFinished` callback rather than emitting on the bus, and the run's *progress*
 * is readable through `getRun()` — which is what the commands will wrap and what the tests drive directly.
 */
import type { CommandContext, CommandRegistry } from "../kernel/registry";
import {
  runCollection,
  type CollectionRunReport,
  type RunnerRequestResult,
  type RunRequestRef,
} from "./runCollection";

export interface RunSnapshot {
  workspaceId: string;
  folderId: string;
  folderName?: string;
  startedAt: number;
  /** How many requests the run intends to send. Known before the first one, so a UI can show `n / total`. */
  total: number;
  completed: number;
  running: boolean;
  cancelled: boolean;
  completedAt?: number;
  /** Set only when the loop itself failed, which is not the same as a request failing — see `runCollection`. */
  error?: string;
  results: RunnerRequestResult[];
  /** Present once the run has finished, so `runner.status` can answer for a completed run. */
  report?: CollectionRunReport;
}

/**
 * The runs, keyed by `{workspaceId, folderId}` (D30). **Module-level, and that is D29** — see the header.
 *
 * `\u0000` as the separator rather than `:` because a workspace or folder id containing a colon is not
 * forbidden anywhere, and a key collision would silently attach one folder's run to another's.
 */
const RUNS = new Map<string, RunSnapshot>();

const keyOf = (workspaceId: string, folderId: string): string => `${workspaceId}\u0000${folderId}`;

/** The run for a folder, running or finished, or `null` if it has never been started. */
export function getRun(workspaceId: string, folderId: string): RunSnapshot | null {
  return RUNS.get(keyOf(workspaceId, folderId)) ?? null;
}

/** Every run the engine is tracking — the basis for a future `runner.list`. */
export function listRuns(workspaceId?: string): RunSnapshot[] {
  const all = [...RUNS.values()];
  return workspaceId === undefined ? all : all.filter((r) => r.workspaceId === workspaceId);
}

/** Test seam: the map is module-level by design, so a suite needs a way to start from empty. */
export function resetRunsForTests(): void {
  RUNS.clear();
}

export interface StartRunOptions {
  registry: CommandRegistry;
  ctx: CommandContext;
  workspaceId: string;
  folderId: string;
  /**
   * Called after each request completes, with its 0-based index. **The wire's seam** — `runner.start` emits
   * `event.runner.progress` from here, so this module never reaches for the bus (and `runner/`'s §6 row keeps
   * `kernel/` imports to types).
   */
  onProgress?: (index: number, result: RunnerRequestResult) => void;
  /** Called once the run has finished, with the report. The other half of the same seam. */
  onFinished?: (report: CollectionRunReport) => void;
}

export type StartRunResult = { ok: true; total: number } | { ok: false; error: string };

/**
 * Start a run, or refuse because one is already going for this folder (D30).
 *
 * **Returns as soon as the run is registered** — it does not await the loop, which is D29 in the small: a
 * caller that had to await the whole run would be a caller that owns its lifetime, and the point is that the
 * engine does.
 */
export function startRun(opts: StartRunOptions): StartRunResult {
  const existing = RUNS.get(keyOf(opts.workspaceId, opts.folderId));
  if (existing?.running) {
    // D30. The message names the folder, because the caller's next question is "which one is running?".
    return {
      ok: false,
      error:
        `A run of folder "${opts.folderId}" is already in flight in workspace "${opts.workspaceId}". ` +
        `Cancel it with runner.cancel, or wait for it to finish.`,
    };
  }

  const requests = loadOrderedRequests(opts);
  const snapshot: RunSnapshot = {
    workspaceId: opts.workspaceId,
    folderId: opts.folderId,
    startedAt: Date.now(),
    total: requests.length,
    completed: 0,
    running: true,
    cancelled: false,
    results: [],
  };
  RUNS.set(keyOf(opts.workspaceId, opts.folderId), snapshot);

  void runCollection({
    registry: opts.registry,
    ctx: opts.ctx,
    workspaceId: opts.workspaceId,
    folderId: opts.folderId,
    requests,
    delayMs: loadDelayMs(opts),
    // The whole of D29: the flag this reads is set by `cancelRun` and by nothing else.
    isCancelled: () => snapshot.cancelled,
    callbacks: {
      onRequestDone: (index, result) => {
        snapshot.results.push(result);
        snapshot.completed = snapshot.results.length;
        opts.onProgress?.(index, result);
      },
    },
  })
    .then((report) => {
      snapshot.running = false;
      snapshot.completedAt = report.completedAt;
      snapshot.report = report;
      opts.onFinished?.(report);
    })
    .catch((e: unknown) => {
      // The loop only rejects if something outside a request threw — a request failure is caught inside
      // `runCollection`. So this is a bug rather than a run outcome, and it is recorded as one.
      snapshot.running = false;
      snapshot.error = e instanceof Error ? e.message : String(e);
    });

  return { ok: true, total: requests.length };
}

/** Stop a run at its next iteration boundary. `{ok:false}` when there is nothing to stop. */
export function cancelRun(workspaceId: string, folderId: string): { ok: boolean; error?: string } {
  const run = RUNS.get(keyOf(workspaceId, folderId));
  if (!run) {
    return { ok: false, error: `No run of folder "${folderId}" in workspace "${workspaceId}".` };
  }
  // Cancelling a finished run is a no-op rather than an error: the caller's intent is satisfied either way,
  // and the alternative is a race the UI would have to defend against.
  run.cancelled = true;
  return { ok: true };
}

/**
 * The folder's requests, in `runner.json`'s order.
 *
 * **Sourced through the registry, not by reading files** — `entity.list` and `runner.loadConfig` are the
 * commands the shell and a headless host both register, so this works in either deployment without this module
 * knowing how a workspace is laid out.
 *
 * The ordering is the renderer's, including its two fallbacks, and both are load-bearing: an id missing from
 * `requestOrder` sorts to the end (`9999`), which is how a request added *after* a folder was configured still
 * runs; and a stable sort is what keeps those late additions in their natural order among themselves.
 */
function loadOrderedRequests(opts: StartRunOptions): RunRequestRef[] {
  const listed = opts.registry.invoke(
    "entity.list",
    { workspaceId: opts.workspaceId, kind: "requests" },
    opts.ctx,
  ) as { entities: Record<string, unknown>[] };

  const inFolder = (listed.entities ?? []).filter((e) => (e.folderId ?? null) === opts.folderId);
  const order = loadRequestOrder(opts);
  const rank = new Map(order.map((id, index) => [id, index]));

  return [...inFolder]
    .sort((a, b) => (rank.get(String(a.id)) ?? 9999) - (rank.get(String(b.id)) ?? 9999))
    .map((e) => ({
      id: String(e.id),
      name: String(e.name ?? ""),
      method: String(e.method ?? "GET"),
      url: String(e.url ?? ""),
    }));
}

/** `runner.json`'s `requestOrder`, or `[]` when the folder has no config — the same default the renderer used. */
function loadRequestOrder(opts: StartRunOptions): string[] {
  const config = loadRunnerConfig(opts);
  const order = config?.requestOrder;
  return Array.isArray(order) ? order.map(String) : [];
}

/** `runner.json`'s `delayMs`, or `undefined` — `runCollection` treats a missing or zero delay the same way. */
function loadDelayMs(opts: StartRunOptions): number | undefined {
  const delay = loadRunnerConfig(opts)?.delayMs;
  return typeof delay === "number" && delay > 0 ? delay : undefined;
}

/**
 * The folder's saved config, or `null`.
 *
 * `RunnerConfigSchema` is `z.record(z.string(), z.unknown())` — deliberately, because the config is the
 * *caller's* shape — so the two fields this module reads are narrowed here rather than trusted. `loadConfig`
 * already answers `null` for a folder with no `runner.json`; the `catch` covers a command that is not
 * registered at all, which is a real possibility for a consumer that registers a subset.
 */
function loadRunnerConfig(opts: StartRunOptions): { requestOrder?: unknown; delayMs?: unknown } | null {
  try {
    return (opts.registry.invoke(
      "runner.loadConfig",
      { workspaceId: opts.workspaceId, folderId: opts.folderId },
      opts.ctx,
    ) ?? null) as { requestOrder?: unknown; delayMs?: unknown } | null;
  } catch {
    return null;
  }
}
