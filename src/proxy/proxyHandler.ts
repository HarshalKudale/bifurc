import * as net from "net";
import * as http from "http";
import * as https from "https";
import { ProxyRule, LocalMapping } from "@/store/config";
import { HOP_BY_HOP } from "@/proxy/constants";
import { sendHtml } from "@/proxy/pages";
import { executeRequestScript, executeResponseScript } from "@/proxy/scriptExecutor";
import { decompressBody, stripContentEncoding } from "@/proxy/decompressUtils";
import { emitLogChunk } from "@/proxy/logEmitter";
import { handleProxyResponse, parseRawHeaders } from "@/proxy/responseUtils";

export function tcpTunnel(
  socket: net.Socket,
  host: string,
  port: number,
  initialData: Buffer | null,
  preamble: string | null,
): void {
  const upstream = net.connect(port, host, () => {
    if (preamble) socket.write(preamble);
    if (initialData && initialData.length > 0) upstream.write(initialData);
    upstream.pipe(socket);
    socket.pipe(upstream);
  });

  upstream.on("error", (_err) => {
    try {
      if (preamble) {
        socket.write("HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
      } else {
        sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${host}:${port}</code>.</p>`);
      }
    } catch { /* socket already gone */ }
    socket.destroy();
  });

  socket.on("error", () => upstream.destroy());
  upstream.on("close", () => socket.destroy());
  socket.on("close", () => upstream.destroy());
}

export function proxyToUpstream(
  socket: net.Socket,
  method: string,
  target: string,
  path: string,
  reqHeaders: Record<string, string>,
  body: Buffer,
  onDone?: (status: number, durationMs: number, resHeaders: Record<string, string>, resBody: string) => void,
  logId?: string,
): void {
  const t0 = Date.now();
  const colon = target.lastIndexOf(":");
  const hostname = colon === -1 ? target : target.slice(0, colon);
  const port = colon === -1 ? 80 : parseInt(target.slice(colon + 1), 10);

  const upstreamHeaders: Record<string, string> = { ...reqHeaders, connection: "close" };
  if (body.length > 0) upstreamHeaders["content-length"] = String(body.length);

  const req = http.request(
    { hostname, port, path: path || "/", method, headers: upstreamHeaders },
    (res) => {
      handleProxyResponse(res, socket, t0, logId, onDone);
    }
  );

  req.on("error", (err) => {
    console.error(`[proxy] upstream ${target} —`, err.message);
    if (socket.writable) {
      sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${target}</code>.</p>`);
    }
    onDone?.(502, Date.now() - t0, {}, "");
  });

  if (body.length > 0) req.write(body);
  req.end();
}

export {
  passthroughToUpstream,
  passthroughToUpstreamHttps,
  fetchUpstreamResponse,
  BufferedProxyResponse
} from "./responseUtils";

export interface RuleMatchResult {
  matched: boolean;
  rule: ProxyRule | null;
  target: string | null;
}

export function matchProxyRule(
  rules: ProxyRule[],
  targetUrl: string,
  mappings: LocalMapping[],
): RuleMatchResult {
  for (const rule of rules) {
    let matches = false;
    if (rule.useRegex) {
      try { matches = new RegExp(rule.pattern).test(targetUrl); } catch { /* invalid regex */ }
    } else {
      matches = targetUrl === rule.pattern;
    }
    if (matches) {
      let target: string | null = null;
      if (rule.targetType === "external") {
        target = rule.targetExternal || null;
      } else {
        target = mappings.find((m) => m.id === rule.targetMappingId)?.target ?? null;
      }
      return { matched: true, rule, target };
    }
  }
  return { matched: false, rule: null, target: null };
}

/**
 * Forward to an upstream target, running request/response scripts if configured.
 * This wraps proxyToUpstream to intercept the response and apply the response script.
 */
export function proxyWithScripts(
  socket: net.Socket,
  method: string,
  target: string,
  path: string,
  reqHeaders: Record<string, string>,
  reqBody: Buffer,
  rule: ProxyRule,
  onDone?: (status: number, durationMs: number, resHeaders: Record<string, string>, resBody: string) => void,
): void {
  let finalHeaders = { ...reqHeaders };
  let finalBody = reqBody;

  // Apply request script
  if (rule.requestScript?.trim()) {
    const result = executeRequestScript(rule.requestScript, finalHeaders, reqBody.toString("utf8"));
    if (!result.error) {
      finalHeaders = result.headers;
      finalBody = Buffer.from(result.body, "utf8");
    }
  }

  if (!rule.responseScript?.trim()) {
    proxyToUpstream(socket, method, target, path, finalHeaders, finalBody, onDone);
    return;
  }

  // With response script: capture full response, run script, then write to socket
  const t0 = Date.now();
  const colon = target.lastIndexOf(":");
  const hostname = colon === -1 ? target : target.slice(0, colon);
  const port = colon === -1 ? 80 : parseInt(target.slice(colon + 1), 10);

  const upstreamHeaders: Record<string, string> = { ...finalHeaders, connection: "close" };
  if (finalBody.length > 0) upstreamHeaders["content-length"] = String(finalBody.length);

  const req = http.request(
    { hostname, port, path: path || "/", method, headers: upstreamHeaders },
    (res) => {
      const chunks: Buffer[] = [];
      res.on("data", (c: Buffer) => chunks.push(c));
      res.on("end", () => {
        const rawResHeaders = parseRawHeaders(res.headers);
        const resBodyBuf = Buffer.concat(chunks);

        // Decompress before passing to the response script so scripts always see plain text
        const ce = rawResHeaders["content-encoding"] ?? "";
        const decompressedBuf = decompressBody(resBodyBuf, ce);
        // Strip content-encoding from the headers we'll forward — body is now plain
        const decompressedHeaders = ce ? stripContentEncoding(rawResHeaders) : rawResHeaders;

        // Apply response script
        const scriptResult = executeResponseScript(rule.responseScript, decompressedHeaders, decompressedBuf.toString("utf8"));
        const finalResHeaders = scriptResult.error ? decompressedHeaders : scriptResult.headers;
        const finalResBody = scriptResult.error ? decompressedBuf : Buffer.from(scriptResult.body, "utf8");

        // Write response to socket
        if (!socket.writable) return;
        let head = `HTTP/1.1 ${res.statusCode} ${res.statusMessage ?? ""}\r\n`;
        for (const [k, v] of Object.entries(finalResHeaders)) head += `${k}: ${v}\r\n`;
        head += `content-length: ${finalResBody.length}\r\nconnection: close\r\n\r\n`;
        socket.write(head);
        socket.write(finalResBody);

        const slice = finalResBody.length <= 512 * 1024 ? finalResBody : finalResBody.slice(0, 512 * 1024);
        onDone?.(res.statusCode ?? 0, Date.now() - t0, finalResHeaders, slice.toString("base64"));
      });
    },
  );

  req.on("error", (err) => {
    console.error(`[proxy+script] upstream ${target} —`, err.message);
    if (socket.writable) sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${target}</code>.</p>`);
    onDone?.(502, Date.now() - t0, {}, "");
  });

  if (finalBody.length > 0) req.write(finalBody);
  req.end();
}
