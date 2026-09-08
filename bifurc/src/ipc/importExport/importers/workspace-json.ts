import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId,
  LocalMapping, ProxyRule, MockRule, SavedRequest, SavedWsConnection, SavedWebhook,
  Folder, Environment, Workspace,
} from "@/store/config";
import {
  writeEntity, writeFlatEntity, writeEnabledSet, bootstrapEnabledSet, readEnabledSet,
  upsertNameEntry, initWorkspaceDir,
} from "@/store/workspaceFs";
import { initWorkspaceRepo } from "@/store/gitStore";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const raw = fs.readFileSync(filePath, "utf-8");
    const snapshot = JSON.parse(raw);
    if (snapshot.schema !== "lp-workspace-v1" || !snapshot.workspace || !snapshot.data) {
      return { ok: false, error: "Not a valid lp-workspace-v1 file" };
    }
    const d = snapshot.data;
    const itemCount =
      (d.mocks?.length ?? 0) + (d.requests?.length ?? 0) + (d.mappings?.length ?? 0) +
      (d.proxyRules?.length ?? 0) + (d.wsConnections?.length ?? 0) +
      (d.environments?.length ?? 0) + (d.webhooks?.length ?? 0);
    // Workspace import always creates a new workspace — no collisions
    return { ok: true, filePath, itemCount, collisionIds: [] };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  _wsId: string,
  filePath: string,
  _strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const raw = fs.readFileSync(filePath, "utf-8");
    const snapshot = JSON.parse(raw);
    if (snapshot.schema !== "lp-workspace-v1" || !snapshot.workspace || !snapshot.data) {
      return { ok: false, error: "Not a valid lp-workspace-v1 file" };
    }

    const cfg = loadConfig();
    const newWsId = generateId();
    const srcWs: Workspace = snapshot.workspace;
    const existingNames = new Set((cfg.workspaces ?? []).map((w: Workspace) => w.name));
    let wsName = srcWs.name;
    if (existingNames.has(wsName)) {
      let n = 2;
      while (existingNames.has(`${srcWs.name} (${n})`)) n++;
      wsName = `${srcWs.name} (${n})`;
    }
    const newWs: Workspace = { id: newWsId, name: wsName, createdAt: Date.now(), activeEnvironmentId: null };
    cfg.workspaces = [...(cfg.workspaces ?? []), newWs];

    const d = snapshot.data;
    const remap = <T extends { workspaceId: string }>(arr: T[] | undefined): T[] =>
      (arr ?? []).map((item) => ({ ...item, workspaceId: newWsId }));

    cfg.mappings       = [...cfg.mappings,       ...remap<LocalMapping>(d.mappings)];
    cfg.proxyRules     = [...cfg.proxyRules,     ...remap<ProxyRule>(d.proxyRules)];
    cfg.mocks          = [...cfg.mocks,          ...remap<MockRule>(d.mocks)];
    cfg.mockFolders    = [...cfg.mockFolders,    ...remap<Folder>(d.mockFolders)];
    cfg.requestFolders = [...cfg.requestFolders, ...remap<Folder>(d.requestFolders)];
    cfg.wsFolders      = [...cfg.wsFolders,      ...remap<Folder>(d.wsFolders)];
    cfg.webhookFolders = [...cfg.webhookFolders, ...remap<Folder>(d.webhookFolders ?? [])];
    cfg.environments   = [...cfg.environments,   ...remap<Environment>(d.environments)];
    cfg.activeWorkspaceId = newWsId;
    cfg.activeEnvironmentId = null;

    // Init the new workspace directory and git repo
    initWorkspaceDir(newWsId, wsName);
    await initWorkspaceRepo(newWsId);

    saveConfig(cfg);

    const reqFolderMap = new Map(remap<Folder>(d.requestFolders).map((f) => [f.id, f.name]));
    const wsFolderMap  = new Map(remap<Folder>(d.wsFolders).map((f) => [f.id, f.name]));
    const hookFolderMap = new Map(remap<Folder>(d.webhookFolders ?? []).map((f) => [f.id, f.name]));

    for (const r of remap<SavedRequest>(d.requests ?? [])) {
      writeEntity(newWsId, "requests", r.id, r, r.folderId ? (reqFolderMap.get(r.folderId) ?? null) : null);
      upsertNameEntry(newWsId, "requests", r.id, { name: r.name, method: r.method, url: r.url });
    }
    for (const c of remap<SavedWsConnection>(d.wsConnections ?? [])) {
      writeEntity(newWsId, "sockets", c.id, c, c.folderId ? (wsFolderMap.get(c.folderId) ?? null) : null);
      upsertNameEntry(newWsId, "sockets", c.id, { name: c.name, url: c.url });
    }
    for (const h of remap<SavedWebhook>(d.webhooks ?? [])) {
      writeEntity(newWsId, "webhooks", h.id, h, h.folderId ? (hookFolderMap.get(h.folderId) ?? null) : null);
      upsertNameEntry(newWsId, "webhooks", h.id, { name: h.name, urlSuffix: h.urlSuffix });
    }
    // Persist mocks and mappings/rules enabled state
    for (const m of remap<MockRule>(d.mocks ?? [])) {
      const set = readEnabledSet(newWsId, "mocks") ?? bootstrapEnabledSet(newWsId, "mocks");
      if (m.enabled) set.add(m.id);
      writeEnabledSet(newWsId, "mocks", set);
      upsertNameEntry(newWsId, "mocks", m.id, { name: m.name, method: m.method, url: m.urlPattern });
    }

    reloadConfig();

    const imported =
      (d.mocks?.length ?? 0) + (d.requests?.length ?? 0) + (d.mappings?.length ?? 0) +
      (d.proxyRules?.length ?? 0) + (d.wsConnections?.length ?? 0) +
      (d.environments?.length ?? 0) + (d.webhooks?.length ?? 0);

    return { ok: true, imported };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
