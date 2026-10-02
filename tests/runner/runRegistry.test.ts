import { describe, it, expect, beforeEach } from "vitest";
import { CommandRegistry } from "@bifurc/engine/kernel/registry";
import {
    runCollection,
    type CollectionRunReport,
    type RunRequestRef,
} from "@bifurc/engine/runner/runCollection";
import {
    startRun,
    cancelRun,
    getRun,
    listRuns,
    resetRunsForTests,
} from "@bifurc/engine/runner/runRegistry";

/**
 * The collection runner's loop and its run state — `plan/19` WI 4.
 *
 * ## Why these drive the engine directly rather than through a transport
 *
 * The wire (`runner.start` / `runner.cancel` / `runner.status`, and the progress events) is the next slice,
 * and it is a *wrapping* of exactly these functions. Testing them here means the two decisions this item
 * settled — **D29** (the run's lifetime is the engine's) and **D30** (one run per `{workspaceId, folderId}`) —
 * are asserted against the code that implements them rather than against a transport's encoding of them.
 *
 * `CommandRegistry` is real, and so is the schema validation `invoke()` performs: the stubs below answer for
 * three commands, and a payload that did not match `request.send`'s schema would fail here rather than
 * silently pass. `runCollection`'s loop is exercised through `request.send` because that is the one call it
 * makes — the pipeline behind that command is `tests/pipeline/runPipeline.test.ts`'s subject, not this file's.
 */
const WS = "ws-1";
const FOLDER = "f-1";
const OTHER_FOLDER = "f-2";

/** A `request.send` result, as far as the loop reads it. */
const outcome = (id: string, over: Record<string, unknown> = {}) => ({
    status: 200,
    durationMs: 5,
    request: { method: "GET", url: `http://upstream.test/${id}` },
    tests: [{ name: "ok", passed: true, durationMs: 0 }],
    testLogs: [],
    ...over,
});

function makeRegistry(opts: {
    send?: (id: string) => unknown;
    requests?: Record<string, unknown>[];
    config?: Record<string, unknown> | null;
}) {
    const registry = new CommandRegistry();
    registry.register("request.send", ((p: { id: string }) => opts.send?.(p.id)) as never);
    registry.register("entity.list", (() => ({ entities: opts.requests ?? [] })) as never);
    registry.register("runner.loadConfig", (() => opts.config ?? null) as never);
    return registry;
}

/** A stored `requests` entity, as `entity.list` would return it. */
const storedRequest = (id: string, folderId = FOLDER) => ({
    id,
    name: `Request ${id}`,
    method: "GET",
    url: `http://example.test/${id}`,
    folderId,
});

const ref = (id: string): RunRequestRef => ({
    id,
    name: `Request ${id}`,
    method: "GET",
    url: `http://example.test/${id}`,
});

/** Await a run's completion through the seam the wire will use. */
function finished(): { promise: Promise<CollectionRunReport>; onFinished: (r: CollectionRunReport) => void } {
    let resolve!: (r: CollectionRunReport) => void;
    const promise = new Promise<CollectionRunReport>((res) => {
        resolve = res;
    });
    return { promise, onFinished: resolve };
}

const ctx = {} as never;

beforeEach(() => {
    resetRunsForTests();
});

describe("runCollection — the loop", () => {
    it("sends every request, in the order it was given, and totals the tests", async () => {
        const seen: string[] = [];
        const registry = makeRegistry({
            send: (id) => {
                seen.push(id);
                return outcome(id);
            },
        });

        const report = await runCollection({
            registry,
            ctx,
            workspaceId: WS,
            folderId: FOLDER,
            requests: [ref("a"), ref("b"), ref("c")],
            isCancelled: () => false,
        });

        expect(seen).toEqual(["a", "b", "c"]);
        expect(report.totalRequests).toBe(3);
        expect(report.totalTests).toBe(3);
        expect(report.passedTests).toBe(3);
        expect(report.failedTests).toBe(0);
        expect(report.results.map((r) => r.requestId)).toEqual(["a", "b", "c"]);
    });

    it("reports the request AS SENT, not as stored", async () => {
        // The pipeline resolves and may rewrite the request, so the stored url is not what went out. A report
        // built from the input would describe a request the run did not make.
        const registry = makeRegistry({
            send: (id) =>
                outcome(id, { request: { method: "POST", url: `http://upstream.test/resolved/${id}` } }),
        });

        const report = await runCollection({
            registry,
            ctx,
            workspaceId: WS,
            folderId: FOLDER,
            requests: [ref("a")],
            isCancelled: () => false,
        });

        expect(report.results[0].method).toBe("POST");
        expect(report.results[0].url).toBe("http://upstream.test/resolved/a");
    });

    it("records a failing request and keeps going — one failure must not abandon the rest", async () => {
        const registry = makeRegistry({
            send: (id) => {
                if (id === "b") throw new Error("ECONNREFUSED");
                return outcome(id);
            },
        });

        const report = await runCollection({
            registry,
            ctx,
            workspaceId: WS,
            folderId: FOLDER,
            requests: [ref("a"), ref("b"), ref("c")],
            isCancelled: () => false,
        });

        expect(report.results).toHaveLength(3);
        expect(report.results[1].status).toBeNull();
        expect(report.results[1].error).toContain("ECONNREFUSED");
        // The one after the failure still ran, which is the assertion that matters.
        expect(report.results[2].status).toBe(200);
    });

    it("stops at the next iteration boundary when cancelled, and reports what it managed", async () => {
        let sent = 0;
        const registry = makeRegistry({
            send: (id) => {
                sent++;
                return outcome(id);
            },
        });

        const report = await runCollection({
            registry,
            ctx,
            workspaceId: WS,
            folderId: FOLDER,
            requests: [ref("a"), ref("b"), ref("c")],
            // Cancelled after the first request, so the loop must stop before the second.
            isCancelled: () => sent >= 1,
        });

        expect(sent).toBe(1);
        expect(report.totalRequests).toBe(1);
        expect(report.results.map((r) => r.requestId)).toEqual(["a"]);
    });
});

describe("runRegistry — D29 and D30", () => {
    it("D29: startRun returns before the run finishes, and the state is readable while it runs", async () => {
        const gate = finished();
        // A `request.send` that only resolves when the test says so — the run is genuinely in flight.
        let release!: () => void;
        const held = new Promise<void>((res) => {
            release = res;
        });
        const registry = makeRegistry({
            send: async (id) => {
                await held;
                return outcome(id);
            },
            requests: [storedRequest("a"), storedRequest("b")],
        });

        const started = startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER, onFinished: gate.onFinished });

        expect(started).toEqual({ ok: true, total: 2 });
        const inFlight = getRun(WS, FOLDER);
        expect(inFlight?.running).toBe(true);
        expect(inFlight?.total).toBe(2);
        expect(inFlight?.completed).toBe(0);

        release();
        const report = await gate.promise;
        expect(report.totalRequests).toBe(2);
        // The snapshot outlives the run, which is what lets `runner.status` answer for a finished one.
        expect(getRun(WS, FOLDER)?.running).toBe(false);
        expect(getRun(WS, FOLDER)?.report?.totalRequests).toBe(2);
    });

    it("D29: only cancelRun stops a run — nothing else in the module can", async () => {
        let sent = 0;
        let release!: () => void;
        const first = new Promise<void>((res) => {
            release = res;
        });
        const registry = makeRegistry({
            send: async (id) => {
                sent++;
                if (sent === 1) await first;
                return outcome(id);
            },
            requests: [storedRequest("a"), storedRequest("b"), storedRequest("c")],
        });

        const gate = finished();
        startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER, onFinished: gate.onFinished });
        release();

        // The explicit cancel is the only lever; there is no disconnect hook to reach for.
        expect(cancelRun(WS, FOLDER)).toEqual({ ok: true });
        await gate.promise;

        // "a" was in flight when the cancel arrived, so it completed; "b" and "c" never started.
        expect(sent).toBe(1);
        expect(getRun(WS, FOLDER)?.cancelled).toBe(true);
    });

    it("D30: a second run of the SAME folder is refused, and the error names it", () => {
        const registry = makeRegistry({
            send: async (id) => {
                await new Promise((r) => setTimeout(r, 0));
                return outcome(id);
            },
            requests: [storedRequest("a")],
        });

        expect(startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER })).toEqual({ ok: true, total: 1 });
        const second = startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER });

        expect(second.ok).toBe(false);
        expect(second.ok === false && second.error).toContain(FOLDER);
    });

    it("D30: two DIFFERENT folders run side by side in one workspace", () => {
        const registry = makeRegistry({
            send: async (id) => {
                await new Promise((r) => setTimeout(r, 0));
                return outcome(id);
            },
            requests: [storedRequest("a", FOLDER), storedRequest("b", OTHER_FOLDER)],
        });

        expect(startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER }).ok).toBe(true);
        expect(startRun({ registry, ctx, workspaceId: WS, folderId: OTHER_FOLDER }).ok).toBe(true);
        expect(listRuns(WS)).toHaveLength(2);
        // Each run sees only its own folder's request, which is the per-folder key doing its job.
        expect(getRun(WS, FOLDER)?.total).toBe(1);
        expect(getRun(WS, OTHER_FOLDER)?.total).toBe(1);
    });

    it("D30: a FINISHED run does not block a new one", async () => {
        const registry = makeRegistry({ send: (id) => outcome(id), requests: [storedRequest("a")] });

        const first = finished();
        startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER, onFinished: first.onFinished });
        await first.promise;

        expect(startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER })).toEqual({ ok: true, total: 1 });
    });

    it("orders the folder's requests by runner.json's requestOrder, late additions last", async () => {
        const seen: string[] = [];
        const registry = makeRegistry({
            send: (id) => {
                seen.push(id);
                return outcome(id);
            },
            // "c" is absent from `requestOrder` — the case a folder gets when a request is added after it was
            // configured, and the reason the rank fallback exists.
            requests: [storedRequest("a"), storedRequest("b"), storedRequest("c")],
            config: { requestOrder: ["b", "a"] },
        });

        const gate = finished();
        startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER, onFinished: gate.onFinished });
        await gate.promise;

        expect(seen).toEqual(["b", "a", "c"]);
    });

    it("ignores a request that belongs to another folder", async () => {
        const seen: string[] = [];
        const registry = makeRegistry({
            send: (id) => {
                seen.push(id);
                return outcome(id);
            },
            requests: [storedRequest("mine"), storedRequest("theirs", OTHER_FOLDER)],
        });

        const gate = finished();
        startRun({ registry, ctx, workspaceId: WS, folderId: FOLDER, onFinished: gate.onFinished });
        await gate.promise;

        expect(seen).toEqual(["mine"]);
    });

    it("cancelRun on a folder with no run is a refusal, not a throw", () => {
        expect(cancelRun(WS, "never-started")).toMatchObject({ ok: false });
        expect(getRun(WS, "never-started")).toBeNull();
    });
});
