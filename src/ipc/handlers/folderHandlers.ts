import { ipcMain } from "electron";
import type { FolderKindValue } from "@bifurc/protocol";
import { Folder } from "@bifurc/engine/store/config";
import { bus } from "@bifurc/engine/eventBus";
import { commandRegistry } from "@bifurc/engine/commands/registry";

// ## Channels only — the registry half now lives in the engine
//
// `folder.add` / `folder.rename` / `folder.move` / `folder.delete` used to be registered at **module
// scope** in this file, which is the `plan/07` §5c defect: a containerised engine (P9) answered
// `UNKNOWN_COMMAND` for all four. Step 3e slice 3 (2026-09-19) moved them into
// `packages/engine/src/store/folderCommands.ts` behind `registerFolderCommands()`, called from
// `src/ipc/handlers.ts`.
//
// Every channel below **delegates** through `commandRegistry.invoke()` rather than duplicating the
// body — unlike `runnerHandlers.ts`, where three of the five channels are self-contained copies. That
// distinction decides what breaks: a delegating channel is only ever as alive as the registry behind
// it, so a suite that registers this handler group by hand **must** also call
// `registerFolderCommands(commandRegistry)`, or the channel exists with nothing behind it. The failure
// is `No handler registered` at *invocation* time, so nothing catches it at build time.
//
// P2 work item 7 — `folder.*` is a clean 1:1 with its legacy channel. `FolderKindValue` in
// `packages/protocol/src/commands/folder.ts` is the same 11-value enum the engine module's local
// `FolderKind` declares, so the channel parameter is typed as the protocol's enum directly and no cast
// is needed.

const ctx = { bus };

export function registerFolderHandlers() {
  ipcMain.handle("folder:add", (_e, kind: FolderKindValue, folder: Omit<Folder, "id" | "createdAt">) =>
    commandRegistry.invoke("folder.add", { kind, ...folder }, ctx));

  ipcMain.handle("folder:rename", (_e, kind: FolderKindValue, id: string, name: string) =>
    commandRegistry.invoke("folder.rename", { kind, id, name }, ctx));

  ipcMain.handle("folder:move", (_e, kind: FolderKindValue, id: string, parentId: string | null) =>
    commandRegistry.invoke("folder.move", { kind, id, parentId }, ctx));

  ipcMain.handle("folder:delete", (_e, kind: FolderKindValue, id: string) =>
    commandRegistry.invoke("folder.delete", { kind, id }, ctx));
}
