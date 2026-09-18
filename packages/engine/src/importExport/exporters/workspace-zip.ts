import * as fs from "fs";
import { ZipArchive } from "archiver";
import { wsDir } from "../../store/workspaceFs";
import type { PathExportResult } from "../types";

/**
 * `workspace-zip` — the one exporter that cannot return content (`plan/04` work item 2, "the two
 * non-mechanical files").
 *
 * `archiver` pipes into a `WriteStream` and the archive is never held in memory, which is the whole
 * point: a workspace archive is the one artifact that can realistically run to hundreds of
 * megabytes. So this function is handed a **destination path** rather than returning a string, and
 * that path comes from the engine's own staging area (`blob/store.createStaging()`), never from the
 * client.
 *
 * The result therefore carries no content — `{ ok: true }` means "the bytes are at the path you gave
 * me", and `export.create` commits the staging handle to turn them into a blob.
 *
 * The old signature was `run(wsId, filePath)` where `filePath` was a path on the **user's** disk,
 * chosen by a dialog shown inside the engine. That is the arrangement `File_Ops_Protocol.md` §4
 * exists to remove; the parameter is still a path, but it is now an engine-local one.
 *
 * ## `archiver` 8: a default import that no longer exists
 *
 * This file used to open with `import archiver from "archiver"` and call
 * `archiver("zip", { zlib: { level: 6 } })`. That is the **v5–v7** API. `archiver` 8 is a pure ESM
 * package (`"type": "module"`, `"exports": "./index.js"`) with **no default export** — it exports
 * `Archiver`, `ZipArchive`, `TarArchive` and `JsonArchive` by name, and its README's quick start is
 * `new ZipArchive({ zlib: { level: 9 } })`.
 *
 * So the old line did not merely look wrong, it *was* wrong: `require("archiver").default` is
 * `undefined`, and `undefined(...)` throws `TypeError: archiver is not a function`. Nothing caught
 * it — `@types/archiver` was pinned at **7**, so the type declarations described the default-export
 * API and the compiler was satisfied, and no test ever exercised this file. Both halves are fixed
 * together: the import below, and `@types/archiver` bumped to `^8.0.0` in the root `package.json`
 * so the types can no longer agree with a call shape the runtime rejects.
 *
 * (`import * as path` was unused here before this change and has been dropped.)
 */
export async function run(wsId: string, destPath: string): Promise<PathExportResult> {
  return new Promise((resolve) => {
    const sourceDir = wsDir(wsId);
    if (!fs.existsSync(sourceDir)) {
      resolve({ ok: false, error: "Workspace directory not found" });
      return;
    }

    const output = fs.createWriteStream(destPath);
    const archive = new ZipArchive({ zlib: { level: 6 } });

    output.on("close", () => resolve({ ok: true }));
    archive.on("error", (err) => resolve({ ok: false, error: err.message }));

    archive.pipe(output);

    // Add all workspace files, skipping .git
    archive.glob("**/*", {
      cwd: sourceDir,
      dot: true,
      ignore: [".git/**", ".git"],
    });

    archive.finalize();
  });
}
