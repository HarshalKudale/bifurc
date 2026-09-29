import { getGit } from "../workspace/gitStore";
import { getPendingDeletions, ALL_ENTITY_KINDS } from "../workspace/workspaceFs";
import {
  setCachedStatus,
  getCachedStatus,
  invalidateCache,
  type EntitySyncStatus,
} from "../workspace/statusCache";

/**
 * The per-workspace status cache moved **down** into `workspace/statusCache.ts` in engine-layout
 * phase 7. It is re-exported here (and `invalidateCache` below) so this module's existing callers —
 * `gitOps.ts` and `syncCommands.ts`, both `gitops`-side — keep importing it from where they always
 * have. The move exists so `workspace/`-side writers (the generic CRUD commands, and the `healthbar.*`
 * commands phase 7 relocated) can invalidate it without a `workspace → gitops` import, which
 * `Engine_Module_Plan.md` §5.1/§6 forbids. See `workspace/statusCache.ts` for the full rationale.
 */
export { invalidateCache, type EntitySyncStatus };

function isEntityPath(p: string): boolean {
  const base = p.split(/[/\\]/).pop();
  if (!base || !base.endsWith(".json")) return false;
  const stem = base.slice(0, -5);
  return stem !== "index" && stem !== "enabled" && stem !== "names" && stem !== "pending-deletions";
}

/**
 * Git uses C-string quoting (wraps in double-quotes, escapes with backslash) for paths that
 * contain spaces, non-ASCII chars, etc.  Strip it so paths match what we compute in the renderer.
 *
 * Exported because any code that parses non-`-z` porcelain output hits the same problem:
 * a path like `mocks/Solo Folder/sf-1.json` arrives as `"mocks/Solo Folder/sf-1.json"`, and
 * comparing it against an unquoted path silently never matches.
 */
export function unquoteGitPath(p: string): string {
  if (p.startsWith('"') && p.endsWith('"')) {
    // Unescape standard C escape sequences git uses: \t \n \\ \"  and \NNN (octal)
    return p.slice(1, -1).replace(/\\(["\\tnr]|[0-7]{1,3})/g, (_, esc: string) => {
      if (esc === '"')  return '"';
      if (esc === '\\') return '\\';
      if (esc === 't')  return '\t';
      if (esc === 'n')  return '\n';
      if (esc === 'r')  return '\r';
      // octal byte — rare in file names but handle it
      return String.fromCharCode(parseInt(esc, 8));
    });
  }
  return p;
}

export async function getWorkspaceSyncStatus(wsId: string): Promise<Record<string, EntitySyncStatus>> {
  try {
    const g = getGit(wsId);
    // Use -z (NUL-terminated) so paths with spaces/special chars are not C-quoted
    const raw = await g.raw(["status", "--porcelain", "-z", "--untracked-files=all"]);
    const result: Record<string, EntitySyncStatus> = {};

    // -z output: each entry is "<XY> <path>\0" (rename: "<XY> <new>\0<old>\0")
    const entries = raw.split("\0");
    let i = 0;
    while (i < entries.length) {
      const entry = entries[i++];
      if (entry.length < 4) continue;
      const xy = entry.slice(0, 2);
      const filePath = entry.slice(3);  // no quotes, no escaping with -z

      // For renames (R/C), the next NUL-separated token is the original path — skip it
      if ((xy[0] === "R" || xy[0] === "C") && i < entries.length) {
        i++; // skip old path
      }

      const relPath = filePath.replace(/\\/g, "/");
      if (!isEntityPath(relPath)) continue;

      // XY codes: ?? = untracked, D = deleted, else = modified/added/renamed
      if (xy === "??") {
        result[relPath] = "new";
      } else if (xy[0] === "D" || xy[1] === "D") {
        result[relPath] = "deleted";
      } else {
        result[relPath] = "modified";
      }
    }

    // Mark all tracked files that aren't in git status output as "clean"
    // ls-files also uses C-quoting without -z, so use -z here too
    const lsRaw = await g.raw(["ls-files", "-z"]);
    for (const entry of lsRaw.split("\0")) {
      if (!entry) continue;
      const relPath = entry.replace(/\\/g, "/");
      if (isEntityPath(relPath) && !result[relPath]) result[relPath] = "clean";
    }

    // Also mark pending-deletion entities as "deleted" even if git doesn't track the deletion
    // (e.g. entity was never committed — file was untracked, now gone, not visible in git status).
    //
    // Driven by `ALL_ENTITY_KINDS` rather than a local list. It used to name only
    // `requests`/`mocks`/`sockets`, so a never-committed `rule`/`webhook`/`graphqlRequest`/
    // `mapping`/… deleted through the UI had a pending-deletion record that nothing ever read —
    // the stub stayed in the tree but its status was `undefined`, which the renderer reads as
    // "not deleted" and therefore renders as an ordinary, clickable row pointing at a file that is
    // gone. See `ALL_ENTITY_KINDS` for the same defect at the other three call sites.
    for (const kind of ALL_ENTITY_KINDS) {
      for (const p of getPendingDeletions(wsId, kind)) {
        const rootPath = `${kind}/${p.id}.json`;
        if (!result[rootPath]) {
          const hasFolderEntry = Object.keys(result).some((k) => k.endsWith(`/${p.id}.json`) && k !== rootPath);
          if (!hasFolderEntry) result[rootPath] = "deleted";
        }
      }
    }

    setCachedStatus(wsId, result);
    return result;
  } catch {
    return getCachedStatus(wsId);
  }
}
