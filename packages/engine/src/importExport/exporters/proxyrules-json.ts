import { loadConfig, ProxyRule } from "../../store/config";
import { readAllEntities, readEnabledSet, bootstrapEnabledSet } from "../../store/workspaceFs";
import type { ExportResult } from "../types";

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    // Read the FULL rule files, NOT `cfg.proxyRules`.
    //
    // `cfg.proxyRules` holds UI stubs that deliberately blank out `targetType`,
    // `targetExternal`, `targetMappingId`, `useRegex` and both scripts (rules are
    // loaded on demand when a tab is opened). Exporting those stubs produced a file
    // that looked valid but, on re-import, replaced every rule with a target-less
    // `mapping` rule and destroyed its scripts — i.e. a silent data-loss bug in a
    // user-facing feature. Every other exporter of full entities (requests-curl,
    // websockets-json, webhooks-json) already reads from disk this way.
    //
    // Rule files strip the `enabled` flag (it lives in enabled.json), so it must be
    // re-injected here or the importer would land every rule disabled.
    const enabledSet = readEnabledSet(wsId, "rules") ?? bootstrapEnabledSet(wsId, "rules");
    const proxyRules = readAllEntities<ProxyRule>(wsId, "rules")
      .filter((r) => r.workspaceId === wsId)
      .map((r) => ({ ...r, enabled: enabledSet.has(r.id) }));
    const ws = cfg.workspaces.find((w) => w.id === wsId);
    const payload = { schema: "lp-proxy-rules-v1", name: ws?.name ?? "Bifurc", proxyRules };
    const content = JSON.stringify(payload, null, 2);
    return { ok: true, content, suggestedName: "proxy-rules-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
