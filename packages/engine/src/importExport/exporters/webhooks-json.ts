import { loadConfig, SavedWebhook } from "../../store/config";
import { readAllEntities } from "../../store/workspaceFs";
import type { ExportResult } from "../types";

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const webhooks = readAllEntities<SavedWebhook>(wsId, "webhooks").filter((h) => h.workspaceId === wsId);
    const folders = cfg.webhookFolders.filter((f) => f.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-webhooks-v1", name: ws?.name ?? "Bifurc", webhooks, folders };
    const content = JSON.stringify(payload, null, 2);
    return { ok: true, content, suggestedName: "webhooks-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
