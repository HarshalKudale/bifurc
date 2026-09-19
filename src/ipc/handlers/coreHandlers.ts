import { ipcMain } from "electron";
import * as fs from "fs";
import * as path from "path";
import { executeIpcScript, IpcScriptOpts } from "@bifurc/engine/proxy/scriptExecutor";
import {
  loadConfig, saveConfig, generateId, AppConfig,
  Environment, Workspace,
} from "@bifurc/engine/store/config";
import {
  initWorkspaceDir, wsDir as workspaceDir
} from "@bifurc/engine/store/workspaceFs";
import { initWorkspaceRepo } from "@bifurc/engine/store/gitStore";
import { discoverServices } from "@bifurc/engine/proxy/service-discovery";
import {
  startServer, stopServer, isRunning, getPort, getServerError,
  reloadConfig, replayRequest,
} from "@bifurc/engine/proxy/server";
import { bus, emitEntityStatus } from "@bifurc/engine/eventBus";
import { restartCompanionServer } from "@bifurc/engine/transport/legacyCompanion";
import { generateRandomWorkspaceName } from "@bifurc/engine/lib/randomNames";
import { gateCreate } from "@bifurc/engine/subscription/entityCount";
import { invalidateCache } from "@bifurc/engine/sync/statusTracker";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { toProtocolKind } from "@bifurc/engine/commands/entityKindMap";
import { registerEntityCrudHandlers } from "@/ipc/handlers/entityCrudFactory";

// ## The five commands that used to be registered here (P6 §5c, moved 2026-09-19)
//
// `config.get`, `env.setActive`, `workspace.setActive`, `entity.load` and `entity.setEnabled` were
// registered **at module scope in this file**. That made them invisible to a containerised engine —
// the implementation lived in a file only the Electron shell has, so a headless host answered
// `UNKNOWN_COMMAND` for commands the protocol declares and `@bifurc/client` advertises — and it made
// the registry-coverage ratchet blind, because in *this* process the shell had registered them.
//
// They now live in `packages/engine/src/store/coreCommands.ts` behind `registerCoreCommands()`, called
// from `src/ipc/handlers.ts` alongside the other `register*Commands` modules. Every body moved
// verbatim: the adapters below were already thin `commandRegistry.invoke()` delegations, so the wire
// behaviour is the same code reached through a different door.
//
// The suites that register this handler group **without** running `registerIpcHandlers()` must now
// call `registerCoreCommands(commandRegistry)` themselves — that is the cost of an explicit call site,
// and it is the point: a module side effect cannot be seen by a ratchet, an explicit call can.
const ctx = { bus };

export function registerCoreHandlers() {
  ipcMain.handle("config:get", () => commandRegistry.invoke("config.get", {}, ctx));

  ipcMain.handle("config:save", (_e, incoming: AppConfig) => {
    const prev = loadConfig();
    saveConfig(incoming);
    reloadConfig();
    bus.emitTyped("settings.changed", {});
    const tlsChanged = incoming.tlsEnabled !== prev.tlsEnabled
      || incoming.tlsCaCertPath !== prev.tlsCaCertPath
      || incoming.tlsCaKeyPath !== prev.tlsCaKeyPath;
    if (!isRunning() || incoming.port !== prev.port || tlsChanged) {
      stopServer();
      startServer(incoming.port);
    }
    if (incoming.companionPort !== prev.companionPort) {
      // Statically imported (see the top of this file) rather than `require`d here.
      // A bare `require("@bifurc/engine/transport/legacyCompanion")` only resolves because
      // `tsc-alias` post-processes the build output — which makes this user-facing
      // path depend on the build pipeline, and makes it impossible to exercise from
      // a test runner (the alias is not resolvable at runtime). There is no import
      // cycle to avoid: `legacyCompanion.ts` imports only `store/`, `sync/`, `proxy/`
      // and `eventBus` — never `src/ipc`.
      restartCompanionServer(incoming.companionPort);
    }
    return { ok: true };
  });

  ipcMain.handle("entity:load", (_e, wsId: string, kind: string, id: string) =>
    commandRegistry.invoke("entity.load", { workspaceId: wsId, kind: toProtocolKind(kind), id }, ctx));

  ipcMain.handle("entity:setEnabled", async (_e, wsId: string, kind: string, id: string, enabled: boolean) => {
    // Kept as an explicit guard, ahead of `commandRegistry.invoke()`, rather than folded into
    // the registered handler: `EntitySetEnabledParams.kind` is a strict 6-value enum, so an
    // invalid `kind` would otherwise fail Zod validation (and throw) instead of resolving with
    // the `{ ok: false, error: "invalid_kind" }` this channel has always returned.
    const enabledKinds = new Set(["mocks", "mappings", "rules", "graphqlMocks", "soapMocks", "grpcMocks"]);
    if (!enabledKinds.has(kind)) return { ok: false, error: "invalid_kind" };
    return commandRegistry.invoke("entity.setEnabled", { workspaceId: wsId, kind, id, enabled }, ctx);
  });

  ipcMain.handle("services:discover", () => discoverServices());

  // `environments` — the last `EntityKind` still holding out of the CRUD collapse (see the
  // "What's left" note in `plan/03-phase-2-engine-extraction.md` work item 7). It fits
  // `CrudFactoryOpts` exactly like `mappings` does (flat storage, no folders, no enabled
  // state) plus one thing none of the twelve original CRUD kinds needed: a create-gate check
  // (`gateKind`, consulted by `entity.create`'s registered handler) and two environment-only
  // delete-time quirks (the "__global__" guard and the active-environment reset), both now
  // handled inside `entityCrudFactory.ts` itself, keyed off `opts.kind === "environments"`.
  registerEntityCrudHandlers<Environment>({
    ipcPrefix: "env",
    kind: "environments",
    configKey: "environments",
    isFlat: true,
    gateKind: "environment",
  });

  ipcMain.handle("env:setActive", (_e, id: string | null) => commandRegistry.invoke("env.setActive", { id }, ctx));

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

  ipcMain.handle("workspace:setActive", (_e, id: string) => commandRegistry.invoke("workspace.setActive", { id }, ctx));

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
    emitEntityStatus(wsId);
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
