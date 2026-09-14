import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, MockRule,
} from "@/store/config";
import {
  writeEntity, upsertNameEntry, readEnabledSet, writeEnabledSet, bootstrapEnabledSet, readAllEntities,
} from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

function syncEnabled(wsId: string, id: string, enabled: boolean): void {
  let set = readEnabledSet(wsId, "mocks");
  if (!set) set = bootstrapEnabledSet(wsId, "mocks");
  if (enabled) set.add(id); else set.delete(id);
  writeEnabledSet(wsId, "mocks", set);
}

interface WireMockStub {
  id?: string;
  name?: string;
  request?: {
    method?: string;
    url?: string;
    urlPattern?: string;
    urlPath?: string;
  };
  response?: {
    status?: number;
    headers?: Record<string, string>;
    body?: string;
    fixedDelayMilliseconds?: number;
  };
}

export function preflight(wsId: string, filePath: string): PreflightResult {
  try {
    const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));
    const stubs: WireMockStub[] = Array.isArray(data) ? data : (data.mappings ?? []);
    const existingIds = new Set(readAllEntities<MockRule>(wsId, "mocks").map((m) => m.id));
    const collisionIds = stubs.filter((s) => s.id && existingIds.has(s.id)).map((s) => s.id!);
    return { ok: true, filePath, itemCount: stubs.length, collisionIds };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  filePath: string,
  strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));
    const stubs: WireMockStub[] = Array.isArray(data) ? data : (data.mappings ?? []);
    const cfg = loadConfig();
    const existingIds = new Set(cfg.mocks.filter((m) => m.workspaceId === wsId).map((m) => m.id));

    let imported = 0;
    let skipped = 0;

    for (const stub of stubs) {
      const srcId = stub.id ?? generateId();
      const isCollision = existingIds.has(srcId);
      if (isCollision && strategy === "keep") { skipped++; continue; }

      const newId = strategy === "new" ? generateId() : srcId;
      const req = stub.request ?? {};
      const res = stub.response ?? {};
      const useRegex = !!(req.urlPattern);
      const urlPattern = req.urlPattern ?? req.url ?? req.urlPath ?? "/";
      const method = (req.method ?? "ANY") === "ANY" ? "*" : (req.method ?? "*").toUpperCase();

      const newMock: MockRule = {
        id: newId,
        name: stub.name ?? `${method} ${urlPattern}`,
        method,
        urlPattern,
        useRegex,
        enabled: false,
        capturedHeaders: {},
        capturedBody: "",
        responseStatus: res.status ?? 200,
        responseHeaders: res.headers ?? {},
        responseBody: res.body ?? "",
        responseDelay: res.fixedDelayMilliseconds,
        createdAt: Date.now(),
        folderId: null,
        workspaceId: wsId,
      };

      const existIdx = cfg.mocks.findIndex((x) => x.id === newId);
      if (existIdx !== -1) cfg.mocks[existIdx] = newMock;
      else cfg.mocks = [...cfg.mocks, newMock];

      writeEntity(wsId, "mocks", newId, newMock, null);
      syncEnabled(wsId, newId, false);
      upsertNameEntry(wsId, "mocks", newId, { name: newMock.name, method: newMock.method, url: newMock.urlPattern });
      imported++;
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported, skipped };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
