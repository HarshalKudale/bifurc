import * as fs from "fs";
import * as path from "path";
import { getGit, queryLog } from "@/store/gitStore";
import {
  wsDir,
  readEntity,
  findEntityRelPath,
  upsertNameEntry,
  removeNameEntry,
  getPendingDeletions,
  removePendingDeletion,
  EntityNameEntry,
} from "@/store/workspaceFs";
import { publishEntities } from "@/sync/publishService";
import {
  getWorkspaceSyncStatus,
  invalidateCache,
  EntitySyncStatus,
} from "@/sync/statusTracker";

export interface FileDiffResult {
  hasDiff: boolean;
  status: EntitySyncStatus;
  diff?: string;
  original?: string | null;
  current?: string | null;
}

export function extractNameEntry(kind: string, entity: any): EntityNameEntry | null {
  if (!entity || typeof entity !== "object") return null;
  switch (kind) {
    case "rules":
      return { name: entity.name ?? "", url: entity.pattern ?? "" };
    case "mocks":
      return { name: entity.name ?? "", method: entity.method ?? "GET", url: entity.urlPattern ?? "" };
    case "requests":
      return { name: entity.name ?? "", method: entity.method ?? "GET", url: entity.url ?? "" };
    case "sockets":
      return { name: entity.name ?? "", url: entity.url ?? "" };
    case "webhooks":
      return { name: entity.name ?? "", urlSuffix: entity.urlSuffix ?? "" };
    case "soapRequests":
      return { name: entity.name ?? "", endpointUrl: entity.endpointUrl ?? "", soapAction: entity.soapAction ?? "" };
    case "soapMocks":
      return { name: entity.name ?? "", soapActionPattern: entity.soapActionPattern ?? "" };
    case "graphqlRequests":
      return { name: entity.name ?? "", endpointUrl: entity.endpointUrl ?? "" };
    case "graphqlMocks":
      return { name: entity.name ?? "", operationName: entity.operationName ?? "" };
    case "grpcRequests":
    case "grpcMocks":
      return { name: entity.name ?? "" };
    default:
      return entity.name ? { name: entity.name } : null;
  }
}

/**
 * Get git diff and status for a specific file relative to HEAD.
 */
export async function getFileDiff(wsId: string, relPath: string): Promise<FileDiffResult> {
  const normalized = relPath.replace(/\\/g, "/");
  const fullStatus = await getWorkspaceSyncStatus(wsId);
  const status: EntitySyncStatus = fullStatus[normalized] ?? "clean";

  if (status === "clean") {
    return { hasDiff: false, status: "clean", diff: "" };
  }

  const g = getGit(wsId);
  try {
    if (status === "modified") {
      const diff = await g.raw(["diff", "HEAD", "--", normalized]);
      return { hasDiff: true, status: "modified", diff };
    }

    if (status === "new") {
      const fullPath = path.join(wsDir(wsId), normalized);
      const current = fs.existsSync(fullPath) ? fs.readFileSync(fullPath, "utf-8") : null;
      return { hasDiff: true, status: "new", current };
    }

    if (status === "deleted") {
      return { hasDiff: true, status: "deleted" };
    }
  } catch {
    // Fallback to basic status
  }

  return { hasDiff: true, status };
}

/**
 * Discard uncommitted changes for a file.
 * Restores the file from git HEAD, or deletes it if untracked.
 * Also synchronizes names.json, cleans up pending deletions, and invalidates sync cache.
 */
export async function discardChanges(
  wsId: string,
  relPath: string,
): Promise<{ ok: boolean; error?: string }> {
  try {
    const g = getGit(wsId);
    let normalized = relPath.replace(/\\/g, "/");
    const parts = normalized.split("/");
    const kind = parts[0];
    const base = parts[parts.length - 1];
    const entityId = base?.endsWith(".json") ? base.slice(0, -5) : null;

    let checkoutSucceeded = false;

    // 1. Try checking out from HEAD
    try {
      await g.raw(["checkout", "HEAD", "--", normalized]);
      checkoutSucceeded = true;
    } catch {
      // Maybe git tracks it under a different subfolder path?
      if (base && base.endsWith(".json")) {
        try {
          const trackedFiles = (await g.raw(["ls-files", `*${base}`]))
            .trim()
            .split(/\r?\n/)
            .filter(Boolean);
          if (trackedFiles.length > 0) {
            normalized = trackedFiles[0].replace(/\\/g, "/");
            await g.raw(["checkout", "HEAD", "--", normalized]);
            checkoutSucceeded = true;
          }
        } catch {
          // ignore ls-files error
        }
      }
    }

    // 2. If checkout did not succeed, check if the file is untracked
    if (!checkoutSucceeded) {
      let fullPath = path.join(wsDir(wsId), normalized);
      if (!fs.existsSync(fullPath) && entityId && kind) {
        const diskRel = findEntityRelPath(wsId, kind, entityId);
        if (diskRel) {
          fullPath = path.join(wsDir(wsId), diskRel);
        }
      }

      if (fs.existsSync(fullPath)) {
        fs.unlinkSync(fullPath);
      }
    }

    // 3. Post-discard cleanup: pending deletions & names.json
    if (entityId) {
      const allKinds = [
        "requests",
        "mocks",
        "sockets",
        "webhooks",
        "rules",
        "mappings",
        "graphqlRequests",
        "graphqlMocks",
        "soapRequests",
        "soapMocks",
        "grpcRequests",
        "grpcMocks",
        "environments",
      ];
      for (const k of allKinds) {
        const pending = getPendingDeletions(wsId, k);
        if (pending.some((e) => e.id === entityId)) {
          removePendingDeletion(wsId, k, entityId);
        }
      }

      // Re-sync names.json with disk state
      const restoredEntity = readEntity<any>(wsId, kind, entityId);
      if (restoredEntity) {
        const nameEntry = extractNameEntry(kind, restoredEntity);
        if (nameEntry) {
          upsertNameEntry(wsId, kind, entityId, nameEntry);
        }
      } else {
        // File is deleted
        removeNameEntry(wsId, kind, entityId);
      }
    }

    invalidateCache(wsId);
    return { ok: true };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

/**
 * Commit and push changes for one or more files or folders.
 */
export async function syncChanges(
  wsId: string,
  paths: string[],
  message?: string,
): Promise<{ ok: boolean; error?: string }> {
  const normalizedPaths = paths.map((p) => p.replace(/\\/g, "/"));
  const result = await publishEntities({ wsId, paths: normalizedPaths, message });
  if (result.ok) {
    for (const p of normalizedPaths) {
      const parts = p.split("/");
      const base = parts[parts.length - 1];
      const entityId = base?.endsWith(".json") ? base.slice(0, -5) : null;
      if (entityId) {
        for (const kind of [
          "requests", "mocks", "sockets", "webhooks", "rules", "mappings",
          "graphqlRequests", "graphqlMocks", "soapRequests", "soapMocks",
          "grpcRequests", "grpcMocks", "environments",
        ]) {
          removePendingDeletion(wsId, kind, entityId);
        }
      }
    }
  }
  invalidateCache(wsId);
  return result;
}

/**
 * Get git history for a specific file.
 */
export async function getFileHistory(
  wsId: string,
  relPath: string,
  opts?: { limit?: number; offset?: number },
) {
  const normalized = relPath.replace(/\\/g, "/");
  return queryLog({
    workspaceId: wsId,
    filePath: normalized,
    limit: opts?.limit ?? 100,
    offset: opts?.offset ?? 0,
  });
}
