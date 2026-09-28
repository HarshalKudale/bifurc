/**
 * The **one** handler for every git write the UI performs on an entity.
 *
 * ## Why one hook replaced seven factories
 *
 * `usePublishHandlers` exposed seven near-identical factories — `makePublishItem`,
 * `makePublishFolder`, `makeFlatPublish`, `makeFlatRevert`, `makeRestoreItem`,
 * `handlePublishHealthBar`, `handlePublishPanel` — and each was a separate opportunity to disagree
 * about what "publish this entity" means. They had. `makeFlatPublish` threw on failure while
 * `makePublishItem` ignored the result entirely, so a failed publish from the folder tree was
 * **silent**; and only one of the seven refreshed the entity status after the config. The four
 * actions that actually exist are:
 *
 * | action | git operation | when |
 * | --- | --- | --- |
 * | **publish** | `git add -A -- <paths>` → commit → push | untracked, or any local work |
 * | **revert** | `git checkout HEAD -- <path>` (or delete if untracked) | local work against HEAD |
 * | **recover** | identical to revert | a deleted-but-uncommitted entity |
 * | **history** | read-only, `history.list` | any tracked entity |
 *
 * So this hook exposes exactly two writers — `publish(target)` and `revert(target)` — plus
 * `publishPanel()` for the footer. "Recover" is not a third operation: it *is* `revert`, and giving
 * it its own function is how a second implementation of revert appeared in the engine
 * (`publishService.restoreEntity`, since deleted). "Sync" and "Publish" are not two operations
 * either — they are two labels for `publish`, chosen by `entityGitView()` from the entity's state.
 *
 * ## A target, not a path
 *
 * Callers name *what* they mean (an entity, a folder, a bare path) and this hook resolves it. That
 * resolution is where the last real inconsistency lived: the mapping panel computed
 * `mappings/<id>.json` itself while the folder tree asked the engine to sanitise a folder name, and
 * the two used different sanitisers — `lib/utils.ts#sanitizeDirName` strips illegal characters while
 * the engine's `folder.publish` replaces them with `-`. Resolving an entity's path here means a
 * folder *rename* and a folder *publish* can no longer compute different paths for the same folder.
 * (The folder case still goes through `folder.publish`, because the engine deliberately owns that
 * sanitiser — see `sync/syncCommands.ts`'s header. This hook therefore passes the **raw** folder
 * name and never pre-sanitises it.)
 *
 * ## Failures are reported, never thrown and never swallowed
 *
 * Every writer resolves to `GitWriteResult`. It does **not** reject, and that is a decision about
 * the call sites rather than about style: they are almost all fire-and-forget context-menu actions
 * (`onClick={() => onPublishItem(id)}`), so a rejected promise would surface as an unhandled
 * rejection in the console and nowhere else — which is the same silent failure the old code had,
 * wearing a stack trace. Instead the result is returned for a caller that has its own error UI, and
 * recorded in `lastError` for the ambient one that `GlobalFooter` renders on every screen.
 *
 * ## Every write ends the same way
 *
 * `refreshConfig()` → `refreshEntitySyncStatus()` → `onAfterWrite()`. In that order: the status map
 * is derived from disk, so re-reading the config first means the tree and the status agree by the
 * time React renders. `onAfterWrite` is what bumps the History sidebar's reload key, so "the entity
 * changed while History was open" is handled for *every* panel rather than only the four that
 * remembered to pass `onAfterSave`.
 */
import { useCallback, useState } from "react";
import type { AppConfig } from "@/types";
import { entityRelPath } from "@/lib/utils";
import {
    panelEntityKind,
    panelSingletonPath,
    foldersForKind,
    ALL_ENTITY_KINDS_IN_UI,
} from "@/lib/entityGit";

/** What the caller wants acted on. */
export type GitTarget =
    /**
     * One entity, by kind and id.
     *
     * No folder list: the hook reads it from the config itself (`foldersForKind`), keyed on the kind
     * it actually resolved. Passing folders in from the call site is how the Requests panel ended up
     * resolving a GraphQL request's path against `requestFolders` and producing a path that does not
     * exist.
     */
    | { type: "entity"; kind: string; id: string }
    /** A workspace-relative path the caller already knows — the folder tree has one per item. */
    | { type: "path"; path: string }
    /**
     * Several entities at once, in **one commit**.
     *
     * A multi-selection used to fire one `publish` per selected entity, which produced N commits for
     * one user gesture — and the History sidebar then showed N separate entries for what the user did
     * once. `entity.publish` has always accepted `paths: string[]`; only the UI was not using it.
     */
    | { type: "paths"; paths: string[] }
    /** A whole folder inside a kind, or the whole kind when `folderName` is `null`. */
    | { type: "folder"; kind: string; folderName: string | null };

export interface GitWriteResult {
    ok: boolean;
    error?: string;
    /**
     * The commit this write created, or `null` when there was nothing to commit.
     *
     * **This is the proof that a git operation happened**, and `ok` is not: Publish and Sync are both
     * `git add` + `git commit`, so both produce a hash — while `ok: true` is also what the engine
     * returns for a publish that staged nothing. Reporting the id is what makes *"clicking publish
     * does nothing"* answerable from the UI, because `null` and a hash are different outcomes and can
     * be said differently.
     *
     * **Absent** for an operation that never commits — a revert is `git checkout HEAD -- <path>` and
     * creates nothing. Absent and `null` are deliberately distinct: `null` means "a commit was
     * expected and did not happen", absent means "this operation does not commit".
     */
    commit?: string | null;
}

function getItemFromConfig(cfg: any, kind: string, id: string) {
    if (kind === "rules") return (cfg.proxyRules ?? []).find((r: any) => r.id === id);
    if (kind === "requests") return (cfg.requests ?? []).find((r: any) => r.id === id);
    if (kind === "mocks") return (cfg.mocks ?? []).find((m: any) => m.id === id);
    if (kind === "webhooks") return (cfg.webhooks ?? []).find((h: any) => h.id === id);
    if (kind === "sockets") return (cfg.wsConnections ?? []).find((c: any) => c.id === id);
    if (kind === "graphqlRequests") return (cfg.graphqlRequests ?? []).find((r: any) => r.id === id);
    if (kind === "soapRequests") return (cfg.soapRequests ?? []).find((r: any) => r.id === id);
    if (kind === "grpcRequests") return (cfg.grpcRequests ?? []).find((r: any) => r.id === id);
    if (kind === "mappings") return (cfg.mappings ?? []).find((m: any) => m.id === id);
    if (kind === "environments") return (cfg.environments ?? []).find((e: any) => e.id === id);
    return null;
}

/** A stable key for one target — used for the in-flight set, so a button can spin. */
function targetKey(op: string, target: GitTarget): string {
    if (target.type === "path") return `${op}:${target.path}`;
    if (target.type === "paths") return `${op}:${target.paths.join("|")}`;
    if (target.type === "folder") return `${op}:${target.kind}/${target.folderName ?? ""}`;
    return `${op}:${target.kind}/${target.id}`;
}

export interface EntityGitActions {
    /** Commit and push. Serves Publish and Sync — the label is `entityGitView()`'s decision. */
    publish(target: GitTarget, message?: string): Promise<GitWriteResult>;
    /** Discard local changes and restore the last synced version. Serves Revert and Recover. */
    revert(target: GitTarget): Promise<GitWriteResult>;
    /** The footer's action for the active panel. */
    publishPanel(): Promise<GitWriteResult>;
    /** `true` while a write for this target is in flight. */
    isBusy(key: string): boolean;
    /** The most recent failure, for the ambient error surface. `null` once dismissed. */
    lastError: string | null;
    clearError(): void;
    /** The in-flight key for a target, so a surface can ask `isBusy(targetKey("publish", t))`. */
    keyFor(op: "publish" | "revert", target: GitTarget): string;
}

export function useEntityGitActions({
    wsConfig,
    wsId,
    refreshConfig,
    refreshEntitySyncStatus,
    panel,
    onAfterWrite,
}: {
    wsConfig: AppConfig;
    wsId: string;
    refreshConfig: () => Promise<void>;
    refreshEntitySyncStatus: (id: string) => void;
    panel: string;
    onAfterWrite?: () => void;
}): EntityGitActions {
    const [busy, setBusy] = useState<Record<string, boolean>>({});
    const [lastError, setLastError] = useState<string | null>(null);

    /**
     * An entity's path, read from the **freshest** config available and with the folder list matched
     * to the kind that actually holds the id.
     *
     * Two things are happening here, and both are fixes rather than caution:
     *
     *  1. **The fresh read.** `wsConfig` is a render-time snapshot, and a create that has not
     *     round-tripped yet leaves the new entity missing from it. The old mapping handler had this
     *     fallback; the folder-tree handlers did not, which is why publishing a just-created mock
     *     from the tree could silently no-op (`if (!item) return`).
     *  2. **The kind recovery.** A caller that named the wrong kind is the *normal* case for the
     *     multiplexed Requests panel — it is bound to `"requests"` but its tree holds GraphQL, SOAP
     *     and gRPC requests too. Scanning for the kind that actually contains the id turns a silent
     *     no-op into the right path. Entity ids are unique, so the scan can only confirm.
     */
    const entityPath = useCallback(
        async (kind: string, id: string): Promise<string | null> => {
            let cfg: any = wsConfig;
            let item = getItemFromConfig(cfg, kind, id);
            if (!item) {
                try {
                    cfg = await window.api.getConfig();
                    item = getItemFromConfig(cfg, kind, id);
                } catch {
                    return null;
                }
            }
            if (!item) {
                for (const candidate of ALL_ENTITY_KINDS_IN_UI) {
                    if (candidate === kind) continue;
                    const found = getItemFromConfig(cfg, candidate, id);
                    if (found) {
                        kind = candidate;
                        item = found;
                        break;
                    }
                }
            }
            if (!item) return null;
            return entityRelPath(kind, item as any, foldersForKind(cfg, kind));
        },
        [wsConfig],
    );

    /** Both writers share this: mark busy, run, report, refresh. Never rejects. */
    const write = useCallback(
        async (key: string, fn: () => Promise<GitWriteResult>): Promise<GitWriteResult> => {
            setBusy((p) => ({ ...p, [key]: true }));
            try {
                const result = await fn();
                setLastError(result.ok ? null : (result.error ?? "Git operation failed"));
                /**
                 * **Refreshed on failure too, and that is not belt-and-braces.**
                 *
                 * `publishEntities()` stages *before* it commits and pushes: a failure at the push
                 * step leaves a real commit behind, and a failure at the commit step leaves the
                 * change staged. So `{ok: false}` does **not** mean "nothing happened" — it means
                 * "the last step failed", and a UI that skipped the refresh would go on showing the
                 * pre-write state. That is the same "does nothing" symptom as a silent no-op, with an
                 * error message the user has to notice on top of it.
                 *
                 * On success the order is config first, then status: the status map is derived from
                 * disk, so re-reading the config first means the tree and the status agree by the time
                 * React renders.
                 */
                await refreshConfig();
                refreshEntitySyncStatus(wsId);
                onAfterWrite?.();
                return result;
            } catch (e) {
                // A thrown error can only come from the engine bridge itself (a dead session, a
                // transport failure) — everything below returns `{ok:false}` instead.
                const message = e instanceof Error ? e.message : String(e);
                setLastError(message);
                return { ok: false, error: message };
            } finally {
                setBusy((p) => ({ ...p, [key]: false }));
            }
        },
        [refreshConfig, refreshEntitySyncStatus, wsId, onAfterWrite],
    );

    const publish = useCallback(
        (target: GitTarget, message?: string) =>
            write(targetKey("publish", target), async () => {
                if (target.type === "folder") {
                    return window.api.publishFolder(wsId, target.kind, target.folderName);
                }
                if (target.type === "paths") {
                    if (target.paths.length === 0) return { ok: true };
                    return window.api.publishEntity(wsId, target.paths, message);
                }
                const path =
                    target.type === "path"
                        ? target.path
                        : await entityPath(target.kind, target.id);
                if (!path) return { ok: false, error: "This entity is no longer in the workspace." };
                return window.api.publishEntity(wsId, [path], message);
            }),
        [write, wsId, entityPath],
    );

    const revert = useCallback(
        (target: GitTarget) =>
            write(targetKey("revert", target), async () => {
                if (target.type === "folder" || target.type === "paths") {
                    // Deliberate: Revert is a per-entity action. A folder revert would have to decide
                    // what to do with untracked files *inside* it, and "delete them" is a destructive
                    // answer that one click on a folder header should not be able to give. A batch
                    // revert is the same decision repeated, so it belongs at the call site where each
                    // file can be named — which is what the folder tree does.
                    return { ok: false, error: "Revert applies to a single entity, not a group." };
                }
                const path =
                    target.type === "path"
                        ? target.path
                        : await entityPath(target.kind, target.id);
                if (!path) return { ok: false, error: "This entity is no longer in the workspace." };
                return window.api.gitDiscard(wsId, path);
            }),
        [write, wsId, entityPath],
    );

    const publishPanel = useCallback(() => {
        const pek = panelEntityKind(panel);
        if (!pek) return Promise.resolve<GitWriteResult>({ ok: true });
        return write(`publish:panel:${panel}`, () => {
            const singleton = panelSingletonPath(panel);
            if (singleton) return window.api.publishEntity(wsId, [singleton]);
            return window.api.publishFolder(wsId, pek.kind, null);
        });
    }, [panel, write, wsId]);

    const clearError = useCallback(() => setLastError(null), []);

    return {
        publish,
        revert,
        publishPanel,
        isBusy: (key) => !!busy[key],
        lastError,
        clearError,
        keyFor: targetKey,
    };
}
