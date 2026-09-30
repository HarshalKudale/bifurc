/**
 * Capture → entity construction — `plan/19` WI 5, item 2.
 *
 * ## What this is, and what it replaces
 *
 * Two pure functions that turn a captured `RequestLogEntry` into something the user can keep:
 * `capturedToRequest` (a saved request, for the editor) and `capturedToMock` (a mock, for replay). They
 * were `reqToHeadersBody` and `buildMockInitial` in `packages/gui/src/components/capture/captureUtils.ts`,
 * and the renderer kept only the **formatting** half of that file.
 *
 * Both are engine work by `plan/19` D22's test — *would this differ if the UI were a terminal?* They decide
 * **which headers to drop**, **how to decode a body** and **how a response is stored**. None of that is
 * about the screen.
 *
 * ## Why `utils/`
 *
 * They read nothing: no file, no config, no sibling state. `RequestLogEntry` arrives as a **type** import
 * from `kernel/telemetry.ts`, which the layering ratchet exempts because `import type` is erased. That is
 * §5.1's "stateless + shared → extract **down**" case, and it is why the two commands that expose this
 * (`capture.buildRequest`, `capture.buildMock`) live in the feature modules that own the entities they
 * build — `rest/` and `mocks/` — rather than in a module of their own.
 *
 * ## ⚠ The hop-by-hop list is a third copy of the same five names, and this one is gone
 *
 * `captureUtils.ts` carried its own `SKIP_HEADERS`, character-for-character identical to
 * `SKIP_CURL_HEADERS` in `utils/curl.ts`. It is deleted, not moved: the list is imported. **The name is
 * now a lie** — it is not curl-specific, and this file is the proof — and consolidating it into its own
 * module (`utils/hopByHop.ts`) is the follow-up. It is recorded rather than done here because the rename
 * touches the renderer's pinned copy and its pin test, which is a separate change from this move.
 *
 * ## ⚠ The base64 fallback is a behaviour port, not a copy-paste
 *
 * The renderer's decoder used `atob`, which **throws** on malformed input, and the `catch` returned the
 * *raw input* so a non-base64 body shows as-is rather than vanishing. Node's `Buffer.from(x, "base64")`
 * does **not** throw — it silently skips offending characters — so a straight port would quietly change
 * what an odd capture becomes. The validity check below is explicit for that reason, and
 * `tests/engine/captureEntities.test.ts` keeps the case that pins it.
 */
import type { RequestLogEntry } from "../kernel/telemetry";
import { SKIP_CURL_HEADERS } from "./curl";
import { isBinaryContentType } from "./contentType";

/** A captured request as the **editor** wants it: hop-by-hop headers gone, body decoded to text. */
export interface CapturedRequest {
  /** Always blank — the caller labels it (`"GET http://…"`, or a comment line from the paste). */
  name: string;
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;
}

/** A captured exchange as a **mock**: the raw capture, plus the fields that make the mock stand alone. */
export interface CapturedMock {
  name: string;
  method: string;
  urlPattern: string;
  useRegex: boolean;
  /** The **raw** capture, hop-by-hop headers included, so the mock can replay byte-for-byte. */
  capturedHeaders: Record<string, string>;
  /** Base64, as captured. */
  capturedBody: string;
  responseStatus: number;
  responseStatusMocked: boolean;
  responseHeaders: Record<string, string>;
  mockedResponseHeaders: string[];
  responseBody: string;
  responseBodyMocked: boolean;
  responseBodyEncoding?: "utf8" | "base64";
  responseDelayMocked: boolean;
}

/**
 * Base64 → text, **falling back to the raw input when it is not valid base64**. See the header: the
 * fallback is the renderer's, and it survives the move.
 */
function b64ToText(b64: string): string {
  if (!b64) return "";
  const stripped = b64.replace(/\s+/g, "");
  if (stripped.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(stripped)) return b64;
  try {
    return Buffer.from(stripped, "base64").toString("utf-8");
  } catch {
    return b64;
  }
}

/** Case-insensitive header lookup. `""` when absent, matching the renderer's `getHeader`. */
function headerOf(headers: Record<string, string>, name: string): string {
  const lower = name.toLowerCase();
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === lower) return v;
  }
  return "";
}

/** A captured request → a saved request. Used by "Save" and by "Open in requests". */
export function capturedToRequest(entry: RequestLogEntry): CapturedRequest {
  const headers: Record<string, string> = {};
  for (const [k, v] of Object.entries(entry.reqHeaders)) {
    if (!SKIP_CURL_HEADERS.has(k.toLowerCase())) headers[k] = v;
  }
  return { name: "", method: entry.method, url: entry.url, headers, body: b64ToText(entry.reqBody) };
}

/**
 * A captured exchange → a mock.
 *
 * **The captured headers are stored raw** — this is the one place `SKIP_CURL_HEADERS` is deliberately *not*
 * applied, because a mock replays the capture byte-for-byte and a `host` that was sent is part of what was
 * captured. The mock *editor* filters them for display (`restTabHelpers.ts#buildMockReqHeaders`, which is
 * where the renderer's pinned copy of the list is used).
 */
export function capturedToMock(entry: RequestLogEntry): CapturedMock {
  const resCt = headerOf(entry.resHeaders, "content-type").toLowerCase();
  const isBinaryRes = isBinaryContentType(resCt);
  return {
    name: "",
    method: entry.method,
    urlPattern: entry.url,
    useRegex: false,
    capturedHeaders: entry.reqHeaders,
    capturedBody: entry.reqBody,
    responseStatus: entry.resStatus ?? 200,
    responseStatusMocked: true,
    responseHeaders: entry.resHeaders,
    mockedResponseHeaders: [],
    responseBody: isBinaryRes ? entry.resBody : (b64ToText(entry.resBody) || "{}"),
    responseBodyMocked: true,
    responseBodyEncoding: isBinaryRes ? "base64" : undefined,
    responseDelayMocked: true,
  };
}
