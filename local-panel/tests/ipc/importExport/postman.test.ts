import { describe, it, expect } from "vitest";
import {
  exportRequestsToPostman,
  parsePostmanRequests,
  exportMocksToPostman,
  parsePostmanMocks,
} from "@/ipc/importExport/formats/postman";
import {
  b64Decode,
  b64Encode,
  hToRecord,
  bodyToText,
  urlRaw,
} from "@/ipc/importExport/formats/postmanHelpers";
import type { SavedRequest, MockRule, Folder } from "@/store/config";

// ── Helpers ──────────────────────────────────────────────────────────────────

function makeRequest(overrides: Partial<SavedRequest> = {}): SavedRequest {
  return {
    id: "req-1",
    workspaceId: "ws-1",
    name: "Get Users",
    method: "GET",
    url: "http://api.example.com/users",
    headers: { accept: "application/json" },
    body: "",
    createdAt: 1000,
    folderId: null,
    ...overrides,
  };
}

function makeMock(overrides: Partial<MockRule> = {}): MockRule {
  return {
    id: "mock-1",
    workspaceId: "ws-1",
    name: "Get Users Mock",
    method: "GET",
    urlPattern: "http://api.example.com/users",
    useRegex: false,
    enabled: true,
    capturedHeaders: {},
    capturedBody: "",
    responseStatus: 200,
    responseHeaders: { "content-type": "application/json" },
    responseBody: '{"users":[]}',
    createdAt: 1000,
    folderId: null,
    ...overrides,
  };
}

// ── b64Decode / b64Encode ─────────────────────────────────────────────────────

describe("b64Decode", () => {
  it("decodes base64 string", () => {
    expect(b64Decode(Buffer.from("hello").toString("base64"))).toBe("hello");
  });

  it("returns empty string for empty input", () => {
    expect(b64Decode("")).toBe("");
  });
});

describe("b64Encode", () => {
  it("encodes text to base64", () => {
    const encoded = b64Encode("hello");
    expect(b64Decode(encoded)).toBe("hello");
  });

  it("returns empty for whitespace-only input", () => {
    expect(b64Encode("   ")).toBe("");
  });
});

// ── hToRecord ─────────────────────────────────────────────────────────────────

describe("hToRecord", () => {
  it("converts Postman header array to record", () => {
    const result = hToRecord([
      { key: "content-type", value: "application/json" },
      { key: "x-api-key", value: "secret" },
    ]);
    expect(result).toEqual({ "content-type": "application/json", "x-api-key": "secret" });
  });

  it("skips disabled headers", () => {
    const result = hToRecord([
      { key: "x-disabled", value: "nope", disabled: true },
      { key: "accept", value: "*/*" },
    ]);
    expect(result).not.toHaveProperty("x-disabled");
    expect(result).toHaveProperty("accept", "*/*");
  });

  it("returns empty record for undefined input", () => {
    expect(hToRecord(undefined)).toEqual({});
  });
});

// ── bodyToText ────────────────────────────────────────────────────────────────

describe("bodyToText", () => {
  it("returns raw body for raw mode", () => {
    expect(bodyToText({ mode: "raw", raw: '{"foo":"bar"}' })).toBe('{"foo":"bar"}');
  });

  it("returns empty string for undefined", () => {
    expect(bodyToText(undefined)).toBe("");
  });

  it("encodes urlencoded body", () => {
    const result = bodyToText({ mode: "urlencoded", urlencoded: [{ key: "a", value: "1" }, { key: "b", value: "2" }] });
    expect(result).toBe("a=1&b=2");
  });

  it("skips disabled urlencoded params", () => {
    const result = bodyToText({ mode: "urlencoded", urlencoded: [{ key: "a", value: "1", disabled: true }, { key: "b", value: "2" }] });
    expect(result).toBe("b=2");
  });
});

// ── urlRaw ────────────────────────────────────────────────────────────────────

describe("urlRaw", () => {
  it("returns string as-is", () => {
    expect(urlRaw("http://example.com")).toBe("http://example.com");
  });

  it("returns raw from PMUrl object", () => {
    expect(urlRaw({ raw: "http://api.example.com" })).toBe("http://api.example.com");
  });
});

// ── exportRequestsToPostman ───────────────────────────────────────────────────

describe("exportRequestsToPostman", () => {
  it("produces valid Postman v2.1 collection JSON", () => {
    const json = exportRequestsToPostman([makeRequest()], []);
    const col = JSON.parse(json);
    expect(col.info.schema).toContain("v2.1.0");
    expect(col.item).toHaveLength(1);
    expect(col.item[0].name).toBe("Get Users");
    expect(col.item[0].request.method).toBe("GET");
    expect(col.item[0].request.url.raw).toBe("http://api.example.com/users");
  });

  it("includes headers in export", () => {
    const json = exportRequestsToPostman([makeRequest({ headers: { "x-api-key": "abc" } })], []);
    const col = JSON.parse(json);
    const headers: { key: string; value: string }[] = col.item[0].request.header;
    expect(headers.some((h) => h.key === "x-api-key" && h.value === "abc")).toBe(true);
  });

  it("uses folder structure", () => {
    const folder: Folder = { id: "f1", name: "Auth", parentId: null, workspaceId: "ws-1", createdAt: 0 };
    const req = makeRequest({ folderId: "f1" });
    const json = exportRequestsToPostman([req], [folder]);
    const col = JSON.parse(json);
    expect(col.item[0].name).toBe("Auth");
    expect(col.item[0].item[0].name).toBe("Get Users");
  });

  it("uses custom collection name", () => {
    const json = exportRequestsToPostman([makeRequest()], [], "My Collection");
    const col = JSON.parse(json);
    expect(col.info.name).toBe("My Collection");
  });
});

// ── parsePostmanRequests ──────────────────────────────────────────────────────

describe("parsePostmanRequests", () => {
  it("round-trips a simple request", () => {
    const json = exportRequestsToPostman([makeRequest()], []);
    const { requests } = parsePostmanRequests(json);
    expect(requests).toHaveLength(1);
    expect(requests[0].method).toBe("GET");
    expect(requests[0].url).toBe("http://api.example.com/users");
    expect(requests[0].name).toBe("Get Users");
  });

  it("round-trips headers", () => {
    const json = exportRequestsToPostman([makeRequest({ headers: { "x-api-key": "abc" } })], []);
    const { requests } = parsePostmanRequests(json);
    expect(requests[0].headers).toHaveProperty("x-api-key", "abc");
  });

  it("round-trips folder structure", () => {
    const folder: Folder = { id: "f1", name: "Auth", parentId: null, workspaceId: "ws-1", createdAt: 0 };
    const req = makeRequest({ folderId: "f1" });
    const json = exportRequestsToPostman([req], [folder]);
    const { folders, requests } = parsePostmanRequests(json);
    expect(folders).toHaveLength(1);
    expect(folders[0].name).toBe("Auth");
    expect(requests[0].folderId).toBe("Auth");
  });

  it("throws on non-Postman JSON", () => {
    expect(() => parsePostmanRequests('{"foo":"bar"}')).toThrow();
  });

  it("round-trips pre/post scripts", () => {
    const req = makeRequest({ preScript: "console.log('pre')", postScript: "pm.test('ok')" });
    const json = exportRequestsToPostman([req], []);
    const { requests } = parsePostmanRequests(json);
    expect(requests[0].preScript).toBe("console.log('pre')");
    expect(requests[0].postScript).toBe("pm.test('ok')");
  });
});

// ── exportMocksToPostman ──────────────────────────────────────────────────────

describe("exportMocksToPostman", () => {
  it("produces valid Postman v2.1 collection JSON", () => {
    const json = exportMocksToPostman([makeMock()], []);
    const col = JSON.parse(json);
    expect(col.info.schema).toContain("v2.1.0");
    expect(col.item).toHaveLength(1);
  });

  it("stores _localpanel extension with urlPattern and enabled", () => {
    const json = exportMocksToPostman([makeMock({ urlPattern: "^/api", useRegex: true, enabled: false })], []);
    const col = JSON.parse(json);
    expect(col.item[0]._localpanel.urlPattern).toBe("^/api");
    expect(col.item[0]._localpanel.useRegex).toBe(true);
    expect(col.item[0]._localpanel.enabled).toBe(false);
  });

  it("stores response body and status", () => {
    const json = exportMocksToPostman([makeMock({ responseStatus: 404, responseBody: '{"error":"not found"}' })], []);
    const col = JSON.parse(json);
    expect(col.item[0].response[0].code).toBe(404);
    expect(col.item[0].response[0].body).toBe('{"error":"not found"}');
  });
});

// ── parsePostmanMocks ─────────────────────────────────────────────────────────

describe("parsePostmanMocks", () => {
  it("round-trips a simple mock", () => {
    const json = exportMocksToPostman([makeMock()], []);
    const { mocks } = parsePostmanMocks(json);
    expect(mocks).toHaveLength(1);
    expect(mocks[0].urlPattern).toBe("http://api.example.com/users");
    expect(mocks[0].responseStatus).toBe(200);
    expect(mocks[0].responseBody).toBe('{"users":[]}');
  });

  it("preserves _localpanel fields on round-trip", () => {
    const json = exportMocksToPostman([makeMock({ urlPattern: "^/api", useRegex: true, enabled: false })], []);
    const { mocks } = parsePostmanMocks(json);
    expect(mocks[0].urlPattern).toBe("^/api");
    expect(mocks[0].useRegex).toBe(true);
    expect(mocks[0].enabled).toBe(false);
  });

  it("round-trips response headers", () => {
    const json = exportMocksToPostman([makeMock({ responseHeaders: { "content-type": "application/json" } })], []);
    const { mocks } = parsePostmanMocks(json);
    expect(mocks[0].responseHeaders).toHaveProperty("content-type", "application/json");
  });

  it("throws on non-Postman JSON", () => {
    expect(() => parsePostmanMocks('{"schema":"something-else"}')).toThrow();
  });
});
