import * as net from "net";
import { AppConfig, Environment } from "../store/config";
import { RequestLogEntry, emitLog } from "./logEmitter";
import { sendHtml } from "./pages";
import { matchMock, serveMock, isFullyMocked, mergeMockWithUpstream, resolveMockOnlyResponse, serveResolvedResponse } from "./mockHandler";
import { matchProxyRule, proxyWithScripts, proxyToUpstream, fetchUpstreamResponse } from "./proxyHandler";
import { replayRequest } from "./server";

export function handleProxyRouting(
  socket: net.Socket,
  cfg: AppConfig,
  env: Environment | null,
  method: string,
  url: string,
  ruleMatchTarget: string,
  reqPath: string,
  headers: Record<string, string>,
  bodyBuf: Buffer,
  reqBodyB64: string,
  baseEntry: Omit<RequestLogEntry, "status" | "via" | "target" | "durationMs" | "resHeaders" | "resBody" | "resStatus">,
  t0: number
): boolean {
  // Mock check — highest priority
  const mock = matchMock(cfg.mocks, method, url, env);
  if (mock) {
    if (isFullyMocked(mock)) {
      serveMock(socket, mock, env);
      const resolved = resolveMockOnlyResponse(mock, env);
      const slice = resolved.body.length <= 512 * 1024 ? resolved.body : resolved.body.slice(0, 512 * 1024);
      emitLog({ ...baseEntry, status: resolved.status, via: "mock", target: `mock:${mock.id}`, durationMs: Date.now() - t0 + resolved.delayMs, resHeaders: resolved.headers, resBody: slice.toString("base64"), resStatus: resolved.status });
      return true;
    }

    const { matched, rule: matchedRule, target: ruleTarget } = matchProxyRule(cfg.proxyRules, ruleMatchTarget, cfg.mappings);
    if (matched && (!matchedRule || !ruleTarget)) {
      sendHtml(socket, 502, "<h1>502 Bad Gateway</h1><p>Proxy rule matched but the target is not configured.</p>");
      emitLog({ ...baseEntry, status: 502, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 502 });
      return true;
    }
    const upstreamPromise = matched && matchedRule && ruleTarget
      ? fetchUpstreamResponse(method, ruleTarget, reqPath, headers, bodyBuf, matchedRule)
      : replayRequest(method, url, headers, reqBodyB64).then((res) => ({
        status: res.status,
        headers: res.headers,
        body: Buffer.from(res.body, "base64"),
        durationMs: Date.now() - t0,
      }));

    upstreamPromise.then((upstream) => {
      const merged = mergeMockWithUpstream(mock, upstream, env);
      serveResolvedResponse(socket, merged);
      const slice = merged.body.length <= 512 * 1024 ? merged.body : merged.body.slice(0, 512 * 1024);
      emitLog({ ...baseEntry, status: merged.status, via: "mock", target: `mock:${mock.id}`, durationMs: upstream.durationMs + merged.delayMs, resHeaders: merged.headers, resBody: slice.toString("base64"), resStatus: merged.status });
    }).catch((err: Error) => {
      console.error(`[mock downstream] ${url} —`, err.message);
      sendHtml(socket, 502, `<h1>502 Bad Gateway</h1><p>Could not connect to <code>${url}</code>.</p>`);
      emitLog({ ...baseEntry, status: 502, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 502 });
    });
    return true;
  }

  // Proxy rule match
  const { matched, rule: matchedRule, target: ruleTarget } = matchProxyRule(cfg.proxyRules, ruleMatchTarget, cfg.mappings);
  if (matched) {
    if (matchedRule && ruleTarget) {
      const hasScripts = !!(matchedRule.requestScript?.trim() || matchedRule.responseScript?.trim());
      if (hasScripts) {
        proxyWithScripts(socket, method, ruleTarget, reqPath, headers, bodyBuf, matchedRule, (status, dur, resH, resB) => {
          emitLog({ ...baseEntry, status, via: "rule", target: ruleTarget, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
        });
      } else {
        proxyToUpstream(socket, method, ruleTarget, reqPath, headers, bodyBuf, (status, dur, resH, resB) => {
          emitLog({ ...baseEntry, status, via: "rule", target: ruleTarget, durationMs: dur, resHeaders: resH, resBody: resB, resStatus: status });
        }, baseEntry.id);
      }
    } else {
      sendHtml(socket, 502, "<h1>502 Bad Gateway</h1><p>Proxy rule matched but the target is not configured.</p>");
      emitLog({ ...baseEntry, status: 502, via: "error", target: null, durationMs: Date.now() - t0, resHeaders: {}, resBody: "", resStatus: 502 });
    }
    return true;
  }

  return false; // Did not handle via routing, caller should passthrough
}
