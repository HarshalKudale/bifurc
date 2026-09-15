import { ipcMain } from "electron";
import type { EntityCreateParams, EntityUpdateParams, EntityDeleteParams } from "@bifurc/protocol";
import { loadConfig, saveConfig, generateId, AppConfig } from "@/store/config";
import { 
  writeEntity, deleteEntityFile, writeFlatEntity, deleteFlatEntityFile,
  upsertNameEntry, removeNameEntry, 
  addPendingDeletion, findEntityRelPath,
  readEnabledSet, writeEnabledSet, bootstrapEnabledSet
} from "@/store/workspaceFs";
import { getGit } from "@/store/gitStore";
import { reloadConfig } from "@/proxy/server";
import { invalidateCache } from "@/sync/statusTracker";
import { bus, emitEntityStatus } from "@/eventBus";
import { commandRegistry } from "@/commands/registry";
import { toProtocolKind, toEngineKind } from "@/commands/entityKindMap";

const ctx = { bus };

/** The shape every CRUD-factory entity satisfies — pulled out so the extracted `*Core`
 * functions below can share it with `registerEntityCrudHandlers`'s own generic constraint. */
type EntityBase = { id: string; workspaceId?: string; folderId?: string | null; enabled?: boolean; createdAt?: number };

/** Every kind registered via `registerEntityCrudHandlers`, keyed by its engine-internal storage
 * `kind` string (e.g. `"rules"`, `"sockets"`) — populated as each `*Handlers.ts` file calls
 * `registerEntityCrudHandlers()` at startup. Looked up lazily (at invoke time, not at
 * registration time) by the generic `entity.create`/`entity.update`/`entity.delete` commands
 * below, so registration order across files does not matter. */
export const entityCrudRegistry = new Map<string, CrudFactoryOpts<any>>();

export async function isGitTracked(wsId: string, relPath: string): Promise<boolean> {
  try {
    const result = await getGit(wsId).raw(["ls-files", "--error-unmatch", "--", relPath]);
    return result.trim().length > 0;
  } catch {
    return false;
  }
}

export function syncEnabledSet(wsId: string, kind: string, id: string, enabled: boolean): void {
  let set = readEnabledSet(wsId, kind);
  if (!set) set = bootstrapEnabledSet(wsId, kind);
  if (enabled) set.add(id); else set.delete(id);
  writeEnabledSet(wsId, kind, set);
}

export interface CrudFactoryOpts<T> {
  kind: string;
  configKey: keyof AppConfig;
  folderConfigKey?: keyof AppConfig;
  ipcPrefix?: string;
  ipcAdd?: string;
  ipcUpdate?: string;
  ipcDelete?: string;
  isFlat?: boolean; // If true, uses writeFlatEntity/deleteFlatEntityFile instead of writeEntity/deleteEntityFile
  hasEnabledState?: boolean;
  validate?: (entity: Partial<T>) => void;
  onAddConflict?: (cfg: AppConfig, newEntity: T) => void;
  getNameEntry?: (entity: T) => any; // Return object for upsertNameEntry/addPendingDeletion
}

// ── Core logic, extracted so both the legacy per-kind `ipcMain.handle` channels and the
// collapsed `entity.create`/`entity.update`/`entity.delete` commands below share one
// implementation (P2 work item 7's "CRUD collapse" — see `registry.ts`'s own status note). Each
// function's body is byte-for-byte what used to live directly inside the corresponding
// `ipcMain.handle` callback; nothing here changes behaviour, only where the code lives.

async function createEntityCore<T extends EntityBase>(opts: CrudFactoryOpts<T>, entity: Omit<T, "id" | "createdAt">): Promise<T> {
  if (opts.validate) opts.validate(entity as Partial<T>);

  const cfg = loadConfig() as any;
  const wsId = (entity as any).workspaceId ?? cfg.activeWorkspaceId;

  const newEntity = { ...entity, id: generateId(), createdAt: Date.now(), workspaceId: wsId } as unknown as T;

  if (opts.hasEnabledState && newEntity.enabled === undefined) {
    newEntity.enabled = true;
  }

  cfg[opts.configKey] = cfg[opts.configKey] ?? [];

  if (opts.onAddConflict) {
    opts.onAddConflict(cfg, newEntity);
  }

  if (opts.ipcPrefix === "rule" || opts.ipcPrefix === "mock") {
    cfg[opts.configKey].unshift(newEntity);
  } else {
    cfg[opts.configKey].push(newEntity);
  }

  saveConfig(cfg);

  if (opts.isFlat) {
    writeFlatEntity(wsId, opts.kind, newEntity.id, newEntity);
  } else {
    let folderName = null;
    if (opts.folderConfigKey && newEntity.folderId) {
      const folders = cfg[opts.folderConfigKey] ?? [];
      folderName = folders.find((f: any) => f.id === newEntity.folderId)?.name ?? null;
    }
    writeEntity(wsId, opts.kind, newEntity.id, newEntity, folderName);
  }

  if (opts.hasEnabledState) {
    syncEnabledSet(wsId, opts.kind, newEntity.id, newEntity.enabled ?? false);
  }

  if (opts.getNameEntry) {
    upsertNameEntry(wsId, opts.kind, newEntity.id, opts.getNameEntry(newEntity));
  }

  // Reload LAST. `reloadConfig()` snapshots the enabled-sets from `enabled.json`, so
  // running it before `syncEnabledSet()` left the running proxy with a stale set —
  // `workspaceCfg()` then filtered the brand-new entity out of routing, and it only
  // started working after some unrelated action happened to reload the config.
  reloadConfig();

  emitEntityStatus(wsId);

  if (["mock", "request"].includes(opts.ipcPrefix ?? "")) {
    bus.emitTyped("entity.changed", { wsId, kind: opts.kind, id: newEntity.id, action: "created" });
  }

  return newEntity;
}

async function updateEntityCore<T extends EntityBase>(opts: CrudFactoryOpts<T>, entity: T): Promise<{ ok: boolean }> {
  const cfg = loadConfig() as any;
  const wsId = entity.workspaceId ?? cfg.activeWorkspaceId;

  cfg[opts.configKey] = cfg[opts.configKey] ?? [];
  const idx = cfg[opts.configKey].findIndex((e: any) => e.id === entity.id);
  if (idx !== -1) cfg[opts.configKey][idx] = entity;

  saveConfig(cfg);

  if (opts.isFlat) {
    writeFlatEntity(wsId, opts.kind, entity.id, entity);
  } else {
    let folderName = null;
    if (opts.folderConfigKey && entity.folderId) {
      const folders = cfg[opts.folderConfigKey] ?? [];
      folderName = folders.find((f: any) => f.id === entity.folderId)?.name ?? null;
    }
    writeEntity(wsId, opts.kind, entity.id, entity, folderName);
  }

  if (opts.getNameEntry) {
    upsertNameEntry(wsId, opts.kind, entity.id, opts.getNameEntry(entity));
  }

  // Reload LAST so the running proxy picks up the edited entity (see the note in
  // `createEntityCore`).
  reloadConfig();

  emitEntityStatus(wsId);
  return { ok: true };
}

async function deleteEntityCore<T extends EntityBase>(opts: CrudFactoryOpts<T>, id: string): Promise<{ ok: boolean }> {
  const cfg = loadConfig() as any;
  const wsId = cfg.activeWorkspaceId;

  const arr = cfg[opts.configKey] ?? [];
  const entity = arr.find((e: any) => e.id === id);

  if (entity) {
    cfg[opts.configKey] = arr.filter((e: any) => e.id !== id);

    if (opts.ipcPrefix === "mapping") {
      cfg.proxyRules = (cfg.proxyRules ?? []).filter((r: any) => (r.targetType ?? "mapping") !== "mapping" || r.targetMappingId !== id);
    }

    saveConfig(cfg);

    if (opts.isFlat) {
      deleteFlatEntityFile(wsId, opts.kind, id);
    } else {
      const relPath = findEntityRelPath(wsId, opts.kind, id);
      const tracked = relPath ? await isGitTracked(wsId, relPath) : false;

      deleteEntityFile(wsId, opts.kind, id);

      if (tracked && opts.getNameEntry) {
        const entry = opts.getNameEntry(entity);
        addPendingDeletion(wsId, opts.kind, { id, folderId: entity.folderId ?? null, ...entry });
      } else if (opts.getNameEntry) {
        removeNameEntry(wsId, opts.kind, id);
      }
    }

    if (opts.hasEnabledState) {
      syncEnabledSet(wsId, opts.kind, id, false);
    }

    // Reload LAST: `saveConfig()` does not unlink files for entities removed from the
    // config, so reloading before the file was deleted (and before the enabled-set was
    // updated) kept the deleted entity live in the proxy's routing tables.
    reloadConfig();

    invalidateCache(wsId);
    emitEntityStatus(wsId);
  }

  return { ok: true };
}

// ── The collapsed `entity.*` commands (P1 item 2 / P2 work item 7). Registered once, at module
// load, exactly like `coreHandlers.ts`'s `config.get`/`env.setActive` — they dispatch to
// whichever kind's `opts` is in `entityCrudRegistry` at invoke time, so it does not matter which
// `*Handlers.ts` file (and therefore which order) populated the registry first. A kind that
// hasn't called `registerEntityCrudHandlers()` yet (or never will — `environments`,
// `graphqlSchemas`, `protoFiles`, `wsdls` have their own bespoke add/delete handlers with no
// factory involvement) throws a clear error rather than silently no-oping.

commandRegistry.register("entity.create", async ({ kind, entity, workspaceId }: EntityCreateParams) => {
  const engineKind = toEngineKind(kind);
  const opts = entityCrudRegistry.get(engineKind);
  if (!opts) throw new Error(`entity.create: kind "${kind}" is not routed through the CommandRegistry yet`);
  const created = await createEntityCore(opts, { ...entity, workspaceId: workspaceId ?? (entity as any).workspaceId } as any);
  return { id: created.id, entity: created as Record<string, unknown> };
});

commandRegistry.register("entity.update", async ({ kind, entity }: EntityUpdateParams) => {
  const engineKind = toEngineKind(kind);
  const opts = entityCrudRegistry.get(engineKind);
  if (!opts) throw new Error(`entity.update: kind "${kind}" is not routed through the CommandRegistry yet`);
  return updateEntityCore(opts, entity as any);
});

commandRegistry.register("entity.delete", async ({ kind, id }: EntityDeleteParams) => {
  const engineKind = toEngineKind(kind);
  const opts = entityCrudRegistry.get(engineKind);
  if (!opts) throw new Error(`entity.delete: kind "${kind}" is not routed through the CommandRegistry yet`);
  return deleteEntityCore(opts, id);
});

export function registerEntityCrudHandlers<T extends EntityBase>(opts: CrudFactoryOpts<T>) {
  entityCrudRegistry.set(opts.kind, opts);
  const protocolKind = toProtocolKind(opts.kind);

  ipcMain.handle(opts.ipcAdd || `${opts.ipcPrefix}:add`, async (_e, entity: Omit<T, "id" | "createdAt">) => {
    // `entity.create` returns `{id, entity}` (the frozen protocol shape); the legacy channel
    // has always returned the raw created entity, so unwrap it here to keep the wire response
    // byte-identical.
    const result = await commandRegistry.invoke(
      "entity.create",
      { kind: protocolKind, entity, workspaceId: (entity as any).workspaceId },
      ctx,
    ) as { id: string; entity: Record<string, unknown> };
    return result.entity;
  });

  ipcMain.handle(opts.ipcUpdate || `${opts.ipcPrefix}:update`, async (_e, entity: T) => {
    return commandRegistry.invoke("entity.update", { kind: protocolKind, entity }, ctx);
  });

  ipcMain.handle(opts.ipcDelete || `${opts.ipcPrefix}:delete`, async (_e, id: string) => {
    return commandRegistry.invoke("entity.delete", { kind: protocolKind, id }, ctx);
  });
}
