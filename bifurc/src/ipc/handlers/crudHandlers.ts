import { ipcMain } from "electron";
import { registerEntityCrudHandlers } from "@/ipc/handlers/entityCrudFactory";
import { loadConfig, LocalMapping, ProxyRule, MockRule, SavedRequest, SavedWsConnection, SavedWebhook } from "@/store/config";
import { registerActiveWebhook, unregisterActiveWebhook, startWebhookServer, stopWebhookServer, isWebhookServerRunning, getWebhookPort, getWebhookServerError } from "@/proxy/webhookServer";

export function registerCrudHandlers() {
  // ── Mappings ───────────────────────────────────────────────────────────────

  registerEntityCrudHandlers<LocalMapping>({
    ipcPrefix: "mapping",
    kind: "mappings",
    configKey: "mappings",
    isFlat: true,
    hasEnabledState: true,
  });

  // ── Proxy Rules ────────────────────────────────────────────────────────────

  registerEntityCrudHandlers<ProxyRule>({
    ipcPrefix: "rule",
    kind: "rules",
    configKey: "proxyRules",
    folderConfigKey: "ruleFolders",
    hasEnabledState: true,
    onAddConflict: (cfg, target) => {
      if (!target.enabled) return;
      const sig = `${target.useRegex ? "re" : "exact"}|${target.pattern}`;
      for (const r of cfg.proxyRules) {
        if (r.id !== target.id && r.enabled && `${r.useRegex ? "re" : "exact"}|${r.pattern}` === sig) {
          r.enabled = false;
        }
      }
    },
    getNameEntry: (r) => ({ name: r.name, url: r.pattern }),
  });

  // ── Mocks ──────────────────────────────────────────────────────────────────

  registerEntityCrudHandlers<MockRule>({
    ipcPrefix: "mock",
    kind: "mocks",
    configKey: "mocks",
    folderConfigKey: "mockFolders",
    hasEnabledState: true,
    validate: (mock) => {
      if (!mock.urlPattern || !mock.urlPattern.trim()) throw new Error("urlPattern is required for mocks");
      if (!mock.method || !mock.method.trim()) throw new Error("method is required for mocks");
    },
    onAddConflict: (cfg, target) => {
      if (!target.enabled) return;
      const sig = `${target.method.toUpperCase()}|${target.urlPattern}|${target.capturedBody ?? ""}`;
      for (const m of cfg.mocks) {
        if (m.id !== target.id && m.enabled && `${m.method.toUpperCase()}|${m.urlPattern}|${m.capturedBody ?? ""}` === sig) {
          m.enabled = false;
        }
      }
    },
    getNameEntry: (m) => ({ name: m.name, method: m.method, url: m.urlPattern }),
  });

  // ── Saved Requests ─────────────────────────────────────────────────────────

  registerEntityCrudHandlers<SavedRequest>({
    ipcPrefix: "request",
    kind: "requests",
    configKey: "requests",
    folderConfigKey: "requestFolders",
    validate: (req) => {
      if (!req.url || !req.url.trim()) throw new Error("url is required for requests");
      if (!req.method || !req.method.trim()) throw new Error("method is required for requests");
    },
    getNameEntry: (req) => ({ name: req.name, method: req.method, url: req.url }),
  });

  // ── WebSocket Connections ──────────────────────────────────────────────────

  registerEntityCrudHandlers<SavedWsConnection>({
    ipcPrefix: "ws",
    kind: "sockets",
    configKey: "wsConnections",
    folderConfigKey: "wsFolders",
    getNameEntry: (conn) => ({ name: conn.name, url: conn.url }),
  });

  // ── Webhooks ───────────────────────────────────────────────────────────────

  registerEntityCrudHandlers<SavedWebhook>({
    ipcPrefix: "webhook",
    kind: "webhooks",
    configKey: "webhooks",
    folderConfigKey: "webhookFolders",
    getNameEntry: (hook) => ({ name: hook.name, urlSuffix: hook.urlSuffix }),
  });

  ipcMain.handle("webhook:registerActive", (_e, webhookId: string, urlSuffix: string) => {
    registerActiveWebhook(webhookId, urlSuffix);
    return { ok: true };
  });

  ipcMain.handle("webhook:unregisterActive", (_e, webhookId: string) => {
    unregisterActiveWebhook(webhookId);
    return { ok: true };
  });

  ipcMain.handle("webhookServer:start", () => {
    const cfg = loadConfig();
    startWebhookServer(cfg.webhookPort ?? 9101);
    return { ok: true };
  });

  ipcMain.handle("webhookServer:stop", () => {
    stopWebhookServer();
    return { ok: true };
  });

  ipcMain.handle("webhookServer:status", () => ({
    running: isWebhookServerRunning(),
    port: getWebhookPort(),
    error: getWebhookServerError(),
  }));
}
