import * as fs from "fs";
import { loadConfig, SavedRequest, SavedWsConnection, SavedWebhook } from "@/store/config";
import { readAllEntities } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const ws = (cfg.workspaces ?? []).find((w) => w.id === wsId);
    if (!ws) return { ok: false, error: "Workspace not found" };

    const snapshot = {
      schema: "lp-workspace-v1",
      workspace: ws,
      data: {
        mappings:       cfg.mappings.filter((m) => m.workspaceId === wsId),
        proxyRules:     cfg.proxyRules.filter((r) => r.workspaceId === wsId),
        mocks:          cfg.mocks.filter((m) => m.workspaceId === wsId),
        requests:       readAllEntities<SavedRequest>(wsId, "requests").filter((r) => r.workspaceId === wsId),
        mockFolders:    cfg.mockFolders.filter((f) => f.workspaceId === wsId),
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
