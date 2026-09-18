import {
  loadConfig, saveConfig, generateId,
  LocalMapping, ProxyRule, MockRule, SavedRequest, SavedWsConnection, SavedWebhook,
  Folder, Environment, Workspace,
} from "../../store/config";
import {
  writeEntity, writeFlatEntity, writeEnabledSet, bootstrapEnabledSet, readEnabledSet,
  upsertNameEntry, initWorkspaceDir,
} from "../../store/workspaceFs";
import { initWorkspaceRepo } from "../../store/gitStore";
import { reloadConfig } from "../../proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy, ImportSource } from "../types";

export function preflight(wsId: string, source: ImportSource): PreflightResult {
  try {
    const snapshot = JSON.parse(source.content);
    if (snapshot.schema !== "lp-workspace-v1" || !snapshot.workspace || !snapshot.data) {
      return { ok: false, error: "Not a valid lp-workspace-v1 file" };
    }
    const d = snapshot.data;
    const itemCount =
      (d.mocks?.length ?? 0) + (d.requests?.length ?? 0) + (d.mappings?.length ?? 0) +
      (d.proxyRules?.length ?? 0) + (d.wsConnections?.length ?? 0) +
      (d.environments?.length ?? 0) + (d.webhooks?.length ?? 0);
    // Workspace import always creates a new workspace — no collisions
    return { ok: true, itemCount, collisionIds: [] };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  _wsId: string,
  source: ImportSource,
  _strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const snapshot = JSON.parse(source.content);
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
    cfg.ruleFolders    = [...cfg.ruleFolders,    ...remap<Folder>(d.ruleFolders ?? [])];
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
    const ruleFolderMap = new Map(remap<Folder>(d.ruleFolders ?? []).map((f) => [f.id, f.name]));

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
    // Proxy rules need an EXPLICIT write: `saveConfig()` deliberately skips rule
    // files (it only ever holds the UI stubs), so without this loop every rule in
    // the snapshot would be dropped on disk and the imported workspace would proxy
    // nothing.
    for (const r of remap<ProxyRule>(d.proxyRules ?? [])) {
      writeEntity(newWsId, "rules", r.id, r, r.folderId ? (ruleFolderMap.get(r.folderId) ?? null) : null);
      const set = readEnabledSet(newWsId, "rules") ?? bootstrapEnabledSet(newWsId, "rules");
      if (r.enabled) set.add(r.id); else set.delete(r.id);
      writeEnabledSet(newWsId, "rules", set);
      upsertNameEntry(newWsId, "rules", r.id, { name: r.name, url: r.pattern });
    }
    // Persist mocks and mappings/rules enabled state.
    //
    // Each loop needs BOTH branches: `enabled.json` does not exist yet for a brand
    // new workspace, so `readEnabledSet` returns null and `bootstrapEnabledSet`
    // treats a missing `enabled` flag as ENABLED (entity files strip the flag). A
    // mock the user had deliberately disabled would therefore come back enabled and
    // start serving traffic again. The explicit `delete` is what prevents that.
    for (const m of remap<MockRule>(d.mocks ?? [])) {
      const set = readEnabledSet(newWsId, "mocks") ?? bootstrapEnabledSet(newWsId, "mocks");
      if (m.enabled) set.add(m.id); else set.delete(m.id);
      writeEnabledSet(newWsId, "mocks", set);
      upsertNameEntry(newWsId, "mocks", m.id, { name: m.name, method: m.method, url: m.urlPattern });
    }
    for (const m of remap<LocalMapping>(d.mappings ?? [])) {
      const set = readEnabledSet(newWsId, "mappings") ?? bootstrapEnabledSet(newWsId, "mappings");
      if (m.enabled) set.add(m.id); else set.delete(m.id);
      writeEnabledSet(newWsId, "mappings", set);
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
