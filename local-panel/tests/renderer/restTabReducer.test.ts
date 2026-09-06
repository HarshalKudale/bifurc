import { describe, it, expect } from "vitest";
import {
  tabReducer, initState, stateToSavePayload, stateToDraft, isDraftEmpty,
  TabState, TabType,
} from "@/components/rest/restTabReducer";
import type { SavedRequest, MockRule } from "@/types";

// ── Fixtures ───────────────────────────────────────────────────────────────

const REQ: SavedRequest = {
  id: "req-1", workspaceId: "ws-1", createdAt: 1000,
  name: "Get User", method: "GET", url: "https://api.example.com/user",
  headers: { "authorization": "Bearer token", "content-type": "application/json" },
  body: '{"test":true}',
  preScript: "pm.environment.set('x', 1);",
  postScript: "pm.test('ok', () => true);",
  folderId: "folder-1",
};

const MOCK: MockRule = {
  id: "mock-1", workspaceId: "ws-1", createdAt: 1000,
  name: "User Mock", method: "GET", urlPattern: "/api/user",
  useRegex: false, enabled: true,
  capturedHeaders: { "content-type": "application/json" },
  capturedBody: btoa('{"input":1}'),
  responseStatus: 200,
  responseStatusMocked: true,
  responseHeaders: { "content-type": "application/json" },
  mockedResponseHeaders: [],
  responseBody: '{"mocked":true}',
  responseBodyMocked: true,
  responseDelayMocked: true,
  folderId: null,
};

// ── initState ──────────────────────────────────────────────────────────────

describe("initState()", () => {
  it("returns blank state for request when no entity/draft", () => {
    const s = initState(null, null, "request");
    expect(s.method).toBe("GET");
    expect(s.url).toBe("");
    expect(s.name).toBe("");
    expect(s.loading).toBe(false);
  });

  it("returns * method for mock when no entity/draft", () => {
    const s = initState(null, null, "mock");
    expect(s.method).toBe("*");
  });

  it("loads from SavedRequest entity", () => {
    const s = initState(REQ, null, "request");
    expect(s.name).toBe("Get User");
    expect(s.method).toBe("GET");
    expect(s.url).toBe("https://api.example.com/user");
    expect(s.folderId).toBe("folder-1");
    expect(s.preScript).toBe("pm.environment.set('x', 1);");
    expect(s.postScript).toBe("pm.test('ok', () => true);");
    expect(s.reqHeaders.some((r) => r.key === "authorization")).toBe(true);
  });

  it("loads from MockRule entity", () => {
    const s = initState(MOCK, null, "mock");
    expect(s.name).toBe("User Mock");
    expect(s.url).toBe("/api/user");
    expect(s.resStatus).toBe(200);
    expect(s.resBody).toBe('{\n  "mocked": true\n}');
    expect(s.resHeaders.some((r) => r.key === "content-type")).toBe(true);
  });

  it("draft takes priority over entity for request", () => {
    const draft = {
      name: "Draft Name", method: "POST", url: "https://other.com",
      folderId: null, headers: {}, body: "body", reqMode: "text" as const,
      preScript: "", postScript: "",
    };
    const s = initState(REQ, draft, "request");
    expect(s.name).toBe("Draft Name");
    expect(s.method).toBe("POST");
    expect(s.url).toBe("https://other.com");
  });

  it("draft takes priority over entity for mock", () => {
    const draft = {
      name: "Mock Draft", method: "*", urlPattern: "/draft",
      useRegex: true, folderId: null,
      reqHeaders: {}, reqBody: "", reqMode: "json" as const,
      resStatus: 404, resStatusMocked: true, resHeaders: {}, mockedResponseHeaders: [], resBody: "not found", resBodyMocked: true, resMode: "text" as const,
      resDelayMocked: true,
    };
    const s = initState(MOCK, draft, "mock");
    expect(s.name).toBe("Mock Draft");
    expect(s.url).toBe("/draft");
    expect(s.useRegex).toBe(true);
    expect(s.resStatus).toBe(404);
    expect(s.resBody).toBe("not found");
  });
});

// ── LOAD_ENTITY ────────────────────────────────────────────────────────────

describe("LOAD_ENTITY action", () => {
  it("resets volatile state (loading=false) when loading entity", () => {
    const s0 = initState(null, null, "request");
    // Simulate in-flight state
    const mid: TabState = { ...s0, loading: true, sendErr: "oops" };
    const s1 = tabReducer(mid, { type: "LOAD_ENTITY", entity: REQ, tabType: "request" });
    expect(s1.loading).toBe(false);
    expect(s1.sendErr).toBe(null);
    expect(s1.url).toBe(REQ.url);
  });
});

// ── REFRESH action (key innovation: preserves volatile state) ──────────────

describe("REFRESH action", () => {
  it("updates entity fields while preserving loading/result state", () => {
    const s0 = initState(REQ, null, "request");
    // User has sent a request — runtime state is live
    const withResult: TabState = {
      ...s0,
      loading: false,
      result: { status: 200, headers: {}, body: btoa("ok") },
      sendErr: null,
    };
    const updatedReq: SavedRequest = { ...REQ, name: "Updated Name", url: "https://new.example.com" };
    const s1 = tabReducer(withResult, { type: "REFRESH", entity: updatedReq, tabType: "request" });
    // Entity fields updated
    expect(s1.name).toBe("Updated Name");
    expect(s1.url).toBe("https://new.example.com");
    // Volatile state preserved
    expect(s1.result).not.toBeNull();
    expect(s1.result?.status).toBe(200);
    expect(s1.loading).toBe(false);
  });

  it("updates mock entity fields while preserving test state", () => {
    const s0 = initState(MOCK, null, "mock");
    const withTest: TabState = { ...s0, testLoading: false, testError: "some error" };
    const updatedMock: MockRule = { ...MOCK, responseBody: '{"updated":true}' };
    const s1 = tabReducer(withTest, { type: "REFRESH", entity: updatedMock, tabType: "mock" });
    expect(s1.resBody).toBe('{\n  "updated": true\n}');
    // testError preserved since REFRESH only updates entity-derived fields
    expect(s1.testError).toBe("some error");
  });
});

// ── SET_FIELD ──────────────────────────────────────────────────────────────

describe("SET_FIELD action", () => {
  it("updates a single field", () => {
    const s0 = initState(null, null, "request");
    const s1 = tabReducer(s0, { type: "SET_FIELD", field: "name", value: "My Request" });
    expect(s1.name).toBe("My Request");
    expect(s1.url).toBe("");
  });
});

// ── SET_REQ_MODE ───────────────────────────────────────────────────────────

describe("SET_REQ_MODE action", () => {
  it("inserts content-type header when switching to json", () => {
    const s0 = initState(null, null, "request");
    const s1 = tabReducer(s0, { type: "SET_REQ_MODE", mode: "json" });
    expect(s1.reqMode).toBe("json");
    const ct = s1.reqHeaders.find((r) => r.key === "content-type");
    expect(ct?.value).toBe("application/json");
  });

  it("removes content-type when switching to none", () => {
    const s0 = initState(REQ, null, "request");
    // REQ has content-type in headers
    const s1 = tabReducer(s0, { type: "SET_REQ_MODE", mode: "none" });
    expect(s1.reqMode).toBe("none");
    const ct = s1.reqHeaders.find((r) => r.key === "content-type");
    expect(ct).toBeUndefined();
  });

  it("replaces existing content-type when switching modes", () => {
    const s0 = initState(REQ, null, "request");
    const s1 = tabReducer(s0, { type: "SET_REQ_MODE", mode: "xml" });
    const ctHeaders = s1.reqHeaders.filter((r) => r.key === "content-type");
    expect(ctHeaders).toHaveLength(1);
    expect(ctHeaders[0].value).toBe("application/xml");
  });
});

// ── SET_RES_MODE ───────────────────────────────────────────────────────────

describe("SET_RES_MODE action", () => {
  it("inserts content-type into resHeaders", () => {
    const s0 = initState(MOCK, null, "mock");
    const s1 = tabReducer(s0, { type: "SET_RES_MODE", mode: "text" });
    expect(s1.resMode).toBe("text");
    const ct = s1.resHeaders.find((r) => r.key === "content-type");
    expect(ct?.value).toBe("text/plain");
  });
});

// ── APPLY_CURL ─────────────────────────────────────────────────────────────

describe("APPLY_CURL action", () => {
  it("applies url, method, headers and body from parsed cURL", () => {
    const s0 = initState(null, null, "request");
    const s1 = tabReducer(s0, {
      type: "APPLY_CURL",
      url: "https://api.test.com/data",
      method: "POST",
      headers: { "content-type": "application/json", "x-api-key": "secret" },
      body: '{"hello":"world"}',
    });
    expect(s1.url).toBe("https://api.test.com/data");
    expect(s1.method).toBe("POST");
    expect(s1.reqHeaders.some((r) => r.key === "x-api-key")).toBe(true);
    expect(s1.reqBody).toContain('"hello"');
    expect(s1.reqMode).toBe("json");
  });

  it("keeps existing fields when cURL provides no url", () => {
    const s0 = initState(REQ, null, "request");
    const s1 = tabReducer(s0, { type: "APPLY_CURL", url: "", method: "", headers: {}, body: "" });
    expect(s1.url).toBe(REQ.url);
    expect(s1.method).toBe(REQ.method);
  });
});

// ── Send lifecycle ─────────────────────────────────────────────────────────

describe("SEND_* actions", () => {
  it("SEND_START sets loading=true and clears errors and durationMs", () => {
    const s0: TabState = { ...initState(REQ, null, "request"), sendErr: "old error", durationMs: 120 };
    const s1 = tabReducer(s0, { type: "SEND_START" });
    expect(s1.loading).toBe(true);
    expect(s1.sendErr).toBeNull();
    expect(s1.result).toBeNull();
    expect(s1.durationMs).toBeNull();
  });

  it("SEND_SUCCESS sets result, durationMs and loading=false", () => {
    const s0 = tabReducer(initState(REQ, null, "request"), { type: "SEND_START" });
    const result = { status: 200, headers: { "content-type": "application/json" }, body: btoa("{}") };
    const s1 = tabReducer(s0, { type: "SEND_SUCCESS", result, resMode: "json", durationMs: 85 });
    expect(s1.loading).toBe(false);
    expect(s1.result?.status).toBe(200);
    expect(s1.durationMs).toBe(85);
    expect(s1.resTab).toBe("body");
    expect(s1.resMode).toBe("json");
  });

  it("SEND_ERROR sets sendErr and loading=false", () => {
    const s0 = tabReducer(initState(REQ, null, "request"), { type: "SEND_START" });
    const s1 = tabReducer(s0, { type: "SEND_ERROR", error: "Network error" });
    expect(s1.loading).toBe(false);
    expect(s1.sendErr).toBe("Network error");
    expect(s1.result).toBeNull();
  });
});

// ── Test lifecycle (mock) ──────────────────────────────────────────────────

describe("TEST_* actions", () => {
  it("TEST_START sets testLoading=true", () => {
    const s0 = initState(MOCK, null, "mock");
    const s1 = tabReducer(s0, { type: "TEST_START" });
    expect(s1.testLoading).toBe(true);
    expect(s1.testError).toBeNull();
  });

  it("TEST_SUCCESS updates editable response fields", () => {
    const s0 = tabReducer(initState(MOCK, null, "mock"), { type: "TEST_START" });
    const resHeaders = [{ id: "r1", enabled: true, key: "content-type", value: "text/plain" }];
    const s1 = tabReducer(s0, {
      type: "TEST_SUCCESS",
      resStatus: 404,
      resHeaders,
      resBody: "Not Found",
      resMode: "text",
    });
    expect(s1.testLoading).toBe(false);
    expect(s1.resStatus).toBe(404);
    expect(s1.resBody).toBe("Not Found");
    expect(s1.resMode).toBe("text");
    expect(s1.resTab).toBe("body");
  });

  it("TEST_ERROR sets testError", () => {
    const s0 = tabReducer(initState(MOCK, null, "mock"), { type: "TEST_START" });
    const s1 = tabReducer(s0, { type: "TEST_ERROR", error: "Connection refused" });
    expect(s1.testLoading).toBe(false);
    expect(s1.testError).toBe("Connection refused");
  });
});

// ── Save lifecycle ─────────────────────────────────────────────────────────

describe("SAVE_* actions", () => {
  it("SAVE_START sets saving=true", () => {
    const s0 = initState(REQ, null, "request");
    const s1 = tabReducer(s0, { type: "SAVE_START" });
    expect(s1.saving).toBe(true);
    expect(s1.saveErr).toBeNull();
  });

  it("SAVE_SUCCESS clears saving", () => {
    const s0 = tabReducer(initState(REQ, null, "request"), { type: "SAVE_START" });
    const s1 = tabReducer(s0, { type: "SAVE_SUCCESS" });
    expect(s1.saving).toBe(false);
  });

  it("SAVE_ERROR sets saveErr", () => {
    const s0 = tabReducer(initState(REQ, null, "request"), { type: "SAVE_START" });
    const s1 = tabReducer(s0, { type: "SAVE_ERROR", error: "Save failed" });
    expect(s1.saving).toBe(false);
    expect(s1.saveErr).toBe("Save failed");
  });
});

// ── stateToSavePayload ─────────────────────────────────────────────────────

describe("stateToSavePayload()", () => {
  it("extracts correct payload for request", () => {
    const s = initState(REQ, null, "request");
    const payload = stateToSavePayload(s, "request") as Omit<SavedRequest, "id" | "createdAt" | "workspaceId">;
    expect(payload.name).toBe("Get User");
    expect(payload.method).toBe("GET");
    expect(payload.url).toBe("https://api.example.com/user");
    expect(payload.headers["authorization"]).toBe("Bearer token");
    expect(payload.preScript).toBe("pm.environment.set('x', 1);");
    expect((payload as any).urlPattern).toBeUndefined();
  });

  it("extracts correct payload for mock", () => {
    const s = initState(MOCK, null, "mock");
    const payload = stateToSavePayload(s, "mock") as Omit<MockRule, "id" | "createdAt" | "workspaceId">;
    expect(payload.name).toBe("User Mock");
    expect(payload.urlPattern).toBe("/api/user");
    expect(payload.responseStatus).toBe(200);
    expect(payload.responseBody).toBe('{\n  "mocked": true\n}');
    expect(payload.mockedResponseHeaders).toEqual([]);
    expect(payload.responseBodyMocked).toBe(true);
    expect(payload.responseStatusMocked).toBe(true);
    // new mocks default to enabled:true; existing mocks preserve enabled via entity spread
    expect((payload as any).enabled).toBe(true);
    expect((payload as any).url).toBeUndefined();
  });

  it("persists mocked response header selections separately from header values", () => {
    const s = initState(MOCK, null, "mock");
    s.resHeaders = [{ id: "r1", enabled: true, key: "set-cookie", value: "session=1", mocked: true }];
    const payload = stateToSavePayload(s, "mock") as Omit<MockRule, "id" | "createdAt" | "workspaceId">;
    expect(payload.responseHeaders["set-cookie"]).toBe("session=1");
    expect(payload.mockedResponseHeaders).toEqual(["set-cookie"]);
  });

  it("trims name and url before saving", () => {
    const s = initState(null, null, "request");
    const dirty = { ...s, name: "  My Req  ", url: "  https://example.com  " };
    const payload = stateToSavePayload(dirty, "request") as Omit<SavedRequest, "id" | "createdAt" | "workspaceId">;
    expect(payload.name).toBe("My Req");
    expect(payload.url).toBe("https://example.com");
  });
});

// ── stateToDraft ───────────────────────────────────────────────────────────

describe("stateToDraft()", () => {
  it("serializes request state to RequestDraft", () => {
    const s = initState(REQ, null, "request");
    const d = stateToDraft(s, "request") as any;
    expect(d.name).toBe("Get User");
    expect(d.url).toBe("https://api.example.com/user");
    expect(d.preScript).toBeTruthy();
    expect(d.urlPattern).toBeUndefined();
  });

  it("serializes mock state to MockDraft", () => {
    const s = initState(MOCK, null, "mock");
    const d = stateToDraft(s, "mock") as any;
    expect(d.urlPattern).toBe("/api/user");
    expect(d.resBody).toBe('{\n  "mocked": true\n}');
    expect(d.url).toBeUndefined();
  });
});

// ── isDraftEmpty ───────────────────────────────────────────────────────────

describe("isDraftEmpty()", () => {
  it("returns true for blank request state", () => {
    const s = initState(null, null, "request");
    expect(isDraftEmpty(s, "request")).toBe(true);
  });

  it("returns false when url is set for request", () => {
    const s = { ...initState(null, null, "request"), url: "https://example.com" };
    expect(isDraftEmpty(s, "request")).toBe(false);
  });

  it("returns true for blank mock state", () => {
    const s = initState(null, null, "mock");
    expect(isDraftEmpty(s, "mock")).toBe(true);
  });

  it("returns false when resBody is set for mock", () => {
    const s = { ...initState(null, null, "mock"), resBody: '{"ok":true}' };
    expect(isDraftEmpty(s, "mock")).toBe(false);
  });
});
