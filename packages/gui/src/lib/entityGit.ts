/**
 * The single place that answers **"what can the user do with this entity's git state?"**
 *
 * ## The defect this file exists to remove
 *
 * Before this module the same question was answered independently in seven places, and they
 * disagreed. Each surface picked its own subset of `EntitySyncStatus` and its own words:
 *
 * | surface | primary action shown when | revert shown when | history shown when |
 * | --- | --- | --- | --- |
 * | folder-tree item menu | `syncSt !== "clean"`, labelled *Commit & Push* | same, *Discard Changes* / *Restore* | `!syncSt \|\| clean \| modified \| deleted` |
 * | folder-tree multi menu | any selected item dirty | **never** | n/a |
 * | editor `BottomBar` | `isDirty \|\| syncStatus !== "clean"`, labelled *Sync* | same | whenever not a draft |
 * | mapping card | `new \| modified`, labelled *Publish* | `modified` only | `clean \| modified` |
 * | rules table | `syncSt !== "clean"`, labelled *Publish* | same, labelled *Revert* | **unconditionally** |
 * | health bar | `!== "clean"`, labelled *Publish* | **no revert at all** | **no history at all** |
 * | environments | **nothing at all** | **nothing** | unconditional |
 *
 * Four vocabularies for two actions, and four different gates for History. Every one of those
 * decisions is now made here, once, and every surface renders the result. A surface that wants a
 * different rule is a change to this file — not a second opinion at a call site.
 *
 * ## The three resting states, and why two were not enough
 *
 * The old model had one axis (`EntitySyncStatus`, i.e. *does this file differ from HEAD?*), which
 * cannot distinguish:
 *
 * ```
 *   file == HEAD == remote     →  genuinely synced      →  no action
 *   file == HEAD ≠ remote      →  committed, unpushed   →  Sync (push)
 *   file ≠ HEAD                →  local work            →  Sync + Revert
 *   file not in the index      →  never published       →  Publish
 * ```
 *
 * The second row is the one that was missing, and it is the whole of the "Publish/Sync is
 * inconsistent" report: a committed-but-unpushed entity read `clean`, so it was offered **no
 * button**, and the footer's panel-level Publish was disabled too (it counts non-`clean` entries).
 * An entity could therefore be committed locally and be impossible to push from its own row. The
 * second axis is `WorkspaceGitState.ahead` — see `packages/engine/src/gitops/gitState.ts`.
 *
 * ## Reads fail open, writes fail closed
 *
 * `status === undefined` means *the status map has not loaded, or this path is not in it* — not
 * *clean*. The two directions are handled differently and deliberately:
 *
 *  - **Writes are refused.** No Publish, no Sync, no Revert. Offering a write on an unknown state
 *    risks a commit the user did not ask for, or a "revert" that deletes a file that was never
 *    committed.
 *  - **Reads are allowed.** History stays available, because it is the one action that cannot
 *    destroy anything, and refusing it would mean the History button flickers off on every status
 *    refresh — the refresh that the act of opening History itself triggers.
 */
import type { EntitySyncStatus, WorkspaceGitState } from "@/types";
import { strings } from "@/lib/strings";

/** The two write actions an entity can be offered. */
export type EntityGitPrimary = "publish" | "sync";

export interface EntityGitView {
    /** The file's position relative to HEAD, or `undefined` when the status map has no entry. */
    status: EntitySyncStatus | undefined;
    /** Whether the status is known at all — `false` disables every write. */
    known: boolean;
    /**
     * The primary action, or `null` when there is nothing to do. `"publish"` only for an untracked
     * entity; `"sync"` for local work *and* for a clean entity whose commits are not pushed.
     */
    primary: EntityGitPrimary | null;
    /** Whether the primary action is currently enabled. */
    primaryEnabled: boolean;
    /** Revert is offered: there is a diff against HEAD to throw away. */
    canRevert: boolean;
    /** History is offered: the entity has at least one commit to show. */
    canViewHistory: boolean;
    /**
     * Deleting this entity leaves a recoverable stub (`pending-deletions.json`) rather than removing
     * it outright — because it is tracked, so the deletion has to be committed.
     */
    deleteIsRecoverable: boolean;
    /** The button's label for `primary`. */
    primaryLabel: string;
    /** A tooltip explaining why the primary action is or is not available. */
    primaryTitle: string;
    /** The short status word for a badge. */
    stateLabel: string;
}

/**
 * Derive everything a surface needs to render an entity's git affordances.
 *
 * `git` may be `null` when no status refresh has reported one yet; that is treated as *unknown* and
 * only removes the push-only Sync case, which is the least surprising degradation — a missing remote
 * reading must never *invent* a Sync button, and it must never suppress the Publish/Revert that a
 * known file status already justifies.
 */
export function entityGitView(
    status: EntitySyncStatus | undefined,
    git: WorkspaceGitState | null | undefined,
): EntityGitView {
    const known = status !== undefined;

    // A clean file whose commits are not on the remote still needs the user's attention.
    const unpushedOnly = status === "clean" && !!git?.hasRemote && !!git?.ahead;

    const primary: EntityGitPrimary | null =
        status === "new" ? "publish"
        : status === "modified" || status === "deleted" ? "sync"
        : unpushedOnly ? "sync"
        : null;

    // Revert means "reset this file to the last synced version". An untracked entity has no such
    // version, so it is not offered — discarding it would be a delete, which is a different action
    // with its own confirmation. A clean entity has nothing to reset.
    const canRevert = status === "modified" || status === "deleted";

    return {
        status,
        known,
        primary,
        primaryEnabled: primary !== null,
        canRevert,
        // Tracked (clean / modified / deleted) or unknown. An untracked entity has no commits, so
        // History would open an empty panel — see the spec's "History is only applicable for
        // published entities".
        canViewHistory: !known || status !== "new",
        deleteIsRecoverable: !known || status !== "new",
        primaryLabel:
            primary === "publish" ? strings.entityGit.publish
            : primary === "sync" ? strings.entityGit.sync
            : "",
        primaryTitle:
            primary === "publish" ? strings.entityGit.publishTooltip
            : primary === "sync" ? (unpushedOnly ? strings.entityGit.pushTooltip : strings.entityGit.syncTooltip)
            : status === "new" ? strings.entityGit.publishTooltip
            : strings.entityGit.noChangesToSync,
        stateLabel: stateLabelFor(status, unpushedOnly),
    };
}

function stateLabelFor(status: EntitySyncStatus | undefined, unpushedOnly: boolean): string {
    switch (status) {
        case "new": return strings.entityGit.stateNew;
        case "modified": return strings.entityGit.stateModified;
        case "deleted": return strings.entityGit.stateDeleted;
        case "clean": return unpushedOnly ? strings.entityGit.stateUnpushed : strings.entityGit.stateSynced;
        default: return strings.entityGit.stateUnknown;
    }
}

/**
 * The status dot colour for an entity, in one place.
 *
 * Two copies of this existed (`MappingsPanel` and `ProxyRulesTable`) and they agreed by luck rather
 * than by construction. Follows the Chinese-market convention the rest of this app uses for
 * "something is outstanding": amber for an edit, red for a create or a delete.
 */
export function entityGitDotColor(status: EntitySyncStatus | undefined): "green" | "yellow" | "red" {
    if (status === "new" || status === "deleted") return "red";
    if (status === "modified") return "yellow";
    return "green";
}

/**
 * The 7-character form of a commit id — what `git log --oneline` prints, and what a person recognises.
 *
 * Kept here rather than inline at each surface because a **truncated id is only useful if it is
 * truncated the same way everywhere**: two surfaces showing 7 and 8 characters of the same commit look
 * like two different commits. The full id belongs in a `title` attribute beside it.
 */
export function shortCommit(sha: string): string {
    return sha.slice(0, 7);
}

// ── Folders and multi-selections ─────────────────────────────────────────────

export interface FolderGitRollup {
    /** How many entity files under the prefixes are not `clean`. */
    changed: number;
    /** The action for the group as a whole, or `null` when there is nothing to do. */
    primary: EntityGitPrimary | null;
    /** Whether any entity in the group is unpublished (untracked). */
    hasUnpublished: boolean;
    /** Whether any entity in the group has local work against HEAD. */
    hasLocalChanges: boolean;
    /** True when the group's only outstanding work is its first publish. */
    allNew: boolean;
}

/**
 * Roll a set of path prefixes up into one group action.
 *
 * **Derived from the status map, not from a separate git call**, and that is deliberate: the folder
 * menu and the entity rows inside it are then guaranteed to agree. A `git status` on the directory
 * would be a second, independently-timed read of the same thing, and the two would disagree for as
 * long as a publish was in flight.
 *
 * A prefix match rather than a directory listing also means a folder's *descendants* count, which is
 * what a user expects from "Publish All" on a parent — the old context menu published only the
 * folder's own path (`kind/Folder/`), silently skipping subfolders.
 */
export function folderGitRollup(
    prefixes: string[],
    files: Record<string, EntitySyncStatus>,
    git?: WorkspaceGitState | null,
): FolderGitRollup {
    const dirty = Object.entries(files).filter(
        ([p, s]) => s !== "clean" && prefixes.some((prefix) => p.startsWith(prefix)),
    );

    const changed = dirty.length;
    const hasUnpublished = dirty.some(([, s]) => s === "new");
    const hasLocalChanges = dirty.some(([, s]) => s !== "new");
    const allNew = changed > 0 && !hasLocalChanges;
    const unpushedOnly = changed === 0 && !!git?.hasRemote && !!git?.ahead;

    return {
        changed,
        primary: changed > 0 ? (allNew ? "publish" : "sync") : unpushedOnly ? "sync" : null,
        hasUnpublished,
        hasLocalChanges,
        allNew,
    };
}

// ── Panel → entity kind ──────────────────────────────────────────────────────

export interface PanelEntityKind {
    /** The engine storage kind — the first path segment, and the `folder.publish` argument. */
    kind: string;
    /** Singular noun for the footer's count. */
    label: string;
    /**
     * Every path prefix this panel's entities can live under.
     *
     * A list, not a string: the requests panel serves four storage kinds (`requests`,
     * `graphqlRequests`, `soapRequests`, `grpcRequests`), and the old single-prefix map named only
     * `requests/` — so a pending GraphQL, SOAP or gRPC request was invisible to the footer's count
     * *and* to its Publish button. The same map omitted `webhooks` entirely, which meant the
     * webhooks panel reported "up to date" with unpublishable work on screen and no way to publish it
     * from the footer.
     */
    prefixes: string[];
}

/**
 * One map for one question: *which entities does this panel publish?*
 *
 * This replaces three maps that had drifted apart — `PANEL_PREFIX` and `PANEL_ENTITY_LABEL` in
 * `GlobalFooter.tsx`, and `PANEL_PUBLISH_KIND` in `usePublishHandlers.ts`. The footer and the publish
 * handler disagreed about `webhooks` (the handler had it, the footer did not), which is why the
 * webhooks panel could not publish from the footer at all.
 *
 * `healthbar` is present even though it is not a folder-tree entity: it is a **singleton file**
 * (`healthbar/services.json`), so its panel publishes exactly one path.
 */
export const PANEL_ENTITY_KIND: Record<string, PanelEntityKind> = {
    mappings: { kind: "mappings", label: "mapping", prefixes: ["mappings/"] },
    rules: { kind: "rules", label: "rule", prefixes: ["rules/"] },
    requests: { kind: "requests", label: "request", prefixes: ["requests/", "graphqlRequests/", "soapRequests/", "grpcRequests/"] },
    "req-rest": { kind: "requests", label: "request", prefixes: ["requests/"] },
    "req-graphql": { kind: "graphqlRequests", label: "request", prefixes: ["graphqlRequests/"] },
    "req-soap": { kind: "soapRequests", label: "request", prefixes: ["soapRequests/"] },
    "req-grpc": { kind: "grpcRequests", label: "request", prefixes: ["grpcRequests/"] },
    mocks: { kind: "mocks", label: "mock", prefixes: ["mocks/"] },
    "mock-rest": { kind: "mocks", label: "mock", prefixes: ["mocks/"] },
    sockets: { kind: "sockets", label: "socket", prefixes: ["sockets/"] },
    environments: { kind: "environments", label: "environment", prefixes: ["environments/"] },
    healthbar: { kind: "healthbar", label: "health check", prefixes: ["healthbar/"] },
};

/** The panel's publishable kind, or `null` for a panel that publishes nothing (settings, audit, …). */
export function panelEntityKind(panel: string): PanelEntityKind | null {
    return PANEL_ENTITY_KIND[panel] ?? null;
}

/** The single path a singleton-file panel publishes, or `null` for a folder-tree panel. */
export function panelSingletonPath(panel: string): string | null {
    return panel === "healthbar" ? "healthbar/services.json" : null;
}

/**
 * A folder tree's `kind` prop → the storage kind its paths actually use.
 *
 * The tree names folders by *singular* kind (`kind="mock"`, matching `folder.add`'s `FolderKind`) while
 * every workspace path is *plural* (`mocks/…`). Both spellings are part of their own frozen contract, so
 * the translation lives here rather than being pushed into either.
 */
export const PLURAL_KIND_BY_TREE_KIND: Record<string, string> = {
    mock: "mocks",
    request: "requests",
    ws: "sockets",
    rule: "rules",
    graphqlRequest: "graphqlRequests",
    soapRequest: "soapRequests",
    grpcRequest: "grpcRequests",
    // A runner is a *view* over a request folder, not its own storage kind.
    runner: "requests",
};


// ── Kind → folder list ───────────────────────────────────────────────────────

/**
 * Which `AppConfig` array holds a kind's folders.
 *
 * `null` for a flat kind — and the `null` is load-bearing rather than a placeholder: it is what stops
 * `entityRelPath()` from being handed the wrong folder list and quietly producing a *root-level* path
 * for an entity that actually lives in a subfolder.
 *
 * **This map is why a GraphQL request published from the Requests panel used to miss.** The panel is
 * multiplexed across four storage kinds but was bound to `kind: "requests"` with `requestFolders`, so
 * a GraphQL request in a folder resolved to `graphqlRequests/<id>.json` — a path that does not exist
 * — and the publish either no-opped or committed nothing. Resolving the folder list from the
 * *resolved* kind, in one place, is the fix.
 */
export const FOLDER_KEY_BY_KIND: Record<string, string | null> = {
    requests: "requestFolders",
    mocks: "mockFolders",
    rules: "ruleFolders",
    sockets: "wsFolders",
    graphqlRequests: "graphqlRequestFolders",
    soapRequests: "soapRequestFolders",
    grpcRequests: "grpcRequestFolders",
    mappings: null,
    environments: null,
};

/**
 * Every kind an entity id could belong to, in the order a lookup should try them.
 *
 * Used to recover from a caller that named the wrong kind — which is the normal case for the
 * multiplexed Requests panel, not an error path. Ordered so the common kinds are tried first; the
 * order only affects cost, never the answer, because entity ids are unique.
 */
export const ALL_ENTITY_KINDS_IN_UI: readonly string[] = [
    "requests", "mocks", "rules", "sockets",
    "graphqlRequests", "soapRequests", "grpcRequests",
    "mappings", "environments",
];

/** The folder list a kind's entities live under, read from the workspace config. */
export function foldersForKind(cfg: any, kind: string): Array<{ id: string; name: string }> {
    const key = FOLDER_KEY_BY_KIND[kind];
    if (!key) return [];
    return (cfg?.[key] ?? []) as Array<{ id: string; name: string }>;
}

