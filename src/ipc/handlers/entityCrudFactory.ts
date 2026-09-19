import { ipcMain } from "electron";
import { loadConfig } from "@bifurc/engine/store/config";
import { bus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@bifurc/engine/commands/registry";
import { toProtocolKind } from "@bifurc/engine/commands/entityKindMap";
import {
  registerCrudKind, registerSimpleKind,
  type CrudFactoryOpts, type SimpleEntityOpts,
  type EntityBase, type SimpleEntityBase,
} from "@bifurc/engine/store/entityCrudCommands";

// ## Channels only — the registry half now lives in the engine
//
// The four collapsed `entity.*` commands (`create` / `update` / `delete` / `list`) used to be
// registered at **module scope** in this file, which is the `plan/07` §5c defect: a containerised
// engine (P9) answered `UNKNOWN_COMMAND` for all four. Step 3e slice 7 (2026-09-19) moved them into
// `packages/engine/src/store/entityCrudCommands.ts` behind `registerEntityCrudCommands()`, called
// from `src/ipc/handlers.ts`.
//
// ## Why this file was the awkward one, and what the split actually is
//
// Slices 1–6 each moved a *file of commands*. This file is a **factory split across the seam**, and
// the dependency ran the wrong way: the four commands dispatch through `entityCrudRegistry` /
// `simpleEntityKinds` at invoke time, but those containers were populated by the **shell** files
// calling `registerEntityCrudHandlers()` / `registerSimpleEntityHandlers()` below. Moving the
// registrations without the containers would have left the four commands consulting an **empty** map —
// every one throwing `"<kind>" is not routed through the CommandRegistry yet`, which is the same
// message a genuinely unregistered kind throws. A plausible error message caused by the move itself.
//
// So the containers went to the engine and the population direction was inverted: this file now calls
// `registerCrudKind()` / `registerSimpleKind()` (the **engine half** of the two factory functions) and
// keeps the three `ipcMain.handle` channels each (the **shell half**). Both exported signatures are
// byte-identical, so **none of the six caller files** — `coreHandlers`, `crudHandlers`,
// `graphqlHandlers`, `grpcHandlers`, `soapHandlers` — needed an edit. That is the property worth
// checking first when what has to move is a factory rather than a command list.
//
// ## What is left here, and why each piece has to be
//
//  - `ipcMain` — genuinely shell-side.
//  - The channel bodies, which **delegate** through `commandRegistry.invoke()`. Same consequence as
//    slices 3–6: a delegating channel is only ever as alive as the registry behind it, so a suite that
//    registers this handler group by hand **must** also call `registerEntityCrudCommands()`, or the
//    channel exists with nothing behind it. The failure is `No handler registered` at *invocation*
//    time, invisible to the compiler.
//  - The `entity.create` result unwrapping. `entity.create` returns `{id, entity}` — the frozen
//    protocol shape — while the legacy channel has always returned the raw created entity, so the
//    wrapper is unwrapped to keep the wire response byte-identical. A gate-blocked create (only
//    `environments` sets `gateKind`) returns `{error, ...gate}` instead, with no `.entity` to unwrap,
//    so that shape passes through unchanged — matching `env:add`'s pre-collapse
//    `{error: "limit_reached", ...gate}` return.
//
// The two `import`s of engine modules other than `entityCrudCommands` are for `loadConfig` (the list
// channel resolves the active workspace), `toProtocolKind` (the wire vocabulary renames two of the
// twelve kinds) and `bus` (the `CommandContext` every delegation passes).
const ctx = { bus };

export function registerEntityCrudHandlers<T extends EntityBase>(opts: CrudFactoryOpts<T>) {
  registerCrudKind(opts);
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

export type { CrudFactoryOpts, SimpleEntityOpts, EntityBase, SimpleEntityBase };

/** Registers the three no-`AppConfig`-array, no-update kinds (`graphqlSchemas`, `protoFiles`,
 * `wsdls`) the same way `registerEntityCrudHandlers` registers the twelve full-CRUD kinds: the
 * legacy per-kind channels become thin adapters over `commandRegistry.invoke()`. */
export function registerSimpleEntityHandlers<T extends SimpleEntityBase>(opts: SimpleEntityOpts) {
  registerSimpleKind(opts);
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
