/**
 * The four collapsed `entity.*` commands (`create` / `update` / `delete` / `list`), moved out of
 * `src/ipc/handlers/entityCrudFactory.ts` (P6 step 3e, slice 7 — the last slice of the §5c pass).
 *
 * ## Why they moved (§5c)
 *
 * They were registered at **module scope** in the shell file, so a containerised engine (P9) answered
 * `UNKNOWN_COMMAND` for all four.
 *
 * ## Why this slice is structurally different from slices 1–6, and harder
 *
 * Every earlier slice was a *file of commands*: the shell file registered some commands and exposed
 * some channels, and the two halves could be separated by moving the registrations and leaving the
 * channels behind. This file is not that. It is a **factory split across the seam**:
 *
 *  - `registerEntityCrudHandlers(opts)` / `registerSimpleEntityHandlers(opts)` are called from **six
 *    shell handler files** (`coreHandlers`, `crudHandlers`, `graphqlHandlers`, `grpcHandlers`,
 *    `soapHandlers`, and one more kind each way). They do two unrelated things: they insert the kind's
 *    options into a module-level container, **and** they install three `ipcMain.handle` channels.
 *  - The four commands read that container **at invoke time** and dispatch to whichever kind's options
 *    are in it.
 *
 * So the commands depend on state the *shell* populates, and the dependency runs the wrong way. Moving
 * the four registrations without moving the containers would have produced four commands consulting an
 * **empty** map — every one of them throwing `"<kind>" is not routed through the CommandRegistry yet`,
 * which is the same error they throw today for a genuinely unregistered kind. That is a silent-failure
 * shape: the message would have been plausible, and the cause would have been the move itself.
 *
 * The resolution is to invert the population direction: this module owns the containers, and the shell
 * files reach them through `registerCrudKind()` / `registerSimpleKind()` — the **engine half** of the
 * two factory functions. `registerEntityCrudHandlers` / `registerSimpleEntityHandlers` stay in the
 * shell (they own `ipcMain`), call into the engine, and keep their signatures **byte-identical**, so
 * **none of the six caller files needed an edit**. That is the property to check first when a factory
 * — rather than a command list — is what has to move.
 *
 * ## The two containers are now private, and that is unobservable
 *
 * `entityCrudRegistry` and `simpleEntityKinds` were `export`ed in the shell file, but a grep of `src/`,
 * `tests/` and `packages/` found **no consumer outside the file** — only the factory's own commands and
 * registration functions touched them. They are module-private here, with `registerCrudKind()` /
 * `registerSimpleKind()` as the entry points. Same for the six `*Core` functions and `isGitTracked`,
 * which were already private. Worth stating because it is the one thing in this slice that is not a
 * pure relocation: it is an encapsulation change with no observable effect, verified by grep rather
 * than assumed.
 *
 * `isGitTracked` also has a **dead duplicate** in `src/ipc/handlers/utils.ts`, a file that is already
 * unreferenced and dies with `registerIpcHandlers()`. Left alone: deleting it is not part of a §5c
 * move, and `git rm` on `src/` is correlated with the step-3 wipe.
 *
 * ## Behaviours preserved verbatim, several of which a clean-up would flatten
 *
 *  - **`gateCreate()` runs ahead of `createEntityCore()`**, so a blocked create never touches config or
 *    disk. Only `environments` sets `gateKind`, and it returns `{error: "limit_reached", ...gate}`
 *    exactly as the pre-collapse `env:add` did. `EntityCreateResult`'s frozen `{id, entity}` shape has
 *    no room for that, which is safe only because the registry validates **params**, not return values
 *    — the same trade-off `entity.setEnabled`'s `invalid_kind` guard already makes.
 *  - **`onAddConflict` disables siblings by mutating the config, and `saveConfig()` cannot persist
 *    that**, because `writeEntity()` strips `enabled` on the way out — enabled state lives exclusively
 *    in `enabled.json`. So the mutation is **diffed** and written through `syncEnabledSet`. Before the
 *    diff existed the change was silently lost: the sibling stayed enabled on disk and switched itself
 *    back on the moment the new entity was deleted. `entity.setEnabled`'s equivalent loop always wrote
 *    through `syncEnabledSet`, which is why the two sites used to disagree about the same rule. Only
 *    `mocks` and `rules` set `onAddConflict`, so for every other kind the flag changes nothing.
 *  - **`mayResolveConflicts` is a required parameter, not a default**, so a new caller has to decide
 *    rather than inherit the privileged path. It is `mayAffectUnnamedEntities(ctx)` at the call site.
 *  - **`reloadConfig()` runs LAST in all three writers**, and the reason is different each time.
 *    On create: `reloadConfig()` snapshots the enabled-sets from `enabled.json`, so running it before
 *    `syncEnabledSet()` left the running proxy with a stale set — `workspaceCfg()` then filtered the
 *    brand-new entity out of routing, and it only started working after some unrelated action happened
 *    to reload. On delete: `saveConfig()` does not unlink files for entities removed from the config,
 *    so reloading before the file was deleted (and before the enabled-set was updated) kept the deleted
 *    entity live in the proxy's routing tables.
 *  - **`invalidateCache()` is called on delete and not on create or update** — the sync status cache is
 *    keyed by workspace, and only a removal can make a previously-reported entity stale.
 *  - **`"__global__"` cannot be deleted** (`environments` only), checked ahead of everything else.
 *  - **Delete branches on git-tracking.** A tracked entity with a name entry becomes a
 *    `addPendingDeletion` — the deletion has to be *published*, so the name entry has to survive until
 *    the commit lands. An untracked one is `removeNameEntry` immediately. Getting this backwards makes
 *    a published deletion either unreplayable or resurrectable.
 *  - **`entity.list` only serves simple kinds and `entity.update` has no simple path at all.**
 *    `graphqlSchemas` / `protoFiles` / `wsdls` are written straight to disk, nothing mirrors them into
 *    an `AppConfig` array, and there is no "update" concept for any of them. Both asymmetries are the
 *    shell's and are preserved.
 *  - The three `"<kind>" is not routed through the CommandRegistry yet` messages are verbatim, including
 *    the `entity.create` / `entity.update` / `entity.delete` / `entity.list` prefixes that distinguish
 *    them.
 *
 * None of the four takes a `CommandContext` except `entity.create` — which needs it for
 * `mayAffectUnnamedEntities`.
 */
import type {
  EntityCreateParams, EntityUpdateParams, EntityDeleteParams, EntityListParams,
} from "@bifurc/protocol";
import { loadConfig, saveConfig, generateId, AppConfig } from "./config";
import {
  writeEntity, deleteEntityFile, writeFlatEntity, deleteFlatEntityFile,
  upsertNameEntry, removeNameEntry,
  addPendingDeletion, findEntityRelPath,
  readAllEntities,
} from "./workspaceFs";
import { syncEnabledSet } from "./enabledSet";
import { getGit } from "./gitStore";
import { reloadConfig } from "../proxy/server";
import { invalidateCache } from "../sync/statusTracker";
import { bus, emitEntityStatus } from "../eventBus";
import { mayAffectUnnamedEntities, type CommandContext, type CommandRegistry } from "../commands/registry";
import { toEngineKind } from "../commands/entityKindMap";
import { gateCreate } from "../subscription/entityCount";

/** The shape every CRUD-factory entity satisfies — pulled out so the extracted `*Core`
 * functions below can share it with `registerEntityCrudHandlers`'s own generic constraint. */
export type EntityBase = { id: string; workspaceId?: string; folderId?: string | null; enabled?: boolean; createdAt?: number };

/** Every kind registered via `registerCrudKind()`, keyed by its engine-internal storage `kind` string
 * (e.g. `"rules"`, `"sockets"`) — populated as each shell `*Handlers.ts` file registers its kinds.
 * Looked up lazily (at invoke time, not at registration time) by the generic
 * `entity.create`/`entity.update`/`entity.delete` commands below, so registration order across files
 * does not matter. Module-private on purpose: nothing outside this file reads it. */
const entityCrudRegistry = new Map<string, CrudFactoryOpts<any>>();

/** The three kinds that don't fit `CrudFactoryOpts` at all — `graphqlSchemas`, `protoFiles`, `wsdls`
 * are written straight to disk with `writeEntity(wsId, kind, id, data, null)`, nothing mirrors them
 * into an `AppConfig` array, and there is no "update" concept for any of them (only add/delete/list).
 * Registered via `registerSimpleKind()`, and consulted as a fallback by
 * `entity.create`/`entity.delete`/`entity.list` when a kind isn't in `entityCrudRegistry`. */
const simpleEntityKinds = new Set<string>();

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
  /** The friendly kind name `gateCreate()` (`../subscription/entityCount`) expects — e.g.
   * `"environment"`. Only `environments` sets this today (its pre-existing `env:add` handler
   * was the one CRUD-ish kind with a create-gate check); every other kind is ungated. Checked
   * by `entity.create`'s registered command handler, ahead of `createEntityCore()`, so a
   * blocked create never touches config/disk — see that handler for the shared shape. */
  gateKind?: string;
}

export interface SimpleEntityOpts {
  /** Engine-internal storage kind, e.g. `"graphqlSchemas"`. */
  kind: string;
  ipcAdd: string;
  ipcDelete: string;
  ipcList: string;
}

/** The engine half of `registerEntityCrudHandlers()` — the shell keeps the three `ipcMain.handle`
 * channels and calls this for the container insert. Split deliberately: the container is engine state,
 * and letting the shell mutate it directly is what made this file's dependency direction wrong. */
export function registerCrudKind<T extends EntityBase>(opts: CrudFactoryOpts<T>): void {
  entityCrudRegistry.set(opts.kind, opts);
}

/** The engine half of `registerSimpleEntityHandlers()` — see `registerCrudKind()`. */
export function registerSimpleKind(opts: SimpleEntityOpts): void {
  simpleEntityKinds.add(opts.kind);
}

async function isGitTracked(wsId: string, relPath: string): Promise<boolean> {
  try {
    const result = await getGit(wsId).raw(["ls-files", "--error-unmatch", "--", relPath]);
    return result.trim().length > 0;
  } catch {
    return false;
  }
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

export type SimpleEntityBase = { id: string; workspaceId?: string; createdAt?: number };

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

// ── The collapsed `entity.*` commands (P1 item 2 / P2 work item 7). They dispatch to whichever
// kind's `opts` is in `entityCrudRegistry` at invoke time, so it does not matter which shell
// `*Handlers.ts` file (and therefore which order) registered the kind first. A kind in neither
// `entityCrudRegistry` nor `simpleEntityKinds` throws a clear error rather than silently
// no-oping — today every `EntityKind` value is registered in one or the other.

export function registerEntityCrudCommands(registry: CommandRegistry): void {
  registry.register("entity.create", async ({ kind, entity, workspaceId }: EntityCreateParams, ctx: CommandContext) => {
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

  registry.register("entity.update", async ({ kind, entity }: EntityUpdateParams) => {
    const engineKind = toEngineKind(kind);
    const opts = entityCrudRegistry.get(engineKind);
    if (!opts) throw new Error(`entity.update: kind "${kind}" is not routed through the CommandRegistry yet`);
    return updateEntityCore(opts, entity as any);
  });

  registry.register("entity.delete", async ({ kind, id }: EntityDeleteParams) => {
    const engineKind = toEngineKind(kind);
    const opts = entityCrudRegistry.get(engineKind);
    if (opts) return deleteEntityCore(opts, id);
    if (simpleEntityKinds.has(engineKind)) return deleteSimpleEntityCore(engineKind, id);
    throw new Error(`entity.delete: kind "${kind}" is not routed through the CommandRegistry yet`);
  });

  registry.register("entity.list", ({ workspaceId, kind }: EntityListParams) => {
    const engineKind = toEngineKind(kind);
    if (!simpleEntityKinds.has(engineKind)) {
      throw new Error(`entity.list: kind "${kind}" is not routed through the CommandRegistry yet`);
    }
    return { entities: listSimpleEntitiesCore<Record<string, unknown>>(workspaceId, engineKind) };
  });
}
