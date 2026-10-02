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
import {
    panelEntityKind,
    panelSingletonPath,
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

// `getItemFromConfig()` lived here until 2026-09-30. It scanned the config for an id, because the kind a
// caller named was not always the kind that held it — the multiplexed Requests panel is bound to
// `"requests"` while its tree also holds GraphQL, SOAP and gRPC requests. The engine's map is keyed by
// **id alone**, so that recovery stopped being a caller-side concern (`plan/19` WI 5, item 3).

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
    relPathOf,
    panel,
    onAfterWrite,
}: {
    wsConfig: AppConfig;
    wsId: string;
    refreshConfig: () => Promise<void>;
    refreshEntitySyncStatus: (id: string) => void;
    /**
     * The engine's `entity id → relPath` lookup (`plan/19` WI 5 item 3).
     *
     * **Passed in rather than read from `useEntityPaths()`**, and that is not a shortcut: this hook runs in
     * `App`'s own body, above any provider `App` renders, so a context read here would return the default
     * (empty) lookup. `App` builds it with `entityPathLookup(entityPaths)` and passes it down.
     */
    relPathOf: (id: string) => string;
    panel: string;
    onAfterWrite?: () => void;
}): EntityGitActions {
    const [busy, setBusy] = useState<Record<string, boolean>>({});
    const [lastError, setLastError] = useState<string | null>(null);

    /**
     * An entity's path — from the **engine's** map, with one re-ask before giving up.
     *
     * This used to compute the path here, and two of the three things it did are now the engine's or gone:
     *
     *  1. **The kind recovery is gone because it is no longer needed.** The old code scanned every kind
     *     looking for the id, because the multiplexed Requests panel is bound to `"requests"` while its tree
     *     also holds GraphQL, SOAP and gRPC requests. The engine's map is keyed by **id alone** — ids are
     *     unique — so a caller that names the wrong kind gets the right path anyway.
     *  2. **The fresh read is now a fallback rather than the first move.** The map is a render-time snapshot
     *     too, and the bug the fresh read fixed was real (publishing a just-created mock from the tree
     *     silently no-op'd — `if (!item) return`). So an id the map does not know re-asks the engine
     *     **once**, which keeps that fix without keeping a second path implementation.
     */
    const entityPath = useCallback(
        async (id: string): Promise<string | null> => {
            const known = relPathOf(id);
            if (known) return known;
            try {
                const payload = await window.api.getEntitySyncStatus(wsId);
                return payload.paths?.entities[id] ?? null;
            } catch {
                return null;
            }
        },
        [relPathOf, wsId],
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
                        : await entityPath(target.id);
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
                        : await entityPath(target.id);
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
