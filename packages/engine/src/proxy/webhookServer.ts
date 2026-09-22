/**
 * Standalone HTTP server for receiving incoming webhook payloads.
 *
 * - Runs on a dedicated port (default 9101) separate from the proxy server.
 * - Base path: POST /bifurc/webhooks/<urlSuffix>
 * - Only webhooks whose tab is currently open (active) accept requests.
 * - Inactive webhooks → 404 JSON error.
 * - GET /bifurc/webhooks/<urlSuffix> → 200 JSON "alive" for easy testing.
 * - Server lifecycle: start/stop via IPC. Status persists across panel navigations.
 *
 * ## P14 work item 2 — the factory, and why the facade is still here
 *
 * `createWebhookServer()` returns an instance owning every piece of state this module used to hold at
 * module scope (the `http.Server`, `currentPort`, `lastError`, and the two active-webhook maps), so a
 * host can run one receiver per workspace. §6 of `plan/14` chose this file — not `proxy/server.ts` — as
 * the place to learn what de-singletonizing costs: ~180 lines, no TLS, no CONNECT tunnelling, and only
 * two maps and a port as global state.
 *
 * **The module-level functions below are a facade over one lazily-created default instance, and they
 * stay until WI 7 flips the desktop onto the runtime.** §9's rollback says WI 1–2 "keep the module-level
 * facade alongside the factory until WI 7 flips the shell. Reverting is deleting the factory and
 * restoring the facade." So this is deliberately two entry points with one implementation: every
 * existing caller (`webhookCommands.ts`, `tests/proxy/webhookServer.test.ts`) keeps working unchanged,
 * and nothing about `window.api` moves.
 *
 * **Two things a naive factory gets wrong, both guarded here:**
 *
 * 1. **`MAX_ACTIVE_WEBHOOKS` becomes per instance but stays a module constant.** `plan/14` WI 2 says the
 *    cap becomes "per workspace — a product decision". Per-workspace scope means each instance enforces
 *    its own count against its own map; it does **not** mean the number differs per instance, so the
 *    value stays the exported `5` rather than becoming a constructor argument. (`renderer/…/
 *    useWebhooksPanelState.ts:11` holds a duplicate `5` for the UI hint — WI 2 must still decide whether
 *    that is pushed to the renderer, which cannot happen before P7.)
 * 2. **`webhookEmitter` is NOT per instance.** It is the process-wide EventEmitter the shell's forward and
 *    the suite subscribe to; a fresh emitter per instance would silently detach both. The instance emits
 *    to the shared emitter *and* the shared bus, exactly as before.
 *
 * The per-workspace *routing* key (`(workspaceId, urlSuffix)` in §2.3) is **WI 1's** protocol change, not
 * this one: this step is the mechanical de-singletonization, and carrying the parameter on
 * `webhook.registerActive` would be a behaviour change with no test to drive it. Noted so the two steps are
 * not conflated.
 *
 * **The maps are already per-instance, which is the part that matters** (re-checked 2026-09-21): both
 * `activeWebhookSuffixes` and `activeSuffixToId` are declared *inside* `createWebhookServer()`, so two
 * instances cannot see each other's suffixes and the acceptance criterion ("a suffix registered in workspace
 * A is `404` in workspace B") already holds by construction. What WI 1 adds is the ability for one instance
 * to serve *several* workspaces at once; nothing here needs to change for the N-instances case.
 */

import * as http from "http";
import { EventEmitter } from "events";
import { bus } from "../eventBus";

// ── Types ──────────────────────────────────────────────────────────────────────

export interface WebhookPayload {
  webhookId: string;
  urlSuffix: string;
  ts: number;
  method: string;
  headers: Record<string, string>;
  body: string;
}

/** The instance a factory call returns. Mirrors the module's old surface, one per runtime. */
export interface WebhookServerInstance {
  readonly workspaceId: string | null;
  registerActive(webhookId: string, urlSuffix: string): void;
  unregisterActive(webhookId: string): void;
  activeCount(): number;
  start(port: number): void;
  stop(): void;
  isRunning(): boolean;
  getPort(): number;
  getError(): string | null;
}

// ── Shared, process-wide ──────────────────────────────────────────────────────

export const webhookEmitter = new EventEmitter();

/** Maximum number of simultaneously active webhook listeners **per workspace**. */
export const MAX_ACTIVE_WEBHOOKS = 5;

/** The fallback port for a receiver that has not been started, matching `webhookCommands.ts`'s own. */
const DEFAULT_WEBHOOK_PORT = 9101;

function normalizeSuffix(suffix: string): string {
  // Strip leading slashes; lowercase; remove trailing slash
  return suffix.replace(/^\/+/, "").replace(/\/+$/, "").toLowerCase();
}

// ── The factory ───────────────────────────────────────────────────────────────

/**
 * Create one webhook receiver. Every mutable the module used to hold at module scope lives here instead,
 * so two callers get two independent receivers, two ports and two error states.
 *
 * `workspaceId` is carried for §2.3's routing key and for diagnostics; it is not yet used to key the
 * active maps (see the header). Passing `null` is the app-level/pre-runtime case.
 */
export function createWebhookServer(opts: { workspaceId?: string | null } = {}): WebhookServerInstance {
  const workspaceId = opts.workspaceId ?? null;

  let server: http.Server | null = null;
  let currentPort = DEFAULT_WEBHOOK_PORT;
  let lastError: string | null = null;

  /** Set of urlSuffixes for webhooks currently open in a tab (active). */
  const activeWebhookSuffixes = new Set<string>();
  /** Map of urlSuffix → webhookId for active webhooks. */
  const activeSuffixToId = new Map<string, string>();

  function stop(): void {
    if (server) {
      server.close();
      server = null;
    }
  }

  function start(port: number): void {
    if (server) stop();
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

        // Emit on the bus — that is how a payload reaches a client. The shell carries it over
        // `EVENT_CHANNEL`; the `webhookEmitter.emit` above is the in-process shortcut that predates it.
        bus.emitTyped("webhook.payload", payload);

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

  return {
    workspaceId,
    registerActive(webhookId, urlSuffix) {
      const normalized = normalizeSuffix(urlSuffix);
      activeWebhookSuffixes.add(normalized);
      activeSuffixToId.set(normalized, webhookId);
    },
    unregisterActive(webhookId) {
      for (const [suffix, id] of activeSuffixToId.entries()) {
        if (id === webhookId) {
          activeWebhookSuffixes.delete(suffix);
          activeSuffixToId.delete(suffix);
          break;
        }
      }
    },
    activeCount() {
      return activeWebhookSuffixes.size;
    },
    start,
    stop,
    isRunning() {
      return server !== null && server.listening;
    },
    getPort() {
      return currentPort;
    },
    getError() {
      return lastError;
    },
  };
}

// ── The module-level facade ────────────────────────────────────────────────────
//
// One lazily-created default instance backing the pre-P14 module functions. Lazy rather than eager so
// that importing this module has no side effect on the shared `webhookEmitter` or the bus — the same
// property the old module-scope `let server = null` had.
//
// **Not "removed in WI 7", which is what this header used to say.** WI 7's desktop half landed 2026-09-22
// and kept this facade *deliberately*: `renderer/panels/WebhooksPanel.tsx` starts the receiver through
// `window.api.startWebhookServer()`, so `webhookServer.start`'s `facade` branch is a live path and not a
// legacy one. What changed is that it can now be **bound** — see `bindWebhookInstance()`.

let defaultInstance: WebhookServerInstance | null = null;

/**
 * P14 WI 7 — the receiver every module function acts on, when someone has adopted one.
 *
 * The mirror of `proxy/server.ts`'s `bindProxyInstance()`, including *why* it is a binding rather than a
 * `getRuntime(activeWorkspaceId)` lookup: WI 4 made the desktop's active pointer independent of its running
 * servers, so resolving by the active id would lose the real receiver after a workspace switch. That file's
 * docblock carries the full argument; it is not repeated here.
 *
 * **The desktop binds without starting.** `src/main.ts` binds its one `WorkspaceRuntime`'s webhook here but
 * calls `runtime.proxy.start(port)`, never `runtime.start()` — the latter binds both servers, and the
 * receiver has always been user-started. Binding must not open a port.
 */
let boundInstance: WebhookServerInstance | null = null;

/**
 * Adopt `instance` as the receiver every module function acts on, or `null` to return to the lazy default.
 *
 * Idempotent and last-write-wins; `src/main.ts` calls it once, a test calls it with `null` to get a clean
 * module back.
 */
export function bindWebhookInstance(instance: WebhookServerInstance | null): void {
  boundInstance = instance;
}

/** The receiver the module functions act on. Creates the lazy default only when nothing is bound. */
function defaultServer(): WebhookServerInstance {
  if (boundInstance) return boundInstance;
  if (!defaultInstance) defaultInstance = createWebhookServer({ workspaceId: null });
  return defaultInstance;
}

/**
 * The receiver to *read* from, without creating one — the mirror of `proxy/server.ts`'s `existingServer()`.
 * `stopWebhookServer()` / `isWebhookServerRunning()` / `getWebhookPort()` / `getWebhookServerError()` are
 * reached from status paths and must not create an instance as a side effect of being asked.
 */
function existingServer(): WebhookServerInstance | null {
  return boundInstance ?? defaultInstance;
}

// ── Active webhook registry (called from renderer IPC) ────────────────────────
//
// The map these three act on is **per instance**, so they follow the binding like everything else.

export function registerActiveWebhook(webhookId: string, urlSuffix: string): void {
  defaultServer().registerActive(webhookId, urlSuffix);
}

export function unregisterActiveWebhook(webhookId: string): void {
  defaultServer().unregisterActive(webhookId);
}

export function getActiveWebhookCount(): number {
  return defaultServer().activeCount();
}

// ── Server lifecycle ──────────────────────────────────────────────────────────

export function startWebhookServer(port: number): void {
  defaultServer().start(port);
}

export function stopWebhookServer(): void {
  existingServer()?.stop();
}

export function isWebhookServerRunning(): boolean {
  return existingServer()?.isRunning() ?? false;
}

export function getWebhookPort(): number {
  return existingServer()?.getPort() ?? DEFAULT_WEBHOOK_PORT;
}

export function getWebhookServerError(): string | null {
  return existingServer()?.getError() ?? null;
}

