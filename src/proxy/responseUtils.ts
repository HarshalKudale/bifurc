import * as http from "http";
import * as net from "net";
import { HOP_BY_HOP } from "@/proxy/constants";
import { emitLogChunk } from "@/proxy/logEmitter";
import { decompressBody, stripContentEncoding } from "@/proxy/decompressUtils";

export function buildResponseHeaders(res: http.IncomingMessage) {
  let head = `HTTP/1.1 ${res.statusCode} ${res.statusMessage ?? ""}\r\n`;
  const resHeaders: Record<string, string> = {};
  for (const [k, v] of Object.entries(res.headers)) {
    if (HOP_BY_HOP.has(k.toLowerCase())) continue;
    if (k.toLowerCase() === "set-cookie") {
      const cookies = Array.isArray(v) ? v : [v];
      for (const c of cookies) { if (c != null) head += `${k}: ${c}\r\n`; }
      resHeaders[k] = cookies.filter((x) => x != null).join("\n");
    } else {
      const vals = Array.isArray(v) ? v : [v];
      const joined = vals.filter((x) => x != null).join(", ");
      resHeaders[k] = joined;
      head += `${k}: ${joined}\r\n`;
    }
  }
  return { head, resHeaders };
}

export function parseRawHeaders(headers: http.IncomingHttpHeaders) {
  const resHeaders: Record<string, string> = {};
  for (const [k, v] of Object.entries(headers)) {
    if (HOP_BY_HOP.has(k.toLowerCase())) continue;
    if (k.toLowerCase() === "set-cookie") {
      const cookies = Array.isArray(v) ? v : [v];
      resHeaders[k] = cookies.filter(Boolean).join("\n");
    } else {
      const vals = Array.isArray(v) ? v : [v];
      resHeaders[k] = vals.filter(Boolean).join(", ");
    }
  }
  return resHeaders;
}

export function handleProxyResponse(
  res: http.IncomingMessage,
  socket: net.Socket,
  t0: number,
  logId?: string,
  onDone?: (status: number, durationMs: number, resHeaders: Record<string, string>, resBody: string) => void
) {
  let { head, resHeaders } = buildResponseHeaders(res);
  head += "connection: close\r\n\r\n";

  const chunks: Buffer[] = [];
  const isStreaming = logId && (
    (resHeaders["content-type"] ?? "").includes("text/event-stream") ||
    (resHeaders["transfer-encoding"] ?? "").includes("chunked")
  );
  res.on("data", (c: Buffer) => {
    chunks.push(c);
    if (isStreaming && logId) emitLogChunk(logId, c, false);
    if (!socket.writable) res.destroy();
  });

  if (!socket.writable) { res.destroy(); return; }
  socket.write(head);
  res.pipe(socket);
  res.on("error", () => socket.destroy());
  socket.on("error", () => res.destroy());
  res.on("end", () => {
    const full = Buffer.concat(chunks);
    if (isStreaming && logId) emitLogChunk(logId, Buffer.alloc(0), true);
    const ce = resHeaders["content-encoding"] ?? "";
    const decompressed = decompressBody(full, ce);
    const logHeaders = ce ? stripContentEncoding(resHeaders) : resHeaders;
    const slice = decompressed.length <= 512 * 1024 ? decompressed : decompressed.slice(0, 512 * 1024);
    onDone?.(res.statusCode ?? 0, Date.now() - t0, logHeaders, slice.toString("base64"));
  });
}

import * as https from "https";
import { ProxyRule } from "@/store/config";
import { sendHtml } from "@/proxy/pages";
import { executeRequestScript, executeResponseScript } from "@/proxy/scriptExecutor";

export function passthroughToUpstream(
  socket: net.Socket,
  method: string,
  rawTarget: string,
  path: string,
  reqHeaders: Record<string, string>,
  body: Buffer,
  onDone?: (status: number, durationMs: number, resHeaders: Record<string, string>, resBody: string) => void,
  logId?: string,
): void {
  const t0 = Date.now();
  let hostname: string;
  let port: number;
  try {
    const fullUrl = rawTarget.startsWith("http") ? rawTarget : `http://${reqHeaders["host"] ?? ""}${path}`;
    const u = new URL(fullUrl);
    hostname = u.hostname;
    port = u.port ? parseInt(u.port, 10) : 80;
  } catch {
    sendHtml(socket, 502, "<h1>502 Bad Gateway</h1><p>Could not parse upstream URL.</p>");
    onDone?.(502, Date.now() - t0, {}, "");
    return;
  }

  const upstreamHeaders: Record<string, string> = { ...reqHeaders, connection: "close" };
  if (body.length > 0) upstreamHeaders["content-length"] = String(body.length);

  const req = http.request(
    { hostname, port, path: path || "/", method, headers: upstreamHeaders },
    (res) => {
      handleProxyResponse(res, socket, t0, logId, onDone);
    }
  );

  req.on("error", (err) => {
    console.error(`[passthrough] ${hostname}:${port} —`, err.message);
    if (socket.writable) {
      sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${hostname}:${port}</code>.</p>`);
    }
    onDone?.(502, Date.now() - t0, {}, "");
  });

  if (body.length > 0) req.write(body);
  req.end();
}

export function passthroughToUpstreamHttps(
  socket: net.Socket,
  method: string,
  hostname: string,
  port: number,
  path: string,
  reqHeaders: Record<string, string>,
  body: Buffer,
  onDone?: (status: number, durationMs: number, resHeaders: Record<string, string>, resBody: string) => void,
  logId?: string,
): void {
  const t0 = Date.now();
  const upstreamHeaders: Record<string, string> = { ...reqHeaders, connection: "close" };
  if (body.length > 0) upstreamHeaders["content-length"] = String(body.length);

  const req = https.request(
    { hostname, port, path: path || "/", method, headers: upstreamHeaders, rejectUnauthorized: false },
    (res) => {
      handleProxyResponse(res, socket, t0, logId, onDone);
    }
  );

  req.on("error", (_err) => {
    if (socket.writable) {
      sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${hostname}:${port}</code>.</p>`);
    }
    onDone?.(502, Date.now() - t0, {}, "");
  });

  if (body.length > 0) req.write(body);
  req.end();
}

export interface BufferedProxyResponse {
  status: number;
  headers: Record<string, string>;
  body: Buffer;
  durationMs: number;
}

export function fetchUpstreamResponse(
  method: string,
  target: string,
  path: string,
  reqHeaders: Record<string, string>,
  reqBody: Buffer,
  rule?: ProxyRule,
): Promise<BufferedProxyResponse> {
  return new Promise((resolve, reject) => {
    let finalHeaders = { ...reqHeaders };
    let finalBody = reqBody;

    if (rule?.requestScript?.trim()) {
      const result = executeRequestScript(rule.requestScript, finalHeaders, reqBody.toString("utf8"));
      if (!result.error) {
        finalHeaders = result.headers;
        finalBody = Buffer.from(result.body, "utf8");
      }
    }

    const t0 = Date.now();
    const targetUrl = target.startsWith("http://") || target.startsWith("https://")
      ? new URL(target)
      : new URL(`http://${target}`);
    const isHttps = targetUrl.protocol === "https:";
    const transport = isHttps ? https : http;
    const requestPath = target.startsWith("http://") || target.startsWith("https://")
      ? `${targetUrl.pathname || "/"}${targetUrl.search || ""}`
      : (path || "/");

    const upstreamHeaders: Record<string, string> = { ...finalHeaders, connection: "close" };
    if (finalBody.length > 0) upstreamHeaders["content-length"] = String(finalBody.length);

    const req = transport.request(
      {
        hostname: targetUrl.hostname,
        port: targetUrl.port ? parseInt(targetUrl.port, 10) : (isHttps ? 443 : 80),
        path: requestPath,
        method,
        headers: upstreamHeaders,
        ...(isHttps ? { rejectUnauthorized: false } : {}),
      },
      (res) => {
        const chunks: Buffer[] = [];
        res.on("data", (c: Buffer) => chunks.push(c));
        res.on("end", () => {
          const rawResHeaders = parseRawHeaders(res.headers);

          const resBodyBuf = Buffer.concat(chunks);
          const ce = rawResHeaders["content-encoding"] ?? "";
          const decompressedBuf = decompressBody(resBodyBuf, ce);
          const decompressedHeaders = ce ? stripContentEncoding(rawResHeaders) : rawResHeaders;

          if (!rule?.responseScript?.trim()) {
            resolve({
              status: res.statusCode ?? 0,
              headers: decompressedHeaders,
              body: decompressedBuf,
              durationMs: Date.now() - t0,
            });
            return;
          }

          const scriptResult = executeResponseScript(rule.responseScript, decompressedHeaders, decompressedBuf.toString("utf8"));
          resolve({
            status: res.statusCode ?? 0,
            headers: scriptResult.error ? decompressedHeaders : scriptResult.headers,
            body: scriptResult.error ? decompressedBuf : Buffer.from(scriptResult.body, "utf8"),
            durationMs: Date.now() - t0,
          });
        });
      },
    );

    req.on("error", reject);
    if (finalBody.length > 0) req.write(finalBody);
    req.end();
  });
}
