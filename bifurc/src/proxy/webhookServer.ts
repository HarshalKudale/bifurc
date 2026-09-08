/**
 * Standalone HTTP server for receiving incoming webhook payloads.
 *
 * - Runs on a dedicated port (default 9101) separate from the proxy server.
 * - Base path: POST /bifurc/webhooks/<urlSuffix>
 * - Only webhooks whose tab is currently open (active) accept requests.
 * - Inactive webhooks → 404 JSON error.
 * - GET /bifurc/webhooks/<urlSuffix> → 200 JSON "alive" for easy testing.
 * - Server lifecycle: start/stop via IPC. Status persists across panel navigations.
 */

import * as http from "http";
import { EventEmitter } from "events";
import { BrowserWindow } from "electron";

// ── Types ──────────────────────────────────────────────────────────────────────

export interface WebhookPayload {
  webhookId: string;
  urlSuffix: string;
  ts: number;
  method: string;
  headers: Record<string, string>;
  body: string;
}

// ── Module-level state ────────────────────────────────────────────────────────

let server: http.Server | null = null;
let currentPort = 9101;
let lastError: string | null = null;

/** Set of urlSuffixes for webhooks currently open in a tab (active). */
const activeWebhookSuffixes = new Set<string>();
/** Map of urlSuffix → webhookId for active webhooks. */
const activeSuffixToId = new Map<string, string>();

export const webhookEmitter = new EventEmitter();

// ── Active webhook registry (called from renderer IPC) ────────────────────────

export function registerActiveWebhook(webhookId: string, urlSuffix: string): void {
  const normalized = normalizeSuffix(urlSuffix);
  activeWebhookSuffixes.add(normalized);
  activeSuffixToId.set(normalized, webhookId);
}

export function unregisterActiveWebhook(webhookId: string): void {
  for (const [suffix, id] of activeSuffixToId.entries()) {
    if (id === webhookId) {
      activeWebhookSuffixes.delete(suffix);
      activeSuffixToId.delete(suffix);
      break;
    }
  }
}

export function getActiveWebhookCount(): number {
  return activeWebhookSuffixes.size;
}

/** Maximum number of simultaneously active webhook listeners. */
export const MAX_ACTIVE_WEBHOOKS = 5;

function normalizeSuffix(suffix: string): string {
  // Strip leading slashes; lowercase; remove trailing slash
  return suffix.replace(/^\/+/, "").replace(/\/+$/, "").toLowerCase();
}

// ── Server lifecycle ──────────────────────────────────────────────────────────

export function startWebhookServer(port: number): void {
  if (server) stopWebhookServer();
  currentPort = port;
  lastError = null;

  server = http.createServer((req, res) => {
    const url = req.url ?? "/";
    const BASE = "/bifurc/webhooks/";

    if (!url.startsWith(BASE)) {
      res.writeHead(404, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "Not found" }));
      return;
    }

    const rawSuffix = url.slice(BASE.length);
    const suffix = normalizeSuffix(rawSuffix);

    // GET: alive check (regardless of active status)
    if (req.method === "GET") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ status: "ok", webhook: suffix || "/" }));
      return;
    }

    if (req.method !== "POST") {
      res.writeHead(405, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "Method not allowed. Use POST." }));
      return;
    }

    // Only accept if this webhook is active (open in a tab)
    if (!activeWebhookSuffixes.has(suffix)) {
      res.writeHead(404, { "Content-Type": "application/json" });
      res.end(JSON.stringify({
        error: "Webhook not active",
        detail: "Open this webhook in a tab to activate it.",
      }));
      return;
    }

    const webhookId = activeSuffixToId.get(suffix) ?? "";
    const headers: Record<string, string> = {};
    for (const [k, v] of Object.entries(req.headers)) {
      if (typeof v === "string") headers[k] = v;
      else if (Array.isArray(v)) headers[k] = v.join(", ");
    }

    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const body = Buffer.concat(chunks).toString("utf-8");
      const payload: WebhookPayload = {
        webhookId,
        urlSuffix: suffix,
        ts: Date.now(),
        method: req.method ?? "POST",
        headers,
        body,
      };

      // Emit to main process listeners (IPC forward)
      webhookEmitter.emit("payload", payload);

      // Forward to all renderer windows
      BrowserWindow.getAllWindows().forEach((w) => {
        if (!w.isDestroyed()) {
          w.webContents.send("webhook:payload", payload);
        }
      });

      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ ok: true, received: true }));
    });

    req.on("error", () => {
      res.writeHead(400, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "Bad request" }));
    });
  });

  server.on("error", (err: NodeJS.ErrnoException) => {
    lastError = err.message;
    webhookEmitter.emit("error", lastError);
    server = null;
  });

  server.listen(port, "127.0.0.1", () => {
    lastError = null;
  });
}

export function stopWebhookServer(): void {
  if (server) {
    server.close();
    server = null;
  }
}

export function isWebhookServerRunning(): boolean {
  return server !== null && server.listening;
}

export function getWebhookPort(): number {
  return currentPort;
}

export function getWebhookServerError(): string | null {
  return lastError;
}
