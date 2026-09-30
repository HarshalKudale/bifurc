import * as fs from "fs";
import * as path from "path";
import { getGit, queryLog } from "../workspace/gitStore";
import {
  wsDir,
  readEntity,
  findEntityRelPath,
  upsertNameEntry,
  removeNameEntry,
  getPendingDeletions,
  removePendingDeletion,
  ALL_ENTITY_KINDS,
  FLAT_ENTITY_KINDS,
  EntityNameEntry,
} from "../workspace/workspaceFs";
import { getKindNameEntry } from "../workspace/kindNameEntries";
import { publishEntities, type PublishResult } from "./publishService";
import { withIndexLockRetry } from "./gitLock";
import {
  getWorkspaceSyncStatus,
  invalidateCache,
  EntitySyncStatus,
} from "./statusTracker";

export interface FileDiffResult {
  hasDiff: boolean;
  status: EntitySyncStatus;
  diff?: string;
  original?: string | null;
  current?: string | null;
}

/**
 * The name entry a kind wants recorded for an entity — the *display* half of a pending deletion.
 *
 * **Registration first, table second, and the order is the point.** Each kind already declares this
 * via `CrudFactoryOpts.getNameEntry` (`store/kinds/*.ts`), which is what `entityCrudCommands` writes
 * into `pending-deletions.json`. This function's table is a **second copy of the same mapping**, and
 * the two had already drifted: `grpcRequests` carries `methodName`/`serviceName` in its registration
 * and not here. Preferring the registration means there is one authority in the running app, and the
 * table survives only as the fallback for a caller that never registered any kinds — which is every
 * unit test that imports the engine's functions directly rather than going through `registerAll`.
 */
export function extractNameEntry(kind: string, entity: any): EntityNameEntry | null {
  const registered = getKindNameEntry(kind, entity);
  if (registered) return registered;
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
    case "soapRequests":
      return { name: entity.name ?? "", endpointUrl: entity.endpointUrl ?? "", soapAction: entity.soapAction ?? "" };
    case "graphqlRequests":
      return { name: entity.name ?? "", endpointUrl: entity.endpointUrl ?? "" };
    case "grpcRequests":
      return { name: entity.name ?? "" };
    case "mappings":
      return { name: entity.label || entity.domain || "", url: entity.domain ?? "" };
    case "environments":
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
      // A checkout writes the index, so it takes the same lock a publish does — retried for the same
      // reason. See `gitLock.ts`.
      await withIndexLockRetry(wsId, () => g.raw(["checkout", "HEAD", "--", normalized]));
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
            await withIndexLockRetry(wsId, () => g.raw(["checkout", "HEAD", "--", normalized]));
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
      // One shared kind list, not a local copy. The copy that used to live here named the same ten
      // kinds, which is exactly how the four copies of this vocabulary drifted apart — see
      // `ALL_ENTITY_KINDS`.
      for (const k of ALL_ENTITY_KINDS) {
        const pending = getPendingDeletions(wsId, k);
        if (pending.some((e) => e.id === entityId)) {
          removePendingDeletion(wsId, k, entityId);
        }
      }

      // Re-sync names.json with disk state — **foldered kinds only**. A flat kind has no names
      // index, so `upsertNameEntry` here would create a stray `{kind}/names.json` on every revert:
      // untracked, invisible in the entity list (it is filtered as a bookkeeping file), and then
      // silently committed by the next `git add -A -- {kind}/`. Reverting a mapping used to leave
      // one behind.
      if (!FLAT_ENTITY_KINDS.includes(kind)) {
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
    }

    invalidateCache(wsId);
    return { ok: true };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

/**
 * Commit and push changes for one or more files or folders.
 *
 * Returns `publishEntities`'s `committed` flag unchanged, so a caller can tell "there was nothing to
 * commit" from "a commit was made" — see `PublishResult`.
 */
export async function syncChanges(
  wsId: string,
  paths: string[],
  message?: string,
): Promise<PublishResult> {
  const normalizedPaths = paths.map((p) => p.replace(/\\/g, "/"));
  const result = await publishEntities({ wsId, paths: normalizedPaths, message });
  if (result.ok) {
    // A published deletion is no longer pending, whichever kind it belonged to. `loadConfig()`
    // rebuilds the deleted entity's stub from this record, so clearing it is what makes the entity
    // finally leave the UI — and leaving any kind out here means that kind's deletion can never be
    // completed. Shared list for the same reason as everywhere else; see `ALL_ENTITY_KINDS`.
    for (const p of normalizedPaths) {
      const parts = p.split("/");
      const base = parts[parts.length - 1];
      const entityId = base?.endsWith(".json") ? base.slice(0, -5) : null;
      if (entityId) {
        for (const kind of ALL_ENTITY_KINDS) {
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
