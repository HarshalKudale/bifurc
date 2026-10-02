import React, { createContext, useContext, useMemo } from "react";

/**
 * The engine's `entity id → relPath` and `folder id → directory prefix` maps, handed to the renderer as a
 * **lookup** — `plan/19` WI 5, item 3.
 *
 * ## What this replaces
 *
 * `lib/utils.ts` had `sanitizeDirName`, `entityRelPath` and `flatEntityRelPath` — a hand-mirror of the
 * engine's `workspaceFs.ts`, whose own docblock said so. Every panel and every git action called one of them
 * to turn an entity into the path the engine's status map and git commands are keyed by.
 *
 * The maps now come from `sync.getEntityStatus` (`EntityStatusPayload.paths`, built by
 * `packages/engine/src/workspace/entityPaths.ts`), and this context is how they reach ~10 consumers without
 * a second prop threaded through `panelFactory` and every panel signature. `entitySyncStatus` already
 * travels that way and it is fifteen props; a second one would be fifteen more for data that is not
 * per-panel.
 *
 * ## Why a context and not a module-level cache
 *
 * A module-level variable would have been one line per consumer instead of one import, and it would be
 * wrong: it does not participate in React's render cycle, so a consumer that rendered between the store
 * write and the state update would read a stale map with nothing to correct it. The provider re-renders
 * when the payload changes, which is the property the panels already rely on for `entitySyncStatus`.
 *
 * ## `""` means unknown, and that is the honest answer
 *
 * Both lookups return `""` for an id they do not have — a freshly created entity before the status refresh
 * lands, or a kind the engine does not map. Consumers already guard (`relPath ? … : undefined`), and the
 * alternative — computing a plausible path here — is exactly the mirror this item deleted. **A `""` is a
 * signal to the caller that the engine has not been asked yet, not a path.**
 */
export interface EntityPathLookup {
  /** The entity's path relative to the workspace root. `""` when the engine has not reported it. */
  relPathOf(id: string): string;
  /** The directory prefix a folder's entities live under, e.g. `requests/My Folder/`. `""` when unknown. */
  folderPrefixOf(folderId: string): string;
}

/** The two maps as the engine sends them, on `EntityStatusPayload.paths`. */
export interface EntityPathMaps {
  entities: Record<string, string>;
  folders: Record<string, string>;
}

const EMPTY: EntityPathLookup = {
  relPathOf: () => "",
  folderPrefixOf: () => "",
};

/**
 * Build the lookup from the engine's maps.
 *
 * **Exported because one caller cannot use the context.** `useEntityGitActions` is a hook called in
 * `App`'s own body — above any provider `App` renders — so a `useEntityPaths()` there would read the
 * *default* context. `App` therefore builds the lookup with this and passes it in as an argument. That is
 * two mechanisms for one fact, and it is deliberate: the alternative is splitting `App` into an outer
 * component that owns `useAppConfigSync` and an inner one that consumes the context, which is a refactor
 * of a ~500-line component for one lookup.
 */
export function entityPathLookup(paths?: EntityPathMaps): EntityPathLookup {
  return {
    relPathOf: (id: string) => paths?.entities[id] ?? "",
    folderPrefixOf: (folderId: string) => paths?.folders[folderId] ?? "",
  };
}

const EntityPathsContext = createContext<EntityPathLookup>(EMPTY);

export function EntityPathsProvider({
  paths,
  children,
}: {
  /** `undefined` — an engine older than this field — reads as "unknown", not as "no entities". */
  paths?: EntityPathMaps;
  children: React.ReactNode;
}) {
  const value = useMemo<EntityPathLookup>(() => entityPathLookup(paths), [paths]);
  return <EntityPathsContext.Provider value={value}>{children}</EntityPathsContext.Provider>;
}

/** The lookup. Outside a provider it answers `""` for everything, which every caller already handles. */
export function useEntityPaths(): EntityPathLookup {
  return useContext(EntityPathsContext);
}
