import { describe, it, expect } from "vitest";
import {
    executeRequestScript,
    executeResponseScript,
    executeIpcScript,
} from "@bifurc/engine/proxy/scriptExecutor";

/**
 * Proxy-rule intercept scripts and request pre/post/test scripts all run inside
 * this sandbox. Two properties matter and are both invisible in a smoke test:
 *
 *   1. mutation actually reaches the caller (a script that "sets a header" must
 *      change the headers object the proxy will forward), and
 *   2. the sandbox is closed — user scripts must not be able to reach `require`,
 *      `process` or `globalThis` escape hatches.
 */

describe("executeRequestScript", () => {
    it("returns the headers and body unchanged for an empty script", () => {
        const res = executeRequestScript("", { "x-a": "1" }, "body");
        expect(res.headers).toEqual({ "x-a": "1" });
        expect(res.body).toBe("body");
        expect(res.error).toBeUndefined();
    });

    it("returns a NEW headers object — it does not mutate the caller's object", () => {
        const headers = { "x-a": "1" };
        const res = executeRequestScript(`lp.request.headers["x-injected"] = "yes";`, headers, "body");
        expect(res.headers["x-injected"]).toBe("yes");
        // Documented contract: the input is copied, so callers must use the
        // returned object. `fetchUpstreamResponse` does exactly that
        // (`finalHeaders = result.headers`).
        expect(headers).toEqual({ "x-a": "1" });
        expect(res.headers).not.toBe(headers);
        expect(res.error).toBeUndefined();
    });

    it("overwrites and deletes headers", () => {
        const res = executeRequestScript(
            `lp.request.headers["x-a"] = "2"; delete lp.request.headers["x-b"];`,
            { "x-a": "1", "x-b": "keep" },
            "",
        );
        expect(res.headers["x-a"]).toBe("2");
        expect(res.headers["x-b"]).toBeUndefined();
    });

    it("replaces the body", () => {
        const res = executeRequestScript(`lp.request.body = JSON.stringify({ rewritten: true });`, {}, `{"old":1}`);
        expect(JSON.parse(res.body)).toEqual({ rewritten: true });
    });

    it("can read the current body", () => {
        const res = executeRequestScript(`lp.request.body = lp.request.body.toUpperCase();`, {}, "abc");
        expect(res.body).toBe("ABC");
    });

    it("reports an error and keeps the mutations made before it threw", () => {
        const res = executeRequestScript(
            `lp.request.headers["x-before"] = "1"; throw new Error("boom");`,
            {},
            "body",
        );
        // Errors constructed INSIDE the vm realm are not `instanceof` the host
        // realm's Error, so the executor falls back to `String(e)` and the message
        // arrives with an "Error: " prefix. Assertion errors thrown by `lp.expect`
        // do not have this prefix because `buildExpect` is a host function.
        // See TESTING.md §6 for the (cosmetic) improvement note.
        expect(res.error).toBe("Error: boom");
        expect(res.headers["x-before"]).toBe("1");
        expect(res.body).toBe("body");
    });

    it("does not expose the response object to a request script", () => {
        const res = executeRequestScript(`lp.response.body = "nope";`, {}, "body");
        expect(res.error).toBeTruthy();
        expect(res.body).toBe("body");
    });
});

describe("executeResponseScript", () => {
    it("replaces the response body", () => {
        const res = executeResponseScript(`lp.response.body = "rewritten";`, {}, "original");
        expect(res.body).toBe("rewritten");
        expect(res.error).toBeUndefined();
    });

    it("mutates response headers", () => {
        const res = executeResponseScript(`lp.response.headers["x-new"] = "1";`, { "x-old": "a" }, "");
        expect(res.headers).toEqual({ "x-old": "a", "x-new": "1" });
    });

    it("can parse and re-serialise a JSON body", () => {
        const res = executeResponseScript(
            `const d = JSON.parse(lp.response.body); d.added = true; lp.response.body = JSON.stringify(d);`,
            {},
            `{"a":1}`,
        );
        expect(JSON.parse(res.body)).toEqual({ a: 1, added: true });
    });

    it("reports an error when the body is not valid JSON", () => {
        const res = executeResponseScript(`JSON.parse(lp.response.body);`, {}, "not-json");
        expect(res.error).toBeTruthy();
        expect(res.body).toBe("not-json");
    });
});

describe("sandbox isolation", () => {
    it("does not expose require, process or module to proxy scripts", () => {
        const res = executeRequestScript(
            `lp.request.headers["probe"] = [typeof require, typeof process, typeof module, typeof globalThis.process].join(",");`,
            {},
            "",
        );
        // `globalThis` exists in a vm context, but `process` must not be reachable
        // through it, and require/module must be entirely undefined.
        expect(res.headers["probe"]).toBe("undefined,undefined,undefined,undefined");
    });

    it("exposes a console for debugging but no process/require escape hatches", () => {
        // Observed: Node gives a fresh vm context a `console` that writes to the
        // app's stdout, so `typeof console` is "object". That is only noise, not an
        // escape — assert the actual capability boundary instead.
        const res = executeResponseScript(
            `lp.response.body = [typeof console, typeof require, typeof process, typeof module].join(",");`,
            {},
            "",
        );
        expect(res.body).toBe("object,undefined,undefined,undefined");
    });

    it("cannot reach the host process through globalThis", () => {
        const res = executeResponseScript(
            `lp.response.body = [typeof globalThis.process, typeof globalThis.require].join(",");`,
            {},
            "",
        );
        expect(res.body).toBe("undefined,undefined");
    });
});

describe("executeIpcScript — pre context", () => {
    const base = {
        script: "",
        context: "pre" as const,
        request: { method: "GET", url: "http://a.test/x", headers: { "x-a": "1" }, body: "" },
        envVars: { TOKEN: "t1" },
    };

    it("returns the request and env untouched for an empty script", () => {
        const res = executeIpcScript(base);
        expect(res.request!.url).toBe("http://a.test/x");
        expect(res.envVars).toEqual({ TOKEN: "t1" });
        expect(res.error).toBeUndefined();
    });

    it("can rewrite url, method and body", () => {
        const res = executeIpcScript({
            ...base,
            script: `lp.request.url = "http://b.test/y"; lp.request.method = "POST"; lp.request.body = "{}";`,
        });
        expect(res.request!.url).toBe("http://b.test/y");
        expect(res.request!.method).toBe("POST");
        expect(res.request!.body).toBe("{}");
    });

    it("can get/set/unset headers and read them all via toObject()", () => {
        const res = executeIpcScript({
            ...base,
            script: `
                lp.request.headers.set("x-new", "n");
                lp.request.headers.unset("x-a");
                lp.request.headers.set("x-count", String(Object.keys(lp.request.headers.toObject()).length));
            `,
        });
        expect(res.request!.headers["x-new"]).toBe("n");
        expect(res.request!.headers["x-a"]).toBeUndefined();
        expect(res.request!.headers["x-count"]).toBe("1");
    });

    it("can read and write environment variables", () => {
        const res = executeIpcScript({
            ...base,
            script: `lp.environment.set("DERIVED", lp.environment.get("TOKEN") + "-suffix");`,
        });
        expect(res.envVars).toEqual({ TOKEN: "t1", DERIVED: "t1-suffix" });
    });

    it("can unset an environment variable", () => {
        const res = executeIpcScript({ ...base, script: `lp.environment.unset("TOKEN");` });
        expect(res.envVars).toEqual({});
    });

    it("coerces non-string environment values to strings", () => {
        const res = executeIpcScript({ ...base, script: `lp.environment.set("N", 42);` });
        expect(res.envVars["N"]).toBe("42");
    });

    it("reports an error when the script throws", () => {
        const res = executeIpcScript({ ...base, script: `throw new Error("pre-boom");` });
        // Cross-realm Error ⇒ `String(e)` ⇒ "Error: " prefix (see the matching
        // note in the executeRequestScript suite).
        expect(res.error).toBe("Error: pre-boom");
        expect(res.request!.url).toBe("http://a.test/x");
    });
});

describe("executeIpcScript — post context", () => {
    const base = {
        script: "",
        context: "post" as const,
        response: { status: 200, headers: { "content-type": "application/json" }, body: `{"a":1}` },
        envVars: {},
    };

    it("exposes status, body, headers and json()", () => {
        const res = executeIpcScript({
            ...base,
            script: `lp.environment.set("SEEN", lp.response.status + ":" + String(lp.response.json().a));`,
        });
        expect(res.envVars["SEEN"]).toBe("200:1");
        expect(res.error).toBeUndefined();
    });

    it("returns null from json() for a non-JSON body", () => {
        const res = executeIpcScript({
            ...base,
            response: { ...base.response, body: "plain" },
            script: `lp.environment.set("J", String(lp.response.json()));`,
        });
        expect(res.envVars["J"]).toBe("null");
    });

    it("exposes headers via get() and toObject()", () => {
        const res = executeIpcScript({
            ...base,
            script: `lp.environment.set("CT", lp.response.headers.get("content-type") + "/" + String(Object.keys(lp.response.headers.toObject()).length));`,
        });
        expect(res.envVars["CT"]).toBe("application/json/1");
    });
});

describe("executeIpcScript — test context", () => {
    const base = {
        context: "test" as const,
        response: { status: 200, headers: { "content-type": "application/json" }, body: `{"a":1}`, responseTime: 12 },
        envVars: { TOKEN: "t1" },
    };

    it("records a passing test", () => {
        const res = executeIpcScript({
            ...base,
            script: `lp.test("status is 200", () => { lp.expect(lp.response.status).to.equal(200); });`,
        });
        expect(res.testResults).toHaveLength(1);
        expect(res.testResults![0].passed).toBe(true);
        expect(res.testResults![0].name).toBe("status is 200");
    });

    it("records a failing test with the assertion message", () => {
        const res = executeIpcScript({
            ...base,
            script: `lp.test("status is 500", () => { lp.expect(lp.response.status).to.equal(500); });`,
        });
        expect(res.testResults![0].passed).toBe(false);
        expect(res.testResults![0].error).toContain("500");
    });

    it("keeps running after a failing test", () => {
        const res = executeIpcScript({
            ...base,
            script: `
                lp.test("fails", () => { lp.expect(1).to.equal(2); });
                lp.test("passes", () => { lp.expect(1).to.equal(1); });
            `,
        });
        expect(res.testResults!.map((t) => t.passed)).toEqual([false, true]);
    });

    it("exposes responseTime, text() and header lookups", () => {
        const res = executeIpcScript({
            ...base,
            script: `
                lp.test("meta", () => {
                    lp.expect(lp.response.responseTime).to.be.above(0);
                    lp.expect(lp.response.text()).to.include("a");
                    lp.expect(lp.response.headers.has("Content-Type")).to.be.true;
                });
            `,
        });
        expect(res.testResults![0].passed).toBe(true);
    });

    it("exposes a read-only environment", () => {
        const res = executeIpcScript({
            ...base,
            script: `
                lp.test("env", () => {
                    lp.expect(lp.environment.get("TOKEN")).to.equal("t1");
                    lp.expect(lp.environment.has("TOKEN")).to.be.true;
                    lp.expect(lp.environment.has("NOPE")).to.be.false;
                });
            `,
        });
        expect(res.testResults![0].passed).toBe(true);
    });

    it("captures console output", () => {
        const res = executeIpcScript({
            ...base,
            script: `console.log("hello"); console.warn("careful");`,
        });
        expect(res.testLogs).toEqual(["hello", "[WARN] careful"]);
    });

    it("reports a top-level script error as a failed pseudo-test", () => {
        const res = executeIpcScript({ ...base, script: `throw new Error("top-level");` });
        expect(res.testResults).toHaveLength(1);
        expect(res.testResults![0].passed).toBe(false);
        expect(res.testResults![0].error).toContain("top-level");
    });

    it("appends an uncaught error to the logs when tests already ran", () => {
        const res = executeIpcScript({
            ...base,
            script: `
                lp.test("first", () => { lp.expect(1).to.equal(1); });
                throw new Error("late-boom");
            `,
        });
        expect(res.testResults![0].passed).toBe(true);
        expect(res.testLogs!.some((l) => l.includes("late-boom"))).toBe(true);
    });
});

describe("executeIpcScript — invalid input", () => {
    it("returns an error when the context does not match the payload", () => {
        const res = executeIpcScript({ script: "", context: "pre", envVars: {} });
        expect(res.error).toBe("Invalid script context or missing request/response");
    });
});
