import { describe, it, expect, vi, beforeEach } from "vitest";

/**
 * The pipeline is the one place the request lifecycle is implemented (`plan/19` D24), so its suite is
 * where the phase's two *invisible* traps get their assertions — `plan/19` §10: "**Traps 1 and 2** get a
 * test each, because both are invisible in a suite that uses no shadowed variable."
 *
 * The workspace is mocked at the two modules `workspace/envState.ts` reads rather than at `config.ts`:
 * `envState` deliberately does **not** use `loadWorkspaceDispatchSlice()` (that would write `enabled.json`
 * as a side effect of reading an environment), so mocking `config.ts` would not reach it. Mocking the
 * leaves keeps the real merge, the real resolver and the **real sandbox** in the test — which is what
 * makes these assertions about behaviour rather than about a stub.
 */
vi.mock("@bifurc/engine/workspace/fs/fsRead", () => ({ readAllEntities: vi.fn(() => []) }));
vi.mock("@bifurc/engine/workspace/appSettings", () => ({ loadSettings: vi.fn(() => ({ workspaces: [] })) }));

import { readAllEntities } from "@bifurc/engine/workspace/fs/fsRead";
import { loadSettings } from "@bifurc/engine/workspace/appSettings";
import {
    runRequestPipeline,
    type PipelineRequest,
    type PipelineRawResponse,
} from "@bifurc/engine/rest/runPipeline";

const WS = "ws-1";

function makeEnv(id: string, vars: Record<string, string>) {
    return {
        id,
        name: id,
        variables: Object.entries(vars).map(([key, value]) => ({ id: key, key, value })),
        createdAt: 0,
        workspaceId: WS,
    };
}

/** Point `loadMergedEnvironment()` at a fixed environment set. */
function useEnvironments(envs: ReturnType<typeof makeEnv>[], activeEnvironmentId: string | null = null) {
    vi.mocked(readAllEntities).mockReturnValue(envs as never);
    vi.mocked(loadSettings).mockReturnValue({
        workspaces: [{ id: WS, name: WS, activeEnvironmentId }],
    } as never);
}

function request(overrides: Partial<PipelineRequest> = {}): PipelineRequest {
    return { method: "GET", url: "https://example.com", headers: {}, body: "", ...overrides };
}

/** A response whose body is base64, which is the wire shape the pipeline must decode for scripts. */
function response(body: string, overrides: Partial<PipelineRawResponse> = {}): PipelineRawResponse {
    return {
        status: 200,
        headers: { "content-type": "text/plain" },
        body: Buffer.from(body, "utf-8").toString("base64"),
        ...overrides,
    };
}

beforeEach(() => {
    vi.mocked(readAllEntities).mockReset();
    vi.mocked(readAllEntities).mockReturnValue([] as never);
    vi.mocked(loadSettings).mockReset();
    vi.mocked(loadSettings).mockReturnValue({ workspaces: [] } as never);
});

describe("runRequestPipeline — resolution", () => {
    it("resolves the url, the header values and the body before sending", async () => {
        useEnvironments([makeEnv("__global__", { host: "example.com", token: "t0" })], "__global__");
        const send = vi.fn(async () => response("ok"));

        await runRequestPipeline({
            workspaceId: WS,
            request: request({
                url: "https://{{host}}/v1",
                headers: { authorization: "Bearer {{token}}" },
                body: '{"who":"{{token}}"}',
            }),
            send,
        });

        expect(send).toHaveBeenCalledTimes(1);
        const [resolved] = send.mock.calls[0];
        expect(resolved.url).toBe("https://example.com/v1");
        expect(resolved.headers.authorization).toBe("Bearer t0");
        expect(resolved.body).toBe('{"who":"t0"}');
    });

    /**
     * `plan/19` §8 trap 2, and the assertion §10 asks for by name.
     *
     * The renderer merged `__global__` with the active environment (`App.tsx:288`) and the engine must
     * agree. Resolving against the **active** environment alone produces `active.example` here only by
     * accident of which list is searched first; the case that separates the two implementations is a key
     * present in **both**, where the active value must win — and a key present in **only** one, where both
     * must still be reachable. A pipeline that resolved against one environment would pass every test
     * above and fail this one.
     */
    it("resolves a shadowed key to the ACTIVE value and still reaches a global-only key", async () => {
        useEnvironments(
            [
                makeEnv("__global__", { host: "global.example", onlyGlobal: "g" }),
                makeEnv("active", { host: "active.example", onlyActive: "a" }),
            ],
            "active",
        );
        const send = vi.fn(async () => response("ok"));

        await runRequestPipeline({
            workspaceId: WS,
            request: request({
                url: "https://{{host}}/{{onlyGlobal}}/{{onlyActive}}",
            }),
            send,
        });

        expect(send.mock.calls[0][0].url).toBe("https://active.example/g/a");
    });

    it("leaves tokens verbatim when the workspace has no environment at all", async () => {
        useEnvironments([]);
        const send = vi.fn(async () => response("ok"));

        await runRequestPipeline({
            workspaceId: WS,
            request: request({ url: "https://{{host}}/v1" }),
            send,
        });

        expect(send.mock.calls[0][0].url).toBe("https://{{host}}/v1");
    });
});

describe("runRequestPipeline — the body encoding boundary (plan/19 §8 trap 4)", () => {
    it("hands the send the body as TEXT, not base64", async () => {
        useEnvironments([]);
        const send = vi.fn(async () => response("ok"));

        await runRequestPipeline({
            workspaceId: WS,
            request: request({ method: "POST", body: '{"a":1}' }),
            send,
        });

        // `request.replay` takes base64; the pipeline takes text. The REST `send` closure encodes, and
        // this asserts the pipeline did **not** — the pair trap 4 wants asserted rather than described.
        expect(send.mock.calls[0][0].body).toBe('{"a":1}');
    });

    it("decodes the response body to text for the post-script and the test-script", async () => {
        useEnvironments([]);
        // Both scripts throw unless they were handed decoded text, so a `undefined` error field is the
        // assertion — there is no other channel through which a post-script can report what it saw.
        const postScript = 'if (lp.response.body !== "hello") throw new Error("post saw: " + lp.response.body);';
        const testScript =
            'lp.test("body is text", function () { if (lp.response.body !== "hello") { throw new Error("saw " + lp.response.body); } });';

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request(),
            scripts: { post: postScript, test: testScript },
            send: async () => response("hello"),
        });

        expect(result.postScriptError).toBeUndefined();
        expect(result.tests).toHaveLength(1);
        expect(result.tests[0].passed).toBe(true);
        // The response itself keeps base64 — the renderer's `b64ToText` stays, and stays presentation.
        expect(result.response.body).toBe(Buffer.from("hello", "utf-8").toString("base64"));
    });
});

describe("runRequestPipeline — script orchestration", () => {
    it("runs the pre-script against the RESOLVED request, not the raw one", async () => {
        useEnvironments([makeEnv("__global__", { host: "example.com" })], "__global__");
        const preScript =
            'if (lp.request.url !== "https://example.com/v1") throw new Error("pre saw: " + lp.request.url);';

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request({ url: "https://{{host}}/v1" }),
            scripts: { pre: preScript },
            send: async () => response("ok"),
        });

        expect(result.preScriptError).toBeUndefined();
    });

    it("lets the pre-script rewrite the request the send receives", async () => {
        useEnvironments([]);
        const send = vi.fn(async () => response("ok"));

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request({ url: "https://example.com/v1" }),
            scripts: { pre: 'lp.request.url = lp.request.url + "/v2"; lp.request.method = "POST";' },
            send,
        });

        expect(send.mock.calls[0][0].url).toBe("https://example.com/v1/v2");
        expect(send.mock.calls[0][0].method).toBe("POST");
        // The result reports what was *actually sent*, which is what a UI has to show to explain the
        // response it is displaying.
        expect(result.request.url).toBe("https://example.com/v1/v2");
    });

    it("lets the pre-script's environment mutation reach the post-script", async () => {
        useEnvironments([makeEnv("__global__", { token: "t0" })], "__global__");
        const postScript =
            'if (lp.environment.get("token") !== "from-pre") throw new Error("post saw: " + lp.environment.get("token"));';

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request(),
            scripts: { pre: 'lp.environment.set("token", "from-pre");', post: postScript },
            send: async () => response("ok"),
        });

        expect(result.postScriptError).toBeUndefined();
    });

    /**
     * The preserved oddity, asserted so that "fix" it is a deliberate change rather than a silent one.
     * Both renderer copies guarded the hand-off with `if (activeEnv && …)`; this reproduces that.
     */
    it("drops a pre-script's environment mutation when the workspace has no environment", async () => {
        useEnvironments([]);
        const postScript =
            'if (lp.environment.get("token") !== undefined) throw new Error("post unexpectedly saw: " + lp.environment.get("token"));';

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request(),
            scripts: { pre: 'lp.environment.set("token", "from-pre");', post: postScript },
            send: async () => response("ok"),
        });

        expect(result.postScriptError).toBeUndefined();
    });

    it("records a pre-script error and sends anyway", async () => {
        useEnvironments([]);
        const send = vi.fn(async () => response("ok"));

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request(),
            scripts: { pre: 'throw new Error("boom");' },
            send,
        });

        expect(result.preScriptError).toContain("boom");
        expect(send).toHaveBeenCalledTimes(1);
    });

    it("does not run a whitespace-only script", async () => {
        useEnvironments([]);

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request(),
            scripts: { pre: "   ", post: "\n", test: "\t" },
            send: async () => response("ok"),
        });

        expect(result.preScriptError).toBeUndefined();
        expect(result.postScriptError).toBeUndefined();
        expect(result.tests).toEqual([]);
    });

    it("returns the test-script's results and its console logs", async () => {
        useEnvironments([]);
        const testScript = [
            'lp.test("passes", function () {});',
            'lp.test("fails", function () { throw new Error("nope"); });',
            'console.log("from the script");',
        ].join("\n");

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request(),
            scripts: { test: testScript },
            send: async () => response("ok"),
        });

        expect(result.tests.map((t) => [t.name, t.passed])).toEqual([
            ["passes", true],
            ["fails", false],
        ]);
        expect(result.testLogs.join("\n")).toContain("from the script");
    });

    it("reports the send's duration to the test-script as responseTime", async () => {
        useEnvironments([]);
        const testScript =
            'lp.test("has a responseTime", function () { if (typeof lp.response.responseTime !== "number") { throw new Error("missing"); } });';

        const result = await runRequestPipeline({
            workspaceId: WS,
            request: request(),
            scripts: { test: testScript },
            send: async () => response("ok"),
        });

        expect(result.tests[0].passed).toBe(true);
        expect(typeof result.durationMs).toBe("number");
    });
});

describe("runRequestPipeline — failure", () => {
    it("propagates a send failure, because the caller decides what it means", async () => {
        useEnvironments([]);

        await expect(
            runRequestPipeline({
                workspaceId: WS,
                request: request(),
                send: async () => {
                    throw new Error("ECONNREFUSED");
                },
            }),
        ).rejects.toThrow("ECONNREFUSED");
    });

    it("does not run the post-script or the test-script when the send fails", async () => {
        useEnvironments([]);
        const postScript = 'throw new Error("post should not run");';
        const testScript = 'lp.test("should not run", function () { throw new Error("x"); });';

        await expect(
            runRequestPipeline({
                workspaceId: WS,
                request: request(),
                scripts: { post: postScript, test: testScript },
                send: async () => {
                    throw new Error("ECONNREFUSED");
                },
            }),
        ).rejects.toThrow("ECONNREFUSED");
    });
});
