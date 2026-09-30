/**
 * Capture → entity construction, moved with the code — `plan/19` WI 5, item 2.
 *
 * These cases were `tests/renderer/captureUtils.test.ts`'s *"turning a capture into a saved request"* and
 * *"turning a capture into a mock"* blocks, written against `reqToHeadersBody` / `buildMockInitial`. Both
 * functions are `capturedToRequest` / `capturedToMock` in
 * `packages/engine/src/utils/captureEntities.ts` now, so the cases moved to the engine's side of the
 * boundary rather than dying with the renderer's copy. The `describe` blocks left in the renderer's suite
 * are the ones that never moved — formatting, resource-type derivation, the fulfilled-by labels.
 *
 * **The base64 cases are new, and they are why this file is not a pure copy-paste.** The renderer decoded
 * with `atob`, which **throws** on malformed input, and its `catch` returned the *raw input* so an odd
 * capture shows as-is rather than vanishing. Node's `Buffer.from(x, "base64")` does **not** throw — it
 * silently skips offending characters — so the port needed an explicit validity check. Without it the
 * behaviour changes on the way over, and nothing in the old suite would have noticed.
 */
import { describe, it, expect } from "vitest";
import type { RequestLogEntry } from "@bifurc/engine/kernel/telemetry";
import { capturedToRequest, capturedToMock } from "@bifurc/engine/utils/captureEntities";

// ── Fixtures ─────────────────────────────────────────────────────────────────

const b64 = (s: string): string => Buffer.from(s, "utf-8").toString("base64");
const b64bytes = (n: number): string => Buffer.alloc(n, 0x61).toString("base64");

function entry(overrides: Partial<RequestLogEntry> = {}): RequestLogEntry {
  return {
    id: "log-1",
    ts: 1_700_000_000_000,
    method: "GET",
    url: "http://api.localhost/users",
    host: "api.localhost",
    status: 200,
    via: "rfc6761",
    target: "127.0.0.1:3000",
    durationMs: 12,
    reqHeaders: { host: "api.localhost", accept: "application/json" },
    reqBody: "",
    resHeaders: { "content-type": "application/json" },
    resBody: b64('{"ok":true}'),
    resStatus: 200,
    ...overrides,
  };
}

// ── capture → saved request ──────────────────────────────────────────────────

describe("capturedToRequest — turning a capture into a saved request", () => {
  it("decodes the body and keeps the caller's headers", () => {
    const req = capturedToRequest(entry({ reqBody: b64('{"name":"ada"}') }));

    expect(req.method).toBe("GET");
    expect(req.url).toBe("http://api.localhost/users");
    expect(req.body).toBe('{"name":"ada"}');
    expect(req.headers["accept"]).toBe("application/json");
  });

  it("strips headers that must be recomputed on replay", () => {
    const req = capturedToRequest(entry({
      reqHeaders: {
        host: "api.localhost",
        connection: "keep-alive",
        "content-length": "13",
        "transfer-encoding": "chunked",
        "proxy-connection": "keep-alive",
        "x-keep": "yes",
      },
    }));

    expect(req.headers).toEqual({ "x-keep": "yes" });
  });

  it("leaves the name blank so the caller can label it", () => {
    expect(capturedToRequest(entry()).name).toBe("");
  });
});

// ── capture → mock ───────────────────────────────────────────────────────────

describe("capturedToMock — turning a capture into a mock", () => {
  it("carries the request and response across verbatim", () => {
    const mock = capturedToMock(entry({
      reqHeaders: { host: "api.localhost", accept: "application/json" },
      reqBody: b64('{"q":1}'),
      resHeaders: { "content-type": "application/json" },
      resBody: b64('{"ok":true}'),
    }));

    expect(mock.method).toBe("GET");
    expect(mock.urlPattern).toBe("http://api.localhost/users");
    expect(mock.useRegex).toBe(false);
    // The mock stores the *raw* capture, so it can replay byte-for-byte — including `host`, which the
    // saved-request path strips. That asymmetry is the point, not an oversight.
    expect(mock.capturedHeaders).toEqual({ host: "api.localhost", accept: "application/json" });
    expect(mock.capturedBody).toBe(b64('{"q":1}'));
    expect(mock.responseStatus).toBe(200);
    expect(mock.responseBody).toBe('{"ok":true}');
    expect(mock.responseBodyEncoding).toBeUndefined();
  });

  it("marks the response status, body and delay as mocked so the mock stands alone", () => {
    const mock = capturedToMock(entry());

    expect(mock.responseStatusMocked).toBe(true);
    expect(mock.responseBodyMocked).toBe(true);
    expect(mock.responseDelayMocked).toBe(true);
    expect(mock.mockedResponseHeaders).toEqual([]);
  });

  it("keeps a binary response as base64 and flags the encoding", () => {
    const mock = capturedToMock(entry({
      resHeaders: { "content-type": "image/png" },
      resBody: b64bytes(32),
    }));

    expect(mock.responseBodyEncoding).toBe("base64");
    expect(mock.responseBody).toBe(b64bytes(32));
  });

  it("falls back to '{}' when a text response has no body", () => {
    const mock = capturedToMock(entry({ resBody: "" }));

    expect(mock.responseBody).toBe("{}");
    expect(mock.responseBodyEncoding).toBeUndefined();
  });

  it("defaults the status to 200 when the capture never got a response", () => {
    const mock = capturedToMock(entry({ resStatus: null, status: null, via: "error" }));

    expect(mock.responseStatus).toBe(200);
  });

  it("preserves a non-2xx captured status", () => {
    const mock = capturedToMock(entry({ resStatus: 503, status: 503 }));

    expect(mock.responseStatus).toBe(503);
  });

  it("finds the content type case-insensitively", () => {
    // The renderer's `getHeader` did this and the engine's `headerOf` must too — a `Content-Type` spelled
    // with capitals would otherwise leave a PNG decoded as text.
    const mock = capturedToMock(entry({
      resHeaders: { "Content-Type": "image/png" },
      resBody: b64bytes(8),
    }));

    expect(mock.responseBodyEncoding).toBe("base64");
  });
});

// ── the base64 fallback (new, and the reason this file exists) ───────────────

describe("the base64 decode preserves the renderer's fallback", () => {
  it("returns the raw input when the body is not valid base64", () => {
    // `atob("!!!not-base64!!!")` threw and the renderer's `catch` returned the input. Node's decoder would
    // silently skip the punctuation and return garbage, so this case is what pins the explicit check.
    expect(capturedToRequest(entry({ reqBody: "!!!not-base64!!!" })).body).toBe("!!!not-base64!!!");
  });

  it("still decodes valid base64 that carries line breaks", () => {
    // `atob` tolerates ASCII whitespace (the HTML spec strips it), so the port must too.
    const wrapped = b64("hello").replace(/(.{2})/g, "$1\n").trim();
    expect(capturedToRequest(entry({ reqBody: wrapped })).body).toBe("hello");
  });

  it("treats an empty body as empty text", () => {
    expect(capturedToRequest(entry({ reqBody: "" })).body).toBe("");
  });
});
