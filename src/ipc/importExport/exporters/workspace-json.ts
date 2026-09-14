import * as fs from "fs";
import { loadConfig, SavedRequest, SavedWsConnection, SavedWebhook, ProxyRule } from "@/store/config";
import { readAllEntities, readEnabledSet, bootstrapEnabledSet } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const ws = (cfg.workspaces ?? []).find((w) => w.id === wsId);
    if (!ws) return { ok: false, error: "Workspace not found" };

    // Rules must come from the FULL entity files, not `cfg.proxyRules` — those are
    // UI stubs with targetType/targetExternal/scripts blanked out, so snapshotting
    // them would silently strip every rule's routing target. See the same fix in
    // exporters/proxyrules-json.ts. The `enabled` flag also has to be re-injected
    // because it lives in enabled.json, not in the entity file.
    const enabledRules = readEnabledSet(wsId, "rules") ?? bootstrapEnabledSet(wsId, "rules");
    const proxyRules = readAllEntities<ProxyRule>(wsId, "rules")
      .filter((r) => r.workspaceId === wsId)
      .map((r) => ({ ...r, enabled: enabledRules.has(r.id) }));

    const snapshot = {
      schema: "lp-workspace-v1",
      workspace: ws,
      data: {
        mappings:       cfg.mappings.filter((m) => m.workspaceId === wsId),
        proxyRules,
        mocks:          cfg.mocks.filter((m) => m.workspaceId === wsId),
        requests:       readAllEntities<SavedRequest>(wsId, "requests").filter((r) => r.workspaceId === wsId),
        mockFolders:    cfg.mockFolders.filter((f) => f.workspaceId === wsId),
        ruleFolders:    cfg.ruleFolders.filter((f) => f.workspaceId === wsId),
        requestFolders: cfg.requestFolders.filter((f) => f.workspaceId === wsId),
        wsConnections:  readAllEntities<SavedWsConnection>(wsId, "sockets").filter((c) => c.workspaceId === wsId),
        wsFolders:      cfg.wsFolders.filter((f) => f.workspaceId === wsId),
        webhooks:       readAllEntities<SavedWebhook>(wsId, "webhooks").filter((h) => h.workspaceId === wsId),
        webhookFolders: cfg.webhookFolders.filter((f) => f.workspaceId === wsId),
        environments:   cfg.environments.filter((e) => e.workspaceId === wsId),
      },
    };
    fs.writeFileSync(filePath, JSON.stringify(snapshot, null, 2), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
