import * as net from "net";
import { MockRule, Environment } from "@/store/config";
import { HOP_BY_HOP } from "@/proxy/constants";
import { resolveRandomizers } from "@/lib/randomizer";

export interface ResolvedMockResponse {
  status: number;
  headers: Record<string, string>;
  body: Buffer;
  delayMs: number;
}

export function resolveVars(text: string, env: Environment | null): string {
  if (!text) return text;
  let result = text;
  if (env) {
    result = result.replace(/\{\{(\w+)\}\}/g, (_match, key) => {
      if (key.startsWith("random.")) return _match;
      const v = env.variables.find((v) => v.key === key);
      return v !== undefined ? v.value : _match;
    });
  }
  return resolveRandomizers(result);
}

export function matchMock(mocks: MockRule[], method: string, url: string, env: Environment | null): MockRule | null {
  for (const m of mocks) {
    if (!m.enabled) continue;
    if (m.method !== "*" && m.method.toUpperCase() !== method.toUpperCase()) continue;
    try {
      const pattern = resolveVars(m.urlPattern, env);
      const hit = m.useRegex
        ? new RegExp(pattern).test(url)
        : url === pattern;
      if (hit) return m;
    } catch { /* invalid regex */ }
  }
  return null;
}

const SKIP_MOCK_RES_HEADERS = new Set([
  ...Array.from(HOP_BY_HOP),
  "content-length",
  "content-encoding",
]);

function resolveMockBody(mock: MockRule, env: Environment | null): Buffer {
  return mock.responseBodyEncoding === "base64"
    ? Buffer.from(mock.responseBody, "base64")
    : Buffer.from(resolveVars(mock.responseBody, env), "utf-8");
}

function resolveMockHeaders(mock: MockRule, env: Environment | null): Record<string, string> {
  const headers: Record<string, string> = {};
  for (const [k, v] of Object.entries(mock.responseHeaders)) {
    if (!SKIP_MOCK_RES_HEADERS.has(k.toLowerCase())) {
      headers[k] = mock.responseBodyEncoding === "base64" ? v : resolveVars(v, env);
    }
  }
  return headers;
}

function ensureResponseHeaders(headers: Record<string, string>, body: Buffer, defaultContentType = "application/json"): Record<string, string> {
  const next = { ...headers };
  if (!Object.keys(next).some((key) => key.toLowerCase() === "content-type")) {
    next["content-type"] = defaultContentType;
  }
  next["content-length"] = String(body.length);
  next["connection"] = "close";
  return next;
}

function writeResponse(socket: net.Socket, status: number, headers: Record<string, string>, body: Buffer): void {
  let head = `HTTP/1.1 ${status} ${statusText(status)}\r\n`;
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === "set-cookie") {
      const cookieLines = v.split("\n").filter(Boolean);
      for (const cookie of cookieLines) head += `${k}: ${cookie}\r\n`;
      continue;
    }
    head += `${k}: ${v}\r\n`;
  }
  head += "\r\n";
  socket.write(head);
  socket.write(body);
  socket.end();
}

function mockedHeaderKeys(mock: MockRule): Set<string> {
  return new Set((mock.mockedResponseHeaders ?? []).map((key) => key.toLowerCase()));
}

export function isFullyMocked(mock: MockRule): boolean {
  if (mock.streamingMode && mock.streamingMode !== "none") return true;
  const keys = Object.keys(resolveMockHeaders(mock, null)).filter((key) => key.trim());
  const mockedKeys = mockedHeaderKeys(mock);
  const allHeadersMocked = keys.every((key) => mockedKeys.has(key.toLowerCase()));
  return (mock.responseStatusMocked ?? true)
    && (mock.responseBodyMocked ?? true)
    && (mock.responseDelayMocked ?? true)
    && allHeadersMocked;
}

export function resolveMockOnlyResponse(mock: MockRule, env: Environment | null): ResolvedMockResponse {
  const body = resolveMockBody(mock, env);
  const headers = ensureResponseHeaders(
    resolveMockHeaders(mock, env),
    body,
    mock.responseBodyEncoding === "base64" ? "application/octet-stream" : "application/json",
  );
  return {
    status: mock.responseStatus,
    headers,
    body,
    delayMs: mock.responseDelay && mock.responseDelay > 0 ? mock.responseDelay : 0,
  };
}

export function mergeMockWithUpstream(
  mock: MockRule,
  upstream: { status: number; headers: Record<string, string>; body: Buffer; durationMs: number },
  env: Environment | null,
): ResolvedMockResponse {
  const resolvedMockHeaders = resolveMockHeaders(mock, env);
  const body = (mock.responseBodyMocked ?? true) ? resolveMockBody(mock, env) : upstream.body;
  const headers = { ...upstream.headers };
  const mockedKeys = mockedHeaderKeys(mock);
  for (const [key, value] of Object.entries(resolvedMockHeaders)) {
    if (mockedKeys.has(key.toLowerCase())) headers[key] = value;
  }

  return {
    status: (mock.responseStatusMocked ?? true) ? mock.responseStatus : upstream.status,
    headers: ensureResponseHeaders(
      headers,
      body,
      mock.responseBodyEncoding === "base64" ? "application/octet-stream" : "application/json",
    ),
    body,
    delayMs: (mock.responseDelayMocked ?? true) ? (mock.responseDelay && mock.responseDelay > 0 ? mock.responseDelay : 0) : 0,
  };
}

export function serveMock(socket: net.Socket, mock: MockRule, env: Environment | null): void {
  const delayMs = mock.responseDelay && mock.responseDelay > 0 ? mock.responseDelay : 0;

  // Route to streaming handler if applicable
  if (mock.streamingMode && mock.streamingMode !== "none") {
    const doStream = () => serveStreamingMock(socket, mock, env);
    if (delayMs > 0) setTimeout(doStream, delayMs);
    else doStream();
    return;
  }

  const send = () => {
    if (!socket.writable) return;
    const resolved = resolveMockOnlyResponse(mock, env);
    writeResponse(socket, resolved.status, resolved.headers, resolved.body);
  };

  if (delayMs > 0) {
    setTimeout(send, delayMs);
  } else {
    send();
  }
}

export function serveResolvedResponse(socket: net.Socket, response: ResolvedMockResponse): void {
  const send = () => {
    if (!socket.writable) return;
    writeResponse(socket, response.status, response.headers, response.body);
  };
  if (response.delayMs > 0) {
    setTimeout(send, response.delayMs);
  } else {
    send();
  }
}

export function serveStreamingMock(socket: net.Socket, mock: MockRule, env: Environment | null): void {
  if (!socket.writable) return;

  const chunkDelay = mock.streamingChunkDelay ?? 100;
  const resolvedBody = mock.responseBodyEncoding === "base64"
    ? mock.responseBody
    : resolveVars(mock.responseBody, env);

  const headers: Record<string, string> = {};
  for (const [k, v] of Object.entries(mock.responseHeaders)) {
    if (!SKIP_MOCK_RES_HEADERS.has(k.toLowerCase())) headers[k] = resolveVars(v, env);
  }

  if (mock.streamingMode === "sse") {
    // SSE mode: split by double-newline, send as events
    headers["content-type"] = headers["content-type"] || "text/event-stream";
    headers["cache-control"] = "no-cache";
    headers["connection"] = "keep-alive";
    headers["transfer-encoding"] = "chunked";

    let head = `HTTP/1.1 ${mock.responseStatus} ${statusText(mock.responseStatus)}\r\n`;
    for (const [k, v] of Object.entries(headers)) head += `${k}: ${v}\r\n`;
    head += "\r\n";
    socket.write(head);

    const events = resolvedBody.split("\n\n").filter(Boolean);
    let idx = 0;

    const sendNext = () => {
      if (!socket.writable || idx >= events.length) {
        // Send terminal chunk and end
        if (socket.writable) {
          socket.write("0\r\n\r\n");
          socket.end();
        }
        return;
      }
      const event = events[idx] + "\n\n";
      const chunk = Buffer.from(event, "utf-8");
      socket.write(`${chunk.length.toString(16)}\r\n`);
      socket.write(chunk);
      socket.write("\r\n");
      idx++;
      setTimeout(sendNext, chunkDelay);
    };
    sendNext();
  } else {
    // Chunked mode: split by separator
    const separator = mock.streamingChunkSeparator ?? "\n\n";
    headers["transfer-encoding"] = "chunked";
    headers["connection"] = "keep-alive";
    if (!headers["content-type"]) headers["content-type"] = "application/json";

    let head = `HTTP/1.1 ${mock.responseStatus} ${statusText(mock.responseStatus)}\r\n`;
    for (const [k, v] of Object.entries(headers)) head += `${k}: ${v}\r\n`;
    head += "\r\n";
    socket.write(head);

    const chunks = resolvedBody.split(separator).filter(Boolean);
    let idx = 0;

    const sendNext = () => {
      if (!socket.writable || idx >= chunks.length) {
        if (socket.writable) {
          socket.write("0\r\n\r\n");
          socket.end();
        }
        return;
      }
      const data = chunks[idx] + (idx < chunks.length - 1 ? separator : "");
      const chunk = Buffer.from(data, "utf-8");
      socket.write(`${chunk.length.toString(16)}\r\n`);
      socket.write(chunk);
      socket.write("\r\n");
      idx++;
      setTimeout(sendNext, chunkDelay);
    };
    sendNext();
  }
}

export function statusText(code: number): string {
  const map: Record<number, string> = {
    200: "OK", 201: "Created", 204: "No Content",
    301: "Moved Permanently", 302: "Found", 304: "Not Modified",
    400: "Bad Request", 401: "Unauthorized", 403: "Forbidden",
    404: "Not Found", 405: "Method Not Allowed", 409: "Conflict",
    422: "Unprocessable Entity", 429: "Too Many Requests",
    500: "Internal Server Error", 502: "Bad Gateway", 503: "Service Unavailable",
  };
  return map[code] ?? "Unknown";
}
