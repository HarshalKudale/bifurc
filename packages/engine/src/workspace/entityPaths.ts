/**
 * `entity id → relPath`, and `folder id → directory prefix` — the two maps the **renderer used to compute
 * for itself** (`plan/19` WI 5, item 3).
 *
 * ## What this replaces, and why it is engine work
 *
 * `packages/gui/src/lib/utils.ts` carried `sanitizeDirName`, `entityRelPath` and `flatEntityRelPath` — a
 * hand-mirror of `workspaceFs.ts` whose own docblock admitted it was "the engine's `sanitizeDirName` … its
 * mirror". It existed because the renderer needs a relPath to look an entity up in the engine's own status
 * map (`sync.getEntityStatus` returns `relPath → dirty marker`) and to name a file for the git commands.
 *
 * A path is a property of the **storage layout**, so it is engine work by `plan/19` D22's test: a terminal
 * client would need the same answer. This module is the engine answering it, once.
 *
 * ## Why the map rides the status payload rather than the config
 *
 * The obvious alternative is to attach `relPath` to each entity in `config.get`. **It is unsafe, and the
 * check is `config.save`:** `workspace/entities/configCommands.ts` hands the incoming config straight to
 * `saveConfig()`, which writes the entity files — so a computed field would be **persisted to disk**, and
 * the next load would read back a stale path. `sync.getEntityStatus` is a read-only *result*, never saved,
 * which is what makes it the right carrier.
 *
 * ## The kind table is the registry, not a second list
 *
 * `forEachCrudKind()` reports `configKey` / `folderConfigKey` / `isFlat` from the same registration the CRUD
 * commands dispatch through, so a new kind appears here for free. A hand-kept table would be the very
 * duplication this item exists to remove, one layer down.
 *
 * ## ⚠ Two limits, recorded rather than papered over
 *
 * 1. **It resolves the *active* workspace**, because it reads `loadConfig()`. `sync.getEntityStatus` takes a
 *    `workspaceId`, and on the desktop those are always the same one — but a host serving N workspaces would
 *    get the active workspace's paths for a non-active id. Fixing it means a per-workspace config reader
 *    (`readAllEntities(wsId, …)` covers entities but **not** folders, which live in `app.json`), which is a
 *    bigger change than this item.
 * 2. **Paths change only when the config changes** — create, delete, move, rename — and every one of those
 *    refreshes the config, which re-runs this. So the status *event* deliberately does **not** carry the
 *    maps: it is a dirty-marker broadcast, and adding them there would mean `runtime/entityStatus.ts`
 *    importing this module, which imports `entities/entityCrudCommands.ts`, which imports
 *    `runtime/entityStatus.ts` — a cycle for no gain.
 */
import { loadConfig } from "./config";
import { entityRelPath, flatEntityRelPath, sanitizeDirName } from "./workspaceFs";
import { forEachCrudKind } from "./entities/entityCrudCommands";

export interface EntityPathMaps {
  /** `entity id` → its path relative to the workspace root, e.g. `requests/My Folder/req_abc.json`. */
  entities: Record<string, string>;
  /** `folder id` → the directory prefix its entities live under, e.g. `requests/My Folder/`. */
  folders: Record<string, string>;
}

/** Build both maps from the active workspace's config. See the header for the two limits. */
export function loadEntityPathMaps(): EntityPathMaps {
  const cfg = loadConfig();
  const entities: Record<string, string> = {};
  const folders: Record<string, string> = {};

  forEachCrudKind((opts) => {
    const list = (cfg[opts.configKey] as { id: string; folderId?: string | null }[] | undefined) ?? [];
    const folderList = opts.folderConfigKey
      ? ((cfg[opts.folderConfigKey] as { id: string; name: string }[] | undefined) ?? [])
      : [];

    // Folder *names*, keyed by id: the path uses the sanitised **name**, not the id, and a folder can be
    // renamed without moving the entities that point at it.
    const folderName = new Map(folderList.map((f) => [f.id, f.name]));
    for (const f of folderList) folders[f.id] = `${opts.kind}/${sanitizeDirName(f.name)}/`;

    for (const e of list) {
      entities[e.id] = opts.isFlat
        ? flatEntityRelPath(opts.kind, e.id)
        : entityRelPath(opts.kind, e.id, e.folderId ? folderName.get(e.folderId) ?? null : null);
    }
  });

  return { entities, folders };
}
