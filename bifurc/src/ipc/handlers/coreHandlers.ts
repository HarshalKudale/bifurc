import { ipcMain, BrowserWindow } from "electron";
import * as fs from "fs";
import * as path from "path";
import { executeIpcScript, IpcScriptOpts } from "@/proxy/scriptExecutor";
import {
  loadConfig, saveConfig, loadEntity, generateId, AppConfig,
  Environment, Workspace,
} from "@/store/config";
import {
  writeFlatEntity, deleteFlatEntityFile,
  initWorkspaceDir, wsDir as workspaceDir, readEnabledSet
} from "@/store/workspaceFs";
import { initWorkspaceRepo } from "@/store/gitStore";
import { discoverServices } from "@/proxy/service-discovery";
import {
  startServer, stopServer, isRunning, getPort, getServerError,
  reloadConfig, replayRequest,
} from "@/proxy/server";
import { logEmitter, RequestLogEntry } from "@/proxy/logEmitter";
import { updateTrayMenu } from "@/main";
import { generateRandomWorkspaceName } from "@/lib/randomNames";
import { gateCreate } from "@/subscription/entityCount";
import { syncEnabledSet } from "@/ipc/handlers/utils";
import { getWorkspaceSyncStatus, invalidateCache } from "@/sync/statusTracker";

function broadcastEntityStatus(wsId: string): void {
  getWorkspaceSyncStatus(wsId).then((status) => {
    BrowserWindow.getAllWindows().forEach((w) => {
      if (!w.isDestroyed()) w.webContents.send("sync:entityStatus", { wsId, status });
    });
  }).catch(() => { });
}

export function registerCoreHandlers() {
  ipcMain.handle("config:get", () => loadConfig());

  ipcMain.handle("config:save", (_e, incoming: AppConfig) => {
    const prev = loadConfig();
    saveConfig(incoming);
    reloadConfig();
    updateTrayMenu();
    const tlsChanged = incoming.tlsEnabled !== prev.tlsEnabled
      || incoming.tlsCaCertPath !== prev.tlsCaCertPath
      || incoming.tlsCaKeyPath !== prev.tlsCaKeyPath;
    if (!isRunning() || incoming.port !== prev.port || tlsChanged) {
      stopServer();
      startServer(incoming.port);
    }
    if (incoming.companionPort !== prev.companionPort) {
      const { restartCompanionServer } = require("@/companion/companionServer");
      restartCompanionServer(incoming.companionPort);
    }
    return { ok: true };
  });

  ipcMain.handle("entity:load", (_e, wsId: string, kind: string, id: string) => {
    const entity = loadEntity(wsId, kind, id);
    if (!entity) return { ok: false };
    const enabledKinds = new Set(["mocks", "mappings", "rules", "graphqlMocks", "soapMocks", "grpcMocks"]);
    if (enabledKinds.has(kind)) {
      const set = readEnabledSet(wsId, kind);
      return { ok: true, entity: { ...entity as object, enabled: set ? set.has(id) : false } };
    }
    return { ok: true, entity };
  });

  ipcMain.handle("entity:setEnabled", async (_e, wsId: string, kind: string, id: string, enabled: boolean) => {
    const enabledKinds = new Set(["mocks", "mappings", "rules", "graphqlMocks", "soapMocks", "grpcMocks"]);
    if (!enabledKinds.has(kind)) return { ok: false, error: "invalid_kind" };

    if (enabled && kind === "mocks") {
      const cfg = loadConfig();
      const target = (cfg.mocks ?? []).find((m) => m.id === id);
      if (target) {
        const sig = `${target.method.toUpperCase()}|${target.urlPattern}|${target.capturedBody ?? ""}`;
        for (const m of cfg.mocks) {
          if (m.id !== id && m.enabled) {
            const mSig = `${m.method.toUpperCase()}|${m.urlPattern}|${m.capturedBody ?? ""}`;
            if (mSig === sig) syncEnabledSet(wsId, "mocks", m.id, false);
          }
        }
      }
    }
    if (enabled && kind === "rules") {
      const cfg = loadConfig();
      const target = (cfg.proxyRules ?? []).find((r) => r.id === id);
      if (target) {
        const sig = `${target.useRegex ? "re" : "exact"}|${target.pattern}`;
        for (const r of cfg.proxyRules) {
          if (r.id !== id && r.enabled) {
            const rSig = `${r.useRegex ? "re" : "exact"}|${r.pattern}`;
            if (rSig === sig) syncEnabledSet(wsId, "rules", r.id, false);
          }
        }
      }
    }

    syncEnabledSet(wsId, kind, id, enabled);
    reloadConfig();
    broadcastEntityStatus(wsId);
    return { ok: true };
  });

  ipcMain.handle("services:discover", () => discoverServices());

  ipcMain.handle("env:add", async (_e, env: Omit<Environment, "id" | "createdAt">) => {
    const cfg = loadConfig();
    const wsId = env.workspaceId ?? cfg.activeWorkspaceId;
    const gate = gateCreate(wsId, "environment");
    if (!gate.allowed) return { error: "limit_reached", ...gate };
    const newEnv: Environment = { ...env, id: generateId(), createdAt: Date.now(), workspaceId: wsId };
    cfg.environments = cfg.environments ?? [];
    cfg.environments.push(newEnv);
    saveConfig(cfg);
    writeFlatEntity(wsId, "environments", newEnv.id, newEnv);
    return newEnv;
  });

  ipcMain.handle("env:update", async (_e, env: Environment) => {
    const cfg = loadConfig();
    const wsId = env.workspaceId ?? cfg.activeWorkspaceId;
    cfg.environments = cfg.environments ?? [];
    const idx = cfg.environments.findIndex((e) => e.id === env.id);
    if (idx !== -1) cfg.environments[idx] = env;
    saveConfig(cfg);
    reloadConfig();
    writeFlatEntity(wsId, "environments", env.id, env);
    return { ok: true };
  });

  ipcMain.handle("env:delete", async (_e, id: string) => {
    if (id === "__global__") return { ok: false, error: "cannot_delete_global" };
    const cfg = loadConfig();
    const env = (cfg.environments ?? []).find((e) => e.id === id);
    cfg.environments = (cfg.environments ?? []).filter((e) => e.id !== id);
    if (cfg.activeEnvironmentId === id) cfg.activeEnvironmentId = null;
    saveConfig(cfg);
    reloadConfig();
    if (env) {
      deleteFlatEntityFile(env.workspaceId, "environments", id);
    }
    return { ok: true };
  });

  ipcMain.handle("env:setActive", (_e, id: string | null) => {
    if (id === "__global__") return { ok: false, error: "cannot_activate_global" };
    const cfg = loadConfig();
    cfg.activeEnvironmentId = id;
    const ws = (cfg.workspaces ?? []).find((w) => w.id === cfg.activeWorkspaceId);
    if (ws) ws.activeEnvironmentId = id;
    saveConfig(cfg);
    reloadConfig();
    return { ok: true };
  });

  ipcMain.handle("script:execute", (_e, opts: IpcScriptOpts) => {
    return executeIpcScript(opts);
  });

  ipcMain.handle("workspace:add", async (_e, name: string) => {
    const cfg = loadConfig();
    const gate = gateCreate(cfg.activeWorkspaceId, "workspace");
    if (!gate.allowed) return { error: "limit_reached", ...gate };
    const finalName = name.trim() || generateRandomWorkspaceName();
    const newWs: Workspace = { id: generateId(), name: finalName, createdAt: Date.now(), activeEnvironmentId: null };
    cfg.workspaces = cfg.workspaces ?? [];
    cfg.workspaces.push(newWs);
    saveConfig(cfg);
    try {
      initWorkspaceDir(newWs.id, newWs.name);
      await initWorkspaceRepo(newWs.id);
    } catch { }
    return newWs;
  });

  ipcMain.handle("workspace:rename", async (_e, id: string, name: string) => {
    const cfg = loadConfig();
    const ws = (cfg.workspaces ?? []).find((w) => w.id === id);
    if (ws) ws.name = name.trim() || ws.name;
    saveConfig(cfg);
    return { ok: true };
  });

  ipcMain.handle("workspace:delete", async (_e, id: string) => {
    const cfg = loadConfig();
    const ws = (cfg.workspaces ?? []).find((w) => w.id === id);
    cfg.workspaces = (cfg.workspaces ?? []).filter((w) => w.id !== id);
    if (cfg.activeWorkspaceId === id) {
      const first = cfg.workspaces[0];
      if (first) {
        cfg.activeWorkspaceId = first.id;
        cfg.activeEnvironmentId = first.activeEnvironmentId;
      }
    }
    saveConfig(cfg);
    reloadConfig();
    return { ok: true };
  });

  ipcMain.handle("workspace:setActive", (_e, id: string) => {
    const cfg = loadConfig();
    const ws = (cfg.workspaces ?? []).find((w) => w.id === id);
    if (!ws) return { ok: false };
    cfg.activeWorkspaceId = id;
    cfg.activeEnvironmentId = ws.activeEnvironmentId;
    saveConfig(cfg);
    reloadConfig();
    return { ok: true, config: loadConfig() };
  });

  ipcMain.handle("request:replay",
    (_e, method: string, url: string, headers: Record<string, string>, bodyBase64: string) =>
      replayRequest(method, url, headers, bodyBase64)
  );

  ipcMain.handle("server:status", () => ({ running: isRunning(), port: getPort(), error: getServerError() }));
  ipcMain.handle("proxy:status", () => ({ running: isRunning() }));

  ipcMain.handle("healthbar:getServices", (_e, wsId: string) => {
    const dir = path.join(workspaceDir(wsId), "healthbar");
    const file = path.join(dir, "services.json");
    if (!fs.existsSync(file)) return [];
    try {
      return JSON.parse(fs.readFileSync(file, "utf-8"));
    } catch {
      return [];
    }
  });

  ipcMain.handle("healthbar:saveServices", (_e, wsId: string, services: unknown[]) => {
    const dir = path.join(workspaceDir(wsId), "healthbar");
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, "services.json");
    fs.writeFileSync(file, JSON.stringify(services, null, 2), "utf-8");
    invalidateCache(wsId);
    broadcastEntityStatus(wsId);
    return { ok: true };
  });

  ipcMain.handle("healthbar:checkUrl", async (_e, url: string) => {
    const start = Date.now();
    return new Promise<{
      ok: boolean;
      statusCode: number | null;
      body: string | null;
      headers: Record<string, string> | null;
      error: string | null;
      durationMs: number;
    }>((resolve) => {
      try {
        const parsedUrl = new URL(url);
        const mod: typeof import("https") = parsedUrl.protocol === "https:"
          ? require("https")
          : require("http");
        const req = (mod as any).get(
          url,
          { timeout: 10000, rejectUnauthorized: false },
          (res: any) => {
            const chunks: Buffer[] = [];
            res.on("data", (chunk: Buffer) => chunks.push(chunk));
            res.on("end", () => {
              const rawBody = Buffer.concat(chunks).toString("utf-8");
              resolve({
                ok: true,
                statusCode: res.statusCode as number,
                body: rawBody.slice(0, 10000),
                headers: res.headers as Record<string, string>,
                error: null,
                durationMs: Date.now() - start,
              });
            });
            res.on("error", (err: Error) => {
              resolve({ ok: false, statusCode: null, body: null, headers: null, error: err.message, durationMs: Date.now() - start });
            });
          }
        );
        req.on("error", (err: Error) => {
          resolve({ ok: false, statusCode: null, body: null, headers: null, error: err.message, durationMs: Date.now() - start });
        });
        req.on("timeout", () => {
          req.destroy();
          resolve({ ok: false, statusCode: null, body: null, headers: null, error: "Request timed out", durationMs: Date.now() - start });
        });
      } catch (err: any) {
        resolve({ ok: false, statusCode: null, body: null, headers: null, error: err?.message ?? "Invalid URL", durationMs: Date.now() - start });
      }
    });
  });
}
