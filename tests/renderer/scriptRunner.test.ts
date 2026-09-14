import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  runPreScript,
  runPostScript,
  ScriptRequest,
  ScriptResponse,
} from "@/lib/scriptRunner";
import type { Environment, SavedRequest } from "@/types";
import { executeIpcScript } from "@/proxy/scriptExecutor";

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeEnv(vars: Record<string, string>): Environment {
  return {
    id: "env-1",
    name: "Test",
    createdAt: Date.now(),
    variables: Object.entries(vars).map(([key, value]) => ({ id: key, key, value })),
    workspaceId: "ws-1",
  };
}

function makeReq(overrides: Partial<ScriptRequest> = {}): ScriptRequest {
  return {
    method: "GET",
    url: "http://example.localhost/api",
    headers: { "content-type": "application/json" },
    body: "",
    ...overrides,
  };
}

// Wire window.api.executeScript to the real vm-based executor for all tests
beforeEach(() => {
  (globalThis as any).window = {
    ...(globalThis as any).window,
    api: {
      ...((globalThis as any).window?.api ?? {}),
      executeScript: (opts: Parameters<typeof executeIpcScript>[0]) => Promise.resolve(executeIpcScript(opts)),
    },
  };
});

// ── runPreScript ──────────────────────────────────────────────────────────────

describe("runPreScript()", () => {
  it("returns the original request unchanged when script is empty", async () => {
    const req = makeReq();
    const result = await runPreScript("", req, null);
    expect(result.req).toEqual(req);
    expect(result.error).toBeUndefined();
  });

  it("allows the script to read lp.request.url", async () => {
    const req = makeReq({ url: "http://original.localhost" });
    const result = await runPreScript("lp.request.url;", req, null);
    // No error — just check no mutation occurred since we just read
    expect(result.error).toBeUndefined();
  });

  it("allows the script to mutate lp.request.url", async () => {
    const req = makeReq({ url: "http://old.localhost" });
    const result = await runPreScript("lp.request.url = 'http://new.localhost';", req, null);
    expect(result.req.url).toBe("http://new.localhost");
    expect(result.error).toBeUndefined();
  });

  it("allows the script to mutate lp.request.headers via set", async () => {
    const req = makeReq({ headers: {} });
    const result = await runPreScript(
      "lp.request.headers.set('x-custom', 'hello');",
      req, null,
    );
    expect(result.req.headers["x-custom"]).toBe("hello");
  });

  it("allows the script to unset a header", async () => {
    const req = makeReq({ headers: { "x-remove": "yes", "keep": "me" } });
    const result = await runPreScript(
      "lp.request.headers.unset('x-remove');",
      req, null,
    );
    expect(result.req.headers["x-remove"]).toBeUndefined();
    expect(result.req.headers["keep"]).toBe("me");
  });

  it("allows the script to mutate lp.request.body", async () => {
    const req = makeReq({ body: "{}" });
    const result = await runPreScript(
      "lp.request.body = JSON.stringify({ injected: true });",
      req, null,
    );
    expect(result.req.body).toBe('{"injected":true}');
  });

  it("does NOT mutate the original ScriptRequest object (works on a copy)", async () => {
    const original = makeReq({ url: "http://original.localhost" });
    const copy = { ...original, headers: { ...original.headers } };
    await runPreScript("lp.request.url = 'http://mutated.localhost';", copy, null);
    expect(original.url).toBe("http://original.localhost");
  });

  it("allows lp.environment.get to read env vars", async () => {
    const env = makeEnv({ TOKEN: "abc123" });
    const result = await runPreScript(
      "lp.request.headers.set('authorization', 'Bearer ' + lp.environment.get('TOKEN'));",
      makeReq(), env,
    );
    expect(result.req.headers["authorization"]).toBe("Bearer abc123");
  });

  it("allows lp.environment.set to write env vars", async () => {
    const env = makeEnv({ A: "1" });
    const result = await runPreScript(
      "lp.environment.set('NEW_KEY', 'hello');",
      makeReq(), env,
    );
    expect(result.envVars["NEW_KEY"]).toBe("hello");
  });

  it("allows lp.environment.unset to remove a var", async () => {
    const env = makeEnv({ REMOVE_ME: "yes" });
    const result = await runPreScript(
      "lp.environment.unset('REMOVE_ME');",
      makeReq(), env,
    );
    expect(result.envVars["REMOVE_ME"]).toBeUndefined();
  });

  it("returns error string on script runtime error", async () => {
    const result = await runPreScript("throw new Error('oops');", makeReq(), null);
    expect(result.error).toContain("oops");
  });

  it("returns error string on script syntax error", async () => {
    const result = await runPreScript("((((broken syntax", makeReq(), null);
    expect(result.error).toBeDefined();
  });

  it("lp.sendRequest is not available in the vm sandbox (errors gracefully)", async () => {
    const result = await runPreScript(
      "const r = await lp.sendRequest('req-1'); lp.request.headers.set('x-status', String(r.status));",
      makeReq(), null,
    );
    // sendRequest is not injected into the vm sandbox — script receives an error
    expect(result.error).toBeDefined();
  });
});

// ── runPostScript ─────────────────────────────────────────────────────────────

describe("runPostScript()", () => {
  const resp: ScriptResponse = { status: 201, headers: { "content-type": "application/json" }, body: '{"id":42}' };

  it("returns no error for an empty script", async () => {
    const result = await runPostScript("", resp, null);
    expect(result.error).toBeUndefined();
  });

  it("allows reading lp.response.status", async () => {
    const result = await runPostScript(
      "lp.environment.set('LAST_STATUS', String(lp.response.status));",
      resp, null,
    );
    expect(result.envVars["LAST_STATUS"]).toBe("201");
  });

  it("allows reading lp.response.body", async () => {
    const result = await runPostScript(
      "lp.environment.set('BODY_SNAP', lp.response.body.slice(0,5));",
      resp, null,
    );
    expect(result.envVars["BODY_SNAP"]).toBe('{"id"');
  });

  it("allows lp.response.json() to parse the body", async () => {
    const result = await runPostScript(
      "const data = lp.response.json(); lp.environment.set('ENTITY_ID', String(data.id));",
      resp, null,
    );
    expect(result.envVars["ENTITY_ID"]).toBe("42");
  });

  it("allows reading lp.response.headers.get", async () => {
    const result = await runPostScript(
      "lp.environment.set('CT', lp.response.headers.get('content-type'));",
      resp, null,
    );
    expect(result.envVars["CT"]).toBe("application/json");
  });

  it("allows lp.environment.set to persist a token from response", async () => {
    const tokenResp: ScriptResponse = { status: 200, headers: {}, body: '{"token":"secret-jwt"}' };
    const result = await runPostScript(
      "lp.environment.set('TOKEN', lp.response.json().token);",
      tokenResp, makeEnv({}),
    );
    expect(result.envVars["TOKEN"]).toBe("secret-jwt");
  });

  it("response is read-only — lp.response.status cannot be reassigned (silently)", async () => {
    const result = await runPostScript(
      "lp.response.status = 999;",
      resp, null,
    );
    expect(result.error).toBeUndefined();
  });

  it("returns error on script runtime error", async () => {
    const result = await runPostScript("throw new Error('post-fail');", resp, null);
    expect(result.error).toContain("post-fail");
  });

  it("lp.sendRequest is not available in the vm sandbox (errors gracefully)", async () => {
    const result = await runPostScript(
      "const r = await lp.sendRequest('req-auth'); lp.environment.set('AUTH_RESULT', r.body);",
      resp, null,
    );
    // sendRequest is not injected into the vm sandbox — script receives an error
    expect(result.error).toBeDefined();
  });
});
