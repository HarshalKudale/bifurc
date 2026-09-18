import { ipcMain } from "electron";
import type { EntityCreateParams, EntityUpdateParams, EntityDeleteParams, EntityListParams } from "@bifurc/protocol";
import { loadConfig, saveConfig, generateId, AppConfig } from "@bifurc/engine/store/config";
import { 
  writeEntity, deleteEntityFile, writeFlatEntity, deleteFlatEntityFile,
  upsertNameEntry, removeNameEntry, 
  addPendingDeletion, findEntityRelPath,
  readEnabledSet, writeEnabledSet, bootstrapEnabledSet,
  readAllEntities,
} from "@bifurc/engine/store/workspaceFs";
import { getGit } from "@bifurc/engine/store/gitStore";
import { reloadConfig } from "@bifurc/engine/proxy/server";
import { invalidateCache } from "@bifurc/engine/sync/statusTracker";
import { bus, emitEntityStatus } from "@bifurc/engine/eventBus";
import { commandRegistry, mayAffectUnnamedEntities, type CommandContext } from "@bifurc/engine/commands/registry";
import { toProtocolKind, toEngineKind } from "@bifurc/engine/commands/entityKindMap";
import { gateCreate } from "@bifurc/engine/subscription/entityCount";

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

/** The three kinds that don't fit `CrudFactoryOpts` at all — `graphqlSchemas`, `protoFiles`,
 * `wsdls` are written straight to disk with `writeEntity(wsId, kind, id, data, null)`, nothing
 * mirrors them into an `AppConfig` array, and there is no "update" concept for any of them
 * (only add/delete/list). Registered via `registerSimpleEntityHandlers()` below, and consulted
 * as a fallback by `entity.create`/`entity.delete`/`entity.list` when a kind isn't in
 * `entityCrudRegistry`. */
export const simpleEntityKinds = new Set<string>();

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
  /** The friendly kind name `gateCreate()` (`@/subscription/entityCount`) expects — e.g.
   * `"environment"`. Only `environments` sets this today (its pre-existing `env:add` handler
   * was the one CRUD-ish kind with a create-gate check); every other kind is ungated. Checked
   * by `entity.create`'s registered command handler, ahead of `createEntityCore()`, so a
   * blocked create never touches config/disk — see that handler for the shared shape. */
  gateKind?: string;
}

// ── Core logic, extracted so both the legacy per-kind `ipcMain.handle` channels and the
// collapsed `entity.create`/`entity.update`/`entity.delete` commands below share one
// implementation (P2 work item 7's "CRUD collapse" — see `registry.ts`'s own status note). Each
// function's body is byte-for-byte what used to live directly inside the corresponding
// `ipcMain.handle` callback; nothing here changes behaviour, only where the code lives.

/**
 * `mayResolveConflicts` is **required** rather than defaulted, so a new caller has to decide instead of
 * inheriting the privileged path. It is `mayAffectUnnamedEntities(ctx)` at the one call site below —
 * see that predicate for why conflict resolution is not a plain `write`.
 */
async function createEntityCore<T extends EntityBase>(opts: CrudFactoryOpts<T>, entity: Omit<T, "id" | "createdAt">, mayResolveConflicts: boolean): Promise<T> {
  if (opts.validate) opts.validate(entity as Partial<T>);

  const cfg = loadConfig() as any;
  const wsId = (entity as any).workspaceId ?? cfg.activeWorkspaceId;

  const newEntity = { ...entity, id: generateId(), createdAt: Date.now(), workspaceId: wsId } as unknown as T;

  if (opts.hasEnabledState && newEntity.enabled === undefined) {
    newEntity.enabled = true;
  }

  cfg[opts.configKey] = cfg[opts.configKey] ?? [];

  // Skipped entirely for a caller that may not touch entities it did not name — the created entity is
  // still stored, so this narrows the *side effect* and not the command. Only `mocks` and `rules` set
  // `onAddConflict`, so for every other kind the flag changes nothing.
  if (opts.onAddConflict && mayResolveConflicts) {
    // `onAddConflict` disables siblings by mutating `cfg`, but **`saveConfig()` cannot persist that**:
    // `writeEntity()` strips `enabled` on the way out, because enabled state lives exclusively in
    // `enabled.json` (see `workspaceFs.ts`). So before this diff existed the mutation was silently
    // lost — the sibling stayed enabled on disk and would have switched itself back on the moment the
    // new entity was deleted. `entity.setEnabled`'s equivalent loop has always written through
    // `syncEnabledSet`, which is why the two sites disagreed about the same rule. This is that same
    // write, derived from a diff so the per-kind `onAddConflict` lambdas stay as simple as they are.
    const enabledBefore = new Map<string, unknown>(
      (cfg[opts.configKey] ?? []).map((e: any) => [e.id, e.enabled]),
    );
    opts.onAddConflict(cfg, newEntity);
    if (opts.hasEnabledState) {
      for (const e of (cfg[opts.configKey] ?? []) as any[]) {
        if (enabledBefore.has(e.id) && enabledBefore.get(e.id) !== e.enabled) {
          syncEnabledSet(wsId, opts.kind, e.id, !!e.enabled);
        }
      }
    }
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

async function deleteEntityCore<T extends EntityBase>(opts: CrudFactoryOpts<T>, id: string): Promise<{ ok: boolean; error?: string }> {
  // `environments`' one delete-time guard: the synthetic "__global__" environment
  // (bootstrapped in `store/config.ts`) can never be deleted. Checked ahead of everything
  // else, byte-for-byte the same early-return `env:delete` had before the CRUD collapse.
  if (opts.kind === "environments" && id === "__global__") {
    return { ok: false, error: "cannot_delete_global" };
  }

  const cfg = loadConfig() as any;
  const wsId = cfg.activeWorkspaceId;

  const arr = cfg[opts.configKey] ?? [];
  const entity = arr.find((e: any) => e.id === id);

  if (entity) {
    cfg[opts.configKey] = arr.filter((e: any) => e.id !== id);

    if (opts.ipcPrefix === "mapping") {
      cfg.proxyRules = (cfg.proxyRules ?? []).filter((r: any) => (r.targetType ?? "mapping") !== "mapping" || r.targetMappingId !== id);
    }

    // `environments`' other delete-time quirk: clear the active-environment pointer if the
    // environment being deleted was active, so nothing keeps resolving variables against an
    // id that no longer exists.
    if (opts.kind === "environments" && cfg.activeEnvironmentId === id) {
      cfg.activeEnvironmentId = null;
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

// ── "Simple" entities — `graphqlSchemas`/`protoFiles`/`wsdls`. No `AppConfig` array, no update,
// no folders, no enabled-state: just `writeEntity`/`deleteEntityFile`/`readAllEntities` against
// the workspace's flat entity store. Byte-for-byte what used to live directly inside
// `graphql:addSchema`/`grpc:addProto`/`soap:addWsdl` and their `delete`/`list` siblings.

type SimpleEntityBase = { id: string; workspaceId?: string; createdAt?: number };

async function createSimpleEntityCore<T extends SimpleEntityBase>(kind: string, entity: Omit<T, "id" | "createdAt">): Promise<T> {
  const cfg = loadConfig();
  const wsId = (entity as any).workspaceId ?? cfg.activeWorkspaceId;
  const newEntity = { ...entity, id: generateId(), createdAt: Date.now(), workspaceId: wsId } as unknown as T;
  writeEntity(wsId, kind, newEntity.id, newEntity, null);
  return newEntity;
}

async function deleteSimpleEntityCore(kind: string, id: string): Promise<{ ok: boolean }> {
  const cfg = loadConfig();
  deleteEntityFile(cfg.activeWorkspaceId, kind, id);
  return { ok: true };
}

function listSimpleEntitiesCore<T>(wsId: string, kind: string): T[] {
  return readAllEntities<T>(wsId, kind);
}

// ── The collapsed `entity.*` commands (P1 item 2 / P2 work item 7). Registered once, at module
// load, exactly like `coreHandlers.ts`'s `config.get`/`env.setActive` — they dispatch to
// whichever kind's `opts` is in `entityCrudRegistry` at invoke time, so it does not matter which
// `*Handlers.ts` file (and therefore which order) populated the registry first. A kind in
// neither `entityCrudRegistry` nor `simpleEntityKinds` throws a clear error rather than
// silently no-oping — today every `EntityKind` value is registered in one or the other.

commandRegistry.register("entity.create", async ({ kind, entity, workspaceId }: EntityCreateParams, ctx: CommandContext) => {
  const engineKind = toEngineKind(kind);
  const mergedEntity = { ...entity, workspaceId: workspaceId ?? (entity as any).workspaceId };
  const opts = entityCrudRegistry.get(engineKind);
  if (opts) {
    // Only `environments` sets `gateKind` today (see `CrudFactoryOpts.gateKind`'s own note).
    // Checked here, ahead of `createEntityCore()`, so a blocked create never touches
    // config/disk — matches `env:add`'s pre-collapse `{error: "limit_reached", ...gate}`
    // return exactly. `EntityCreateResult`'s frozen `{id, entity}` shape has no room for this,
    // but the registry (`registry.ts`) only validates *params*, not return values, so this is
    // safe: it is the same trade-off `entity:setEnabled`'s `invalid_kind` guard already made.
    if (opts.gateKind) {
      const wsId = (mergedEntity as any).workspaceId ?? loadConfig().activeWorkspaceId;
      const gate = gateCreate(wsId, opts.gateKind);
      if (!gate.allowed) return { error: "limit_reached", ...gate };
    }
    const created = await createEntityCore(opts, mergedEntity as any, mayAffectUnnamedEntities(ctx));
    return { id: created.id, entity: created as Record<string, unknown> };
  }
  if (simpleEntityKinds.has(engineKind)) {
    const created = await createSimpleEntityCore(engineKind, mergedEntity as any);
    return { id: created.id, entity: created as Record<string, unknown> };
  }
  throw new Error(`entity.create: kind "${kind}" is not routed through the CommandRegistry yet`);
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
  if (opts) return deleteEntityCore(opts, id);
  if (simpleEntityKinds.has(engineKind)) return deleteSimpleEntityCore(engineKind, id);
  throw new Error(`entity.delete: kind "${kind}" is not routed through the CommandRegistry yet`);
});

commandRegistry.register("entity.list", ({ workspaceId, kind }: EntityListParams) => {
  const engineKind = toEngineKind(kind);
  if (!simpleEntityKinds.has(engineKind)) {
    throw new Error(`entity.list: kind "${kind}" is not routed through the CommandRegistry yet`);
  }
  return { entities: listSimpleEntitiesCore<Record<string, unknown>>(workspaceId, engineKind) };
});

export function registerEntityCrudHandlers<T extends EntityBase>(opts: CrudFactoryOpts<T>) {
  entityCrudRegistry.set(opts.kind, opts);
  const protocolKind = toProtocolKind(opts.kind);

  ipcMain.handle(opts.ipcAdd || `${opts.ipcPrefix}:add`, async (_e, entity: Omit<T, "id" | "createdAt">) => {
    // `entity.create` returns `{id, entity}` (the frozen protocol shape); the legacy channel
    // has always returned the raw created entity, so unwrap it here to keep the wire response
    // byte-identical. A gate-blocked create (only `environments` sets `gateKind`) returns
    // `{error, ...gate}` instead — no `.entity` to unwrap — so that shape passes through
    // unchanged, matching `env:add`'s pre-collapse `{error: "limit_reached", ...gate}` return.
    const result = await commandRegistry.invoke(
      "entity.create",
      { kind: protocolKind, entity, workspaceId: (entity as any).workspaceId },
      ctx,
    ) as { id: string; entity: Record<string, unknown> } | { error: string };
    return "entity" in result ? result.entity : result;
  });

  ipcMain.handle(opts.ipcUpdate || `${opts.ipcPrefix}:update`, async (_e, entity: T) => {
    return commandRegistry.invoke("entity.update", { kind: protocolKind, entity }, ctx);
  });

  ipcMain.handle(opts.ipcDelete || `${opts.ipcPrefix}:delete`, async (_e, id: string) => {
    return commandRegistry.invoke("entity.delete", { kind: protocolKind, id }, ctx);
  });
}

export interface SimpleEntityOpts {
  /** Engine-internal storage kind, e.g. `"graphqlSchemas"`. */
  kind: string;
  ipcAdd: string;
  ipcDelete: string;
  ipcList: string;
}

/** Registers the three no-`AppConfig`-array, no-update kinds (`graphqlSchemas`, `protoFiles`,
 * `wsdls`) the same way `registerEntityCrudHandlers` registers the twelve full-CRUD kinds: the
 * legacy per-kind channels become thin adapters over `commandRegistry.invoke()`. */
export function registerSimpleEntityHandlers<T extends SimpleEntityBase>(opts: SimpleEntityOpts) {
  simpleEntityKinds.add(opts.kind);
  const protocolKind = toProtocolKind(opts.kind);

  ipcMain.handle(opts.ipcAdd, async (_e, entity: Omit<T, "id" | "createdAt">) => {
    const result = await commandRegistry.invoke(
      "entity.create",
      { kind: protocolKind, entity, workspaceId: (entity as any).workspaceId },
      ctx,
    ) as { id: string; entity: Record<string, unknown> };
    return result.entity;
  });

  ipcMain.handle(opts.ipcDelete, async (_e, id: string) => {
    return commandRegistry.invoke("entity.delete", { kind: protocolKind, id }, ctx);
  });

  ipcMain.handle(opts.ipcList, async () => {
    const wsId = loadConfig().activeWorkspaceId;
    const result = await commandRegistry.invoke(
      "entity.list",
      { workspaceId: wsId, kind: protocolKind },
      ctx,
    ) as { entities: Record<string, unknown>[] };
    return result.entities;
  });
}
