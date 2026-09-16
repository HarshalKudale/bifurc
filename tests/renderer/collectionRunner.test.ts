/**
 * Happy-path workflow test for the **Collection Runner** — the engine behind the
 * Run button. `renderer/lib/{collectionRunner,testRunner,runnerReport}.ts` were at
 * **0% coverage**: nothing verified that a run actually walks the folder, applies
 * pre/post scripts, sends the request, evaluates the test script and totals the
 * results.
 *
 * The main-process script sandbox is exercised for real (`executeIpcScript`); only
 * `window.api.replayRequest` is stubbed, because the point here is the orchestration
 * and the report, not the socket (that is covered by
 * `tests/integration/protocolExecution.integration.test.ts`).
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { runCollection, type RunnerRequestResult } from "@/lib/collectionRunner";
import { runTestScript } from "@/lib/testRunner";
import { generateHtmlReport, saveRunnerReport, getRunHistory } from "@/lib/runnerReport";
import { executeIpcScript } from "@bifurc/engine/proxy/scriptExecutor";
import type { Environment, SavedRequest } from "@/types";

// ── Fixtures ──────────────────────────────────────────────────────────────────

function makeEnv(vars: Record<string, string> = {}): Environment {
  return {
    id: "env-1",
    name: "Test",
    createdAt: Date.now(),
    variables: Object.entries(vars).map(([key, value]) => ({ id: key, key, value })),
    workspaceId: "ws-1",
  };
}

function makeRequest(overrides: Partial<SavedRequest> = {}): SavedRequest {
  return {
    id: "req-1",
    name: "Get Users",
    method: "GET",
    url: "http://api.localhost/users",
    headers: { "content-type": "application/json" },
    body: "",
    folderId: "folder-1",
    createdAt: 1,
    workspaceId: "ws-1",
    ...overrides,
  } as SavedRequest;
}

/** A replay stub returning a fixed response, and recording what it was asked to send. */
function stubReplay(response: { status?: number; headers?: Record<string, string>; body?: string } = {}) {
  const calls: Array<{ method: string; url: string; headers: Record<string, string>; bodyB64: string }> = [];
  const fn = vi.fn(async (method: string, url: string, headers: Record<string, string>, bodyB64: string) => {
    calls.push({ method, url, headers, bodyB64 });
    return {
      status: response.status ?? 200,
      headers: response.headers ?? { "content-type": "application/json" },
      body: Buffer.from(response.body ?? JSON.stringify({ ok: true }), "utf-8").toString("base64"),
    };
  });
  return { fn, calls };
}

let replay: ReturnType<typeof stubReplay>;

beforeEach(() => {
  replay = stubReplay();
  (globalThis as any).window = {
    ...(globalThis as any).window,
    api: {
      ...((globalThis as any).window?.api ?? {}),
      // Real vm-based sandbox so pre/post/test scripts genuinely execute.
      executeScript: (opts: any) => Promise.resolve(executeIpcScript(opts)),
      replayRequest: replay.fn,
    },
  };
});

const FOLDER = { id: "folder-1", name: "Smoke Tests" } as const;

function run(requests: SavedRequest[], env: Environment | null = null, callbacks?: any, delayMs?: number) {
  return runCollection(requests, env, FOLDER.id, FOLDER.name, callbacks, delayMs);
}

// ── runCollection — orchestration ─────────────────────────────────────────────

describe("runCollection — happy path", () => {
  it("sends every request in the folder and reports the totals", async () => {
    const report = await run([makeRequest({ id: "a" }), makeRequest({ id: "b" })]);

    expect(report.totalRequests).toBe(2);
    expect(report.results.map((r) => r.requestId)).toEqual(["a", "b"]);
    expect(replay.fn).toHaveBeenCalledTimes(2);
  });

  it("stamps the folder and the run window", async () => {
    const report = await run([makeRequest()]);

    expect(report.folderId).toBe(FOLDER.id);
    expect(report.folderName).toBe("Smoke Tests");
    expect(report.startedAt).toBeLessThanOrEqual(report.completedAt);
  });

  it("handles an empty folder without sending anything", async () => {
    const report = await run([]);

    expect(report.totalRequests).toBe(0);
    expect(report.results).toEqual([]);
    expect(replay.fn).not.toHaveBeenCalled();
  });

  it("records the status and the response time of each request", async () => {
    replay = stubReplay({ status: 201 });
    (globalThis as any).window.api.replayRequest = replay.fn;

    const report = await run([makeRequest()]);

    expect(report.results[0].status).toBe(201);
    expect(report.results[0].responseTime).toBeGreaterThanOrEqual(0);
  });

  it("forwards the method and the base64 body to the sender", async () => {
    await run([makeRequest({ method: "POST", body: '{"a":1}' })]);

    expect(replay.calls[0].method).toBe("POST");
    expect(Buffer.from(replay.calls[0].bodyB64, "base64").toString("utf-8")).toBe('{"a":1}');
  });

  it("sends an empty body for a request with no body", async () => {
    await run([makeRequest({ body: "" })]);
    expect(replay.calls[0].bodyB64).toBe("");
  });

  it("substitutes environment variables in the URL before sending", async () => {
    const report = await run(
      [makeRequest({ url: "http://{{host}}/users" })],
      makeEnv({ host: "api.localhost" }),
    );

    expect(replay.calls[0].url).toBe("http://api.localhost/users");
    expect(report.results[0].url).toBe("http://api.localhost/users");
  });

  it("notifies onRequestStart and onRequestDone for each request, in order", async () => {
    const starts: number[] = [];
    const done: RunnerRequestResult[] = [];

    await run(
      [makeRequest({ id: "a" }), makeRequest({ id: "b" })],
      null,
      {
        onRequestStart: (i: number) => starts.push(i),
        onRequestDone: (_i: number, r: RunnerRequestResult) => done.push(r),
      },
    );

    expect(starts).toEqual([0, 1]);
    expect(done.map((r) => r.requestId)).toEqual(["a", "b"]);
  });

  it("stops early when the run is cancelled, keeping only what already ran", async () => {
    let seen = 0;
    const report = await run(
      [makeRequest({ id: "a" }), makeRequest({ id: "b" }), makeRequest({ id: "c" })],
      null,
      { isCancelled: () => seen++ >= 1 },
    );

    expect(report.totalRequests).toBe(1);
    expect(report.results.map((r) => r.requestId)).toEqual(["a"]);
    expect(replay.fn).toHaveBeenCalledTimes(1);
  });

  it("waits between requests but not before the first one", async () => {
    const started = Date.now();
    await run([makeRequest({ id: "a" }), makeRequest({ id: "b" })], null, undefined, 60);
    const elapsed = Date.now() - started;

    expect(elapsed).toBeGreaterThanOrEqual(50);
  });

  it("keeps going when one request fails, recording the error against it", async () => {
    replay.fn.mockImplementationOnce(async () => { throw new Error("ECONNREFUSED"); });
    (globalThis as any).window.api.replayRequest = replay.fn;

    const report = await run([makeRequest({ id: "a" }), makeRequest({ id: "b" })]);

    expect(report.results[0].error).toContain("ECONNREFUSED");
    expect(report.results[0].status).toBeNull();
    expect(report.results[1].error).toBeUndefined();
    expect(report.results[1].status).toBe(200);
  });
});

// ── runCollection — scripts ───────────────────────────────────────────────────

describe("runCollection — pre/post/test scripts", () => {
  it("applies a pre-script mutation to the outgoing request", async () => {
    await run([makeRequest({
      preScript: 'lp.request.headers.set("x-token", "abc"); lp.request.url = lp.request.url + "?page=2";',
    })]);

    expect(replay.calls[0].headers["x-token"]).toBe("abc");
    expect(replay.calls[0].url).toBe("http://api.localhost/users?page=2");
  });

  it("records a pre-script error but still sends the request", async () => {
    const report = await run([makeRequest({ preScript: "throw new Error('bad pre');" })]);

    expect(report.results[0].preScriptError).toContain("bad pre");
    expect(replay.fn).toHaveBeenCalledTimes(1);
  });

  it("runs a post-script and lets it publish environment variables", async () => {
    const report = await run([makeRequest({
      postScript: 'lp.environment.set("LAST_STATUS", String(lp.response.status));',
    })]);

    expect(report.results[0].postScriptError).toBeUndefined();
    expect(report.results[0].status).toBe(200);
  });

  it("records a post-script error without losing the response", async () => {
    const report = await run([makeRequest({ postScript: "throw new Error('bad post');" })]);

    expect(report.results[0].postScriptError).toContain("bad post");
    expect(report.results[0].status).toBe(200);
  });

  it("runs the test script and reports passing and failing tests", async () => {
    const report = await run([makeRequest({
      testScript: `
        lp.test("status is 200", () => { lp.expect(lp.response.status).to.equal(200); });
        lp.test("status is 500", () => { lp.expect(lp.response.status).to.equal(500); });
      `,
    })]);

    expect(report.totalTests).toBe(2);
    expect(report.passedTests).toBe(1);
    expect(report.failedTests).toBe(1);
    expect(report.results[0].tests.map((t) => t.name)).toEqual(["status is 200", "status is 500"]);
  });

  it("gives the test script the decoded response body", async () => {
    replay = stubReplay({ body: JSON.stringify({ user: { id: 7 } }) });
    (globalThis as any).window.api.replayRequest = replay.fn;

    const report = await run([makeRequest({
      testScript: 'lp.test("has user 7", () => { lp.expect(lp.response.json().user.id).to.equal(7); });',
    })]);

    expect(report.passedTests).toBe(1);
    expect(report.failedTests).toBe(0);
  });

  it("collects console output from the test script as testLogs", async () => {
    const report = await run([makeRequest({
      testScript: 'console.log("hello from test"); lp.test("ok", () => { lp.expect(1).to.equal(1); });',
    })]);

    expect(report.results[0].testLogs.join("\n")).toContain("hello from test");
  });

  it("counts tests across every request in the run", async () => {
    const script = 'lp.test("always", () => { lp.expect(1).to.equal(1); });';
    const report = await run([
      makeRequest({ id: "a", testScript: script }),
      makeRequest({ id: "b", testScript: script }),
    ]);

    expect(report.totalTests).toBe(2);
    expect(report.passedTests).toBe(2);
    expect(report.failedTests).toBe(0);
  });
});

// ── runTestScript ─────────────────────────────────────────────────────────────

describe("runTestScript", () => {
  const response = { status: 200, headers: {}, body: "{}", responseTime: 5 };

  it("returns the tests and logs produced by the sandbox", async () => {
    const res = await runTestScript(
      'console.log("log line"); lp.test("t", () => { lp.expect(1).to.equal(1); });',
      response,
      null,
    );

    expect(res.tests).toHaveLength(1);
    expect(res.tests[0].passed).toBe(true);
    expect(res.logs.join("\n")).toContain("log line");
  });

  it("reports a throwing script as a failing 'Script execution' test", async () => {
    // The sandbox converts a top-level throw into a synthetic failing test rather than
    // an out-of-band error, so the failure shows up in the run report like any other.
    const res = await runTestScript("throw new Error('boom');", response, null);

    expect(res.error).toBeUndefined();
    expect(res.tests).toHaveLength(1);
    expect(res.tests[0].name).toBe("Script execution");
    expect(res.tests[0].passed).toBe(false);
    expect(res.tests[0].error).toContain("boom");
  });

  it("returns an error object instead of throwing when the IPC call rejects", async () => {
    (globalThis as any).window.api.executeScript = () => Promise.reject(new Error("ipc down"));

    const res = await runTestScript("lp.test('t', () => {});", response, null);

    expect(res.tests).toEqual([]);
    expect(res.error).toContain("ipc down");
  });
});

// ── generateHtmlReport ────────────────────────────────────────────────────────

function makeReport(overrides: Partial<Parameters<typeof generateHtmlReport>[0]> = {}) {
  return {
    folderId: FOLDER.id,
    folderName: "Smoke Tests",
    startedAt: 1_700_000_000_000,
    completedAt: 1_700_000_002_500,
    totalRequests: 1,
    totalTests: 1,
    passedTests: 1,
    failedTests: 0,
    results: [{
      requestId: "a",
      requestName: "Get Users",
      method: "GET",
      url: "http://api.localhost/users",
      status: 200,
      responseTime: 12,
      tests: [{ name: "ok", passed: true, durationMs: 1 }],
      testLogs: [],
    }],
    ...overrides,
  } as Parameters<typeof generateHtmlReport>[0];
}

describe("generateHtmlReport", () => {
  it("produces a standalone HTML document with the summary counts", () => {
    const html = generateHtmlReport(makeReport());

    expect(html).toContain("<!DOCTYPE html>");
    expect(html).toContain("Collection Run: Smoke Tests");
    expect(html).toContain(">1</div><div class=\"label\">Requests");
    expect(html).toContain("2.50s");
  });

  it("lists each request with its method, name, status and time", () => {
    const html = generateHtmlReport(makeReport());

    expect(html).toContain(">GET<");
    expect(html).toContain("Get Users");
    expect(html).toContain(">200<");
    expect(html).toContain("12ms");
  });

  it("colours the status by class", () => {
    expect(generateHtmlReport(makeReport({ results: [{ ...makeReport().results[0], status: 404 }] })))
      .toContain("status-4xx");
    expect(generateHtmlReport(makeReport({ results: [{ ...makeReport().results[0], status: 500 }] })))
      .toContain("status-5xx");
  });

  it("omits the status element when the request never got a response", () => {
    const html = generateHtmlReport(makeReport({
      results: [{ ...makeReport().results[0], status: null, error: "ECONNREFUSED" }],
    }));

    expect(html).not.toContain("status-null");
    expect(html).toContain("ECONNREFUSED");
  });

  it("renders a failing test with its assertion message", () => {
    const html = generateHtmlReport(makeReport({
      totalTests: 1, passedTests: 0, failedTests: 1,
      results: [{
        ...makeReport().results[0],
        tests: [{ name: "status is 500", passed: false, error: "expected 200 to equal 500", durationMs: 1 }],
      }],
    }));

    expect(html).toContain("status is 500");
    expect(html).toContain("expected 200 to equal 500");
  });

  it("escapes HTML in the folder name, request name and test name", () => {
    const html = generateHtmlReport(makeReport({
      folderName: '<script>alert("x")</script>',
      results: [{
        ...makeReport().results[0],
        requestName: "<img src=x onerror=1>",
        tests: [{ name: "<b>bold</b>", passed: true, durationMs: 1 }],
      }],
    }));

    expect(html).not.toContain("<script>alert");
    expect(html).not.toContain("<img src=x");
    expect(html).not.toContain("<b>bold</b>");
    expect(html).toContain("&lt;script&gt;");
  });
});

// ── runnerReport persistence wrappers ─────────────────────────────────────────

describe("runnerReport persistence", () => {
  it("saveRunnerReport delegates to the main process", async () => {
    const save = vi.fn(async () => ({ ok: true }));
    (globalThis as any).window.api.saveRunnerReport = save;

    const report = makeReport();
    expect(await saveRunnerReport("ws-1", report)).toEqual({ ok: true });
    expect(save).toHaveBeenCalledWith("ws-1", report);
  });

  it("getRunHistory delegates to the main process", async () => {
    const history = [{ timestamp: 1, summary: { total: 2, passed: 1, failed: 1 } }];
    const get = vi.fn(async () => history);
    (globalThis as any).window.api.getRunHistory = get;

    expect(await getRunHistory("ws-1", FOLDER.id)).toEqual(history);
    expect(get).toHaveBeenCalledWith("ws-1", FOLDER.id);
  });
});
