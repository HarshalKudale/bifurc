/**
 * `folder.add` / `folder.rename` / `folder.move` / `folder.delete` — the four folder commands, moved
 * out of `src/ipc/handlers/folderHandlers.ts` (P6 step 3e, slice 3).
 *
 * ## Why they moved (§5c)
 *
 * They were registered at **module scope** in the shell file, so a containerised engine (P9) answered
 * `UNKNOWN_COMMAND` for all four. That matters more here than for most §5c slices: `folderHandlers.ts`'s
 * channel adapters are **thin `commandRegistry.invoke()` delegations** rather than self-contained
 * duplicates, so in the shell these four channels were only ever as alive as the registry behind them —
 * and on a headless host there was no registry behind them at all.
 *
 * ## The shape worth understanding before editing: folders live in *two* places
 *
 * Every command here has to keep two stores in step, and the pairs are not the same thing:
 *
 *  - **`AppConfig`** — the folder array is `FOLDER_CONFIG[kind].configKey` (`mockFolders`,
 *    `requestFolders`, …). This is what the renderer reads to draw the sidebar.
 *  - **`index.json` in the workspace** — `readIndex()` / `writeIndex()` keep the folder list *and*
 *    `order` (the manual drag order) per `fsKind`. This is what the on-disk tree is derived from.
 *
 * `folder.rename` and `folder.move` update both; `folder.add` writes the config first and then the
 * index; `folder.delete` additionally detaches the folder's entities from the config array
 * (`entityConfigKey`) and drops their ids from `index.order`. **`fsKind` is not the same string as
 * `kind`** — a websocket folder is `kind: "ws"` with `fsKind: "sockets"`, and a rule folder is
 * `"rule"` / `"rules"` — which is why the table is copied here rather than derived.
 *
 * ## Behaviour that looks like a bug and is not
 *
 * `folder.rename` and `folder.move` **resolve `{ok: true}` even when the id matches no folder**. The
 * `if (f)` guards mean the work is skipped, not that the caller is told. That is the shell's behaviour
 * and it is preserved rather than improved: a move that starts reporting a failure for an unknown id
 * is a contract change smuggled in under a move, and the renderer's declared result is `{ok: boolean}`.
 * The same reasoning kept `{ok: false}` (no `error`) in `runner.saveConfig`.
 *
 * `folder.rename`'s `renameSync` → `cpSync` + `rmSync` fallback is also preserved verbatim: on Windows
 * a rename can fail across a lock, and the copy path is what makes it work. The `console.warn` on a
 * double failure is the shell's, and stays — a silent no-op there would leave the config renamed and
 * the directory not.
 */
import * as fs from "fs";
import * as path from "path";
import type {
    FolderAddParams,
    FolderDeleteParams,
    FolderMoveParams,
    FolderRenameParams,
} from "@bifurc/protocol";
import { loadConfig, saveConfig, generateId, type Folder } from "../config";
import {
    readIndex,
    writeIndex,
    wsDir,
    sanitizeDirName,
    deleteEntityDir,
} from "../workspaceFs";
import { bus } from "../../kernel/eventBus";
import type { CommandRegistry } from "../../kernel/registry";

/**
 * The 11 folder kinds, as the shell declared them. `packages/protocol/src/commands/folder.ts`'s
 * `FolderKindValue` is the same enum, which is why P2 work item 7 could convert these four
 * commands without a schema gap — and why the channel adapters cast rather than translate.
 * (`plan/17` work item 8 removed three of its values — the three mock folder kinds — which changes
 * the enum's size but not that argument.)
 */
type FolderKind = "mock" | "request" | "ws" | "rule" | "graphqlRequest" | "grpcRequest" | "soapRequest";

/**
 * `kind` → where that kind's folders live. `configKey` is the `AppConfig` array; `entityConfigKey` is
 * the `AppConfig` array holding the entities *inside* those folders (used only by `folder.delete`, to
 * detach them); `fsKind` is the directory name under the workspace.
 *
 * Note `fsKind` is not derivable from `kind`: `ws` → `sockets`, `rule` → `rules`. Keep the table.
 *
 * `plan/17` work item 8 removed the three mock folder kinds (`graphqlMock`, `grpcMock`, `soapMock`)
 * with their entity kinds; REST mocks are the only mock kind left, and they were never foldered
 * separately.
 */
const FOLDER_CONFIG: Record<FolderKind, { configKey: string; entityConfigKey?: string; fsKind: string }> = {
    mock: { configKey: "mockFolders", entityConfigKey: "mocks", fsKind: "mocks" },
    ws: { configKey: "wsFolders", entityConfigKey: "wsConnections", fsKind: "sockets" },
    rule: { configKey: "ruleFolders", entityConfigKey: "proxyRules", fsKind: "rules" },
    graphqlRequest: { configKey: "graphqlRequestFolders", entityConfigKey: "graphqlRequests", fsKind: "graphqlRequests" },
    grpcRequest: { configKey: "grpcRequestFolders", entityConfigKey: "grpcRequests", fsKind: "grpcRequests" },
    soapRequest: { configKey: "soapRequestFolders", entityConfigKey: "soapRequests", fsKind: "soapRequests" },
    request: { configKey: "requestFolders", entityConfigKey: "requests", fsKind: "requests" },
};

export function registerFolderCommands(registry: CommandRegistry): void {
    registry.register("folder.add", async (params: FolderAddParams) => {
        const kind = params.kind as FolderKind;
        const cfg = loadConfig() as any;
        const wsId = params.workspaceId ?? cfg.activeWorkspaceId;
        const newFolder: Folder = { name: params.name, parentId: params.parentId, id: generateId(), createdAt: Date.now(), workspaceId: wsId };

        const conf = FOLDER_CONFIG[kind];
        cfg[conf.configKey] = cfg[conf.configKey] ?? [];
        cfg[conf.configKey].push(newFolder);
        saveConfig(cfg);

        const folderDir = path.join(wsDir(wsId), conf.fsKind, sanitizeDirName(newFolder.name));
        fs.mkdirSync(folderDir, { recursive: true });

        const idx = readIndex(wsId, conf.fsKind as any);
        idx.folders.push(newFolder);
        writeIndex(wsId, conf.fsKind as any, idx);
        bus.emitTyped("entity.changed", { wsId, kind, id: newFolder.id, action: "created" });
        return newFolder;
    });

    registry.register("folder.rename", (params: FolderRenameParams) => {
        const kind = params.kind as FolderKind;
        const cfg = loadConfig() as any;
        const conf = FOLDER_CONFIG[kind];
        const arr: Folder[] = cfg[conf.configKey] ?? [];

        const f = arr.find((x: Folder) => x.id === params.id);
        const oldName = f?.name;
        if (f) f.name = params.name;
        cfg[conf.configKey] = arr;
        saveConfig(cfg);

        if (f && oldName) {
            const base = path.join(wsDir(f.workspaceId), conf.fsKind);
            const oldDir = path.join(base, sanitizeDirName(oldName));
            const newDir = path.join(base, sanitizeDirName(params.name));
            if (fs.existsSync(oldDir) && oldDir !== newDir) {
                try { fs.renameSync(oldDir, newDir); }
                catch (renameErr) {
                    try {
                        fs.cpSync(oldDir, newDir, { recursive: true });
                        fs.rmSync(oldDir, { recursive: true, force: true });
                    } catch (copyErr) { console.warn(`[folder:rename] Error:`, (copyErr as Error).message); }
                }
            }
            const idx = readIndex(f.workspaceId, conf.fsKind as any);
            const fi = idx.folders.find((x: Folder) => x.id === params.id);
            if (fi) fi.name = params.name;
            writeIndex(f.workspaceId, conf.fsKind as any, idx);
        }
        return { ok: true };
    });

    registry.register("folder.move", (params: FolderMoveParams) => {
        const kind = params.kind as FolderKind;
        const cfg = loadConfig() as any;
        const conf = FOLDER_CONFIG[kind];
        const arr: Folder[] = cfg[conf.configKey] ?? [];

        const f = arr.find((x: Folder) => x.id === params.id);
        if (f) f.parentId = params.parentId;
        cfg[conf.configKey] = arr;

        if (f) {
            const idx = readIndex(f.workspaceId, conf.fsKind as any);
            const fi = idx.folders.find((x: Folder) => x.id === params.id);
            if (fi) fi.parentId = params.parentId;
            writeIndex(f.workspaceId, conf.fsKind as any, idx);
        }
        saveConfig(cfg);
        return { ok: true };
    });

    registry.register("folder.delete", async (params: FolderDeleteParams) => {
        const kind = params.kind as FolderKind;
        const cfg = loadConfig() as any;
        const conf = FOLDER_CONFIG[kind];

        const arr: Folder[] = cfg[conf.configKey] ?? [];
        const folder = arr.find((f: Folder) => f.id === params.id);

        let affectedEntityIds: string[] = [];
        if (conf.entityConfigKey) {
            const entities: any[] = cfg[conf.entityConfigKey] ?? [];
            affectedEntityIds = entities.filter(e => e.folderId === params.id).map(e => e.id);
            cfg[conf.entityConfigKey] = entities.filter(e => e.folderId !== params.id);
        }
        cfg[conf.configKey] = arr.filter((f: Folder) => f.id !== params.id);
        saveConfig(cfg);

        if (folder) {
            const wsId = folder.workspaceId;
            deleteEntityDir(wsId, conf.fsKind as any, folder.name);

            // A request folder owns its run history, and it is keyed by *folder id* rather than by
            // name, so `deleteEntityDir` does not reach it. Deleting the folder without this would
            // leave `.runs/<id>/` behind forever, with nothing left in the UI to reach it.
            if (kind === "request") {
                const runnerDir = path.join(wsDir(wsId), "requests", ".runs", params.id);
                if (fs.existsSync(runnerDir)) fs.rmSync(runnerDir, { recursive: true, force: true });
            }

            const idx = readIndex(wsId, conf.fsKind as any);
            idx.folders = idx.folders.filter((f: any) => f.id !== params.id);
            const deletedSet = new Set(affectedEntityIds);
            idx.order = (idx.order ?? []).filter((eid: string) => !deletedSet.has(eid));
            writeIndex(wsId, conf.fsKind as any, idx);
        }
        return { ok: true };
    });
}
