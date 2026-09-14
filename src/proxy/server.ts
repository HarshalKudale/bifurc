import * as net from "net";
import * as http from "http";
import * as https from "https";
import { loadConfig, AppConfig, Environment, ProxyRule } from "@/store/config";
import { readEnabledSet, bootstrapEnabledSet, readAllEntities } from "@/store/workspaceFs";
import { HOP_BY_HOP } from "@/proxy/constants";
import { sendHtml, buildHomePage, buildNotMappedPage } from "@/proxy/pages";
import { matchMock, serveMock, isFullyMocked, mergeMockWithUpstream, resolveMockOnlyResponse, serveResolvedResponse } from "@/proxy/mockHandler";
import { matchGraphQLMock, matchSoapMock, serveProtocolMock, GraphQLMockDef, SoapMockDef } from "@/proxy/protocolMockHandler";
import { tcpTunnel, proxyToUpstream, passthroughToUpstream, passthroughToUpstreamHttps, matchProxyRule, proxyWithScripts, fetchUpstreamResponse } from "@/proxy/proxyHandler";
import { loadCA, unloadCA, isCALoaded, clearCertCache } from "@/proxy/tlsCert";
import { interceptTls } from "@/proxy/tlsIntercept";
import { decompressBody, stripContentEncoding } from "@/proxy/decompressUtils";
import { handleProxyRouting } from "@/proxy/routingUtils";
import { RequestLogEntry, logEmitter, emitLog, emitLogChunk } from "@/proxy/logEmitter";
export { RequestLogEntry, logEmitter, emitLogChunk } from "@/proxy/logEmitter";

import { mkId, activeEnv, EnabledSets, loadEnabledSets, workspaceCfg } from "./serverUtils";

let server: net.Server | null = null;
let currentPort = 0;
let currentConfig: AppConfig;
let fullRules: ProxyRule[] = [];
let lastError: string | null = null;
let enabledSets: EnabledSets = { mocks: new Set(), mappings: new Set(), rules: new Set() };

export function startServer(port: number): void {
  if (server) stopServer();
  currentConfig = loadConfig();
  fullRules = readAllEntities<ProxyRule>(currentConfig.activeWorkspaceId, "rules");
  enabledSets = loadEnabledSets(currentConfig.activeWorkspaceId);
  currentPort = port;
  lastError = null;

  if (currentConfig.tlsEnabled && currentConfig.tlsCaCertPath && currentConfig.tlsCaKeyPath) {
    loadCA(currentConfig.tlsCaCertPath, currentConfig.tlsCaKeyPath);
  } else {
    unloadCA();
  }

  server = net.createServer((socket) => {
    let buf = Buffer.alloc(0);
    let dispatched = false;

    socket.on("data", (chunk) => {
      if (dispatched) return;
      buf = Buffer.concat([buf, chunk]);
      const sep = buf.indexOf("\r\n\r\n");
      if (sep === -1) return;
      dispatched = true;

      const headerSection = buf.slice(0, sep).toString("utf-8");
      const bodyBuf = buf.slice(sep + 4);
      const lines = headerSection.split("\r\n");
      const parts = lines[0].split(" ");
      const method = parts[0] ?? "GET";
      const rawTarget = parts[1] ?? "/";

      if (method === "CONNECT") {
        const lastColon = rawTarget.lastIndexOf(":");
        const host = rawTarget.slice(0, lastColon);
        const tlsPort = parseInt(rawTarget.slice(lastColon + 1) || "443", 10);
        if (currentConfig.tlsEnabled && isCALoaded()) {
          interceptTls(socket, host, tlsPort, (tlsSocket, decMethod, decTarget, decHeaders, decBody, hostname, hPort) => {
            dispatchHttps(tlsSocket, decMethod, decTarget, decHeaders, decBody, hostname, hPort);
          });
        } else {
          tcpTunnel(socket, host, tlsPort, null, "HTTP/1.1 200 Connection Established\r\n\r\n");
        }
      } else {
        dispatch(socket, method, rawTarget, lines.slice(1), bodyBuf);
      }
    });

    socket.on("error", () => socket.destroy());
  });

  server.on("error", (err: NodeJS.ErrnoException) => {
    if (err.code === "EACCES") {
      lastError = `Permission denied — run as Administrator to bind port ${port}`;
      console.error(`[server:${port}]`, lastError);
    } else if (err.code === "EADDRINUSE") {
      lastError = `Port ${port} is already in use`;
      console.error(`[server:${port}]`, lastError);
    } else {
      lastError = err.message;
      console.error(`[server:${port}] error:`, err.message);
    }
    logEmitter.emit("server-error", lastError);
  });

  server.listen(port, "127.0.0.1", () => {
    lastError = null;
    console.log(`[server:${port}] listening on 127.0.0.1:${port}`);
  });
}

export function stopServer(): void {
  if (server) {
    server.close();
    server = null;
    unloadCA();
    clearCertCache();
    console.log(`[server:${currentPort}] stopped`);
  }
}

export function isRunning(): boolean {
  return server !== null && server.listening;
}

export function getPort(): number {
  return currentPort;
}

export function getServerError(): string | null {
  return lastError;
}

export function reloadConfig(): void {
  currentConfig = loadConfig();
  fullRules = readAllEntities<ProxyRule>(currentConfig.activeWorkspaceId, "rules");
  enabledSets = loadEnabledSets(currentConfig.activeWorkspaceId);
}


// ── Replay a captured request, return {status, headers, body (base64)} ────

export { replayRequest } from "./serverReplay";

// ── HTTP dispatch ─────────────────────────────────────────────────────────





function dispatch(
  socket: net.Socket,
  method: string,
  rawTarget: string,
  headerLines: string[],
  bodyBuf: Buffer,
): void {
  const cfg = workspaceCfg(currentConfig ?? loadConfig(), fullRules, enabledSets);
  const t0 = Date.now();
  const id = mkId();

  // Parse headers, strip hop-by-hop
  const headers: Record<string, string> = {};
  for (const line of headerLines) {
    const idx = line.indexOf(":");
    if (idx === -1) continue;
    const key = line.slice(0, idx).trim().toLowerCase();
    if (!HOP_BY_HOP.has(key)) headers[key] = line.slice(idx + 1).trim();
  }

  const hostHeader = (headers["host"] ?? "").toLowerCase();
  const host = hostHeader.split(":")[0];

  let reqPath = "/";
  if (rawTarget.startsWith("http://") || rawTarget.startsWith("https://")) {
    try { const u = new URL(rawTarget); reqPath = u.pathname + u.search; } catch { reqPath = "/"; }
  } else {
    reqPath = rawTarget || "/";
  }

  const url = rawTarget.startsWith("http") ? rawTarget : `http://${hostHeader}${reqPath}`;
  const reqBodyB64 = bodyBuf.length > 0 ? bodyBuf.toString("base64") : "";

  const baseEntry: Omit<RequestLogEntry, "status" | "via" | "target" | "durationMs" | "resHeaders" | "resBody" | "resStatus"> = {
    id, ts: t0, method, url, host,
    reqHeaders: headers,
    reqBody: reqBodyB64,
  };

  // 1. localhost base → home page (no log)
  if (host === "localhost") {
    sendHtml(socket, 200, buildHomePage(cfg, currentPort));
    return;
  }

  // 2. *.localhost → mapping lookup (RFC 6761) — no mock matching here, these are internal
  if (host.endsWith(".localhost")) {
    const mapping = cfg.mappings.find((m) => m.enabled && m.domain.toLowerCase() === host);
    const env = activeEnv(cfg);
    const mock = matchMock(cfg.mocks, method, url, env);
    if (mock) {
      if (isFullyMocked(mock)) {
        serveMock(socket, mock, env);
        const resolved = resolveMockOnlyResponse(mock, env);
        const slice = resolved.body.length <= 512 * 1024 ? resolved.body : resolved.body.slice(0, 512 * 1024);
        emitLog({ ...baseEntry, status: resolved.status, via: "mock", target: `mock:${mock.id}`, durationMs: Date.now() - t0 + resolved.delayMs, resHeaders: resolved.headers, resBody: slice.toString("base64"), resStatus: resolved.status });
        return;
      }

      if (!mapping) {
        sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>No downstream mapping exists for <code>${host}</code>, so this partial mock cannot fetch live headers.</p>`);
        emitLog({ ...baseEntry, status: 502, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 502 });
        return;
      }

      fetchUpstreamResponse(method, mapping.target, reqPath, headers, bodyBuf).then((upstream) => {
        const merged = mergeMockWithUpstream(mock, upstream, env);
        serveResolvedResponse(socket, merged);
        const slice = merged.body.length <= 512 * 1024 ? merged.body : merged.body.slice(0, 512 * 1024);
        emitLog({ ...baseEntry, status: merged.status, via: "mock", target: `mock:${mock.id}`, durationMs: upstream.durationMs + merged.delayMs, resHeaders: merged.headers, resBody: slice.toString("base64"), resStatus: merged.status });
      }).catch((err: Error) => {
        console.error(`[mock downstream] ${mapping.target}${reqPath} —`, err.message);
        sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${mapping.target}</code>.</p>`);
        emitLog({ ...baseEntry, status: 502, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 502 });
      });
      return;
    }

    if (mapping) {
      proxyToUpstream(socket, method, mapping.target, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
        emitLog({ ...baseEntry, status, via: "rfc6761", target: mapping.target, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
      }, baseEntry.id);
    } else {
      sendHtml(socket, 404, buildNotMappedPage(host, currentPort));
      emitLog({ ...baseEntry, status: 404, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 404 });
    }
    return;
  }

  // 3. Forward proxy path (always active for absolute http:// targets)
  if (rawTarget.startsWith("http://") || rawTarget.startsWith("https://")) {
    // 3a. Proxy routing (mocks, rules)
    const env = activeEnv(cfg);
    if (handleProxyRouting(socket, cfg, env, method, url, rawTarget, reqPath, headers, bodyBuf, reqBodyB64, baseEntry, t0)) {
      return;
    }

    // 3c. Passthrough
    passthroughToUpstream(socket, method, rawTarget, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
      emitLog({ ...baseEntry, status, via: "proxy", target: host, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
    }, baseEntry.id);
    return;
  }

  sendHtml(socket, 400, "<h1>400 Bad Request</h1><p>Not a localhost domain.</p>");
  emitLog({ ...baseEntry, status: 400, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 400 });
}

// ── HTTPS dispatch (TLS intercepted) ─────────────────────────────────────────

import type { TLSSocket } from "tls";

function dispatchHttps(
  socket: TLSSocket,
  method: string,
  rawTarget: string,  // relative path, e.g. "/api/users"
  headerLines: string[],
  bodyBuf: Buffer,
  hostname: string,
  port: number,
): void {
  const cfg = workspaceCfg(currentConfig ?? loadConfig(), fullRules, enabledSets);
  const t0 = Date.now();
  const id = mkId();

  // Parse headers, strip hop-by-hop
  const headers: Record<string, string> = {};
  for (const line of headerLines) {
    const idx = line.indexOf(":");
    if (idx === -1) continue;
    const key = line.slice(0, idx).trim().toLowerCase();
    if (!HOP_BY_HOP.has(key)) headers[key] = line.slice(idx + 1).trim();
  }

  const reqPath = rawTarget || "/";
  const portSuffix = port === 443 ? "" : `:${port}`;
  const url = `https://${hostname}${portSuffix}${reqPath}`;
  const reqBodyB64 = bodyBuf.length > 0 ? bodyBuf.toString("base64") : "";

  const baseEntry: Omit<RequestLogEntry, "status" | "via" | "target" | "durationMs" | "resHeaders" | "resBody" | "resStatus"> = {
    id, ts: t0, method, url, host: hostname,
    reqHeaders: headers,
    reqBody: reqBodyB64,
  };

  // Proxy routing (mocks, rules)
  const env = activeEnv(cfg);
  if (handleProxyRouting(socket, cfg, env, method, url, url, reqPath, headers, bodyBuf, reqBodyB64, baseEntry, t0)) {
    return;
  }

  // 3. Passthrough — forward to real upstream over HTTPS
  passthroughToUpstreamHttps(socket, method, hostname, port, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
    emitLog({ ...baseEntry, status, via: "proxy", target: hostname, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
  }, baseEntry.id);
}
