/**
 * Client-side shims for the import/export round-trip tests.
 *
 * WHY THIS EXISTS
 * ---------------
 * P3 work item 2 changed the import/export interface from **paths to content**: an exporter now
 * returns `{ content, suggestedName }` and an importer consumes `{ content, filename }`
 * (`File_Ops_Protocol.md` §2.1). The format-level tests still want to round-trip through a real
 * file — a real file on disk is exactly what a client writes — so these helpers are the two halves
 * of that seam and nothing else:
 *
 *   `exportToFile()`   — run the exporter, then write its content to disk. This is what
 *                        `src/ipc/importExportHandlers.ts` does after `export.create` returns
 *                        inline bytes, minus the blob branch.
 *   `preflightFile()` / `importFile()` — read a file back into an `ImportSource` and hand it to the
 *                        importer. This is what the shell does after `blob.put` has staged the
 *                        upload, minus the upload.
 *
 * **They are deliberately not a reimplementation of the blob path.** That path is exercised for
 * real, against the real command layer and the real store, by
 * `tests/integration/importExportBlob.integration.test.ts` (`export.create` → `blob.put` →
 * `import.preflight` → `import.commit`). Keeping the format-level tests on plain files means a
 * failure there points at a format codec rather than at the transport, which is the whole reason
 * the two suites are separate.
 *
 * The one thing these helpers *do* reproduce faithfully is the `filename`: under a blob an importer
 * never sees a path, only the client-supplied name. Two importers depend on it
 * (`environments-dotenv` names the new environment after it; `requests-openapi` picks YAML vs JSON
 * by its extension), so the tests must supply it the same way the client does — as a bare basename.
 *
 * Taking the exporter/importer **object** rather than a `(kind, format)` pair is deliberate: the
 * format-level tests already hold that object from `getExporter()`/`getImporter()` and assert on
 * it, so wrapping the object keeps the conversion at the call site purely mechanical and leaves
 * each test's choice of format stated exactly once.
 */

import * as fs from "fs";
import * as path from "path";
import type { ExporterFn, PathExporterFn } from "@bifurc/engine/importExport/registry";
import type {
    CollisionStrategy,
    ExportResult,
    ImportResult,
    ImportSource,
    PathExportResult,
    PreflightResult,
} from "@bifurc/engine/importExport/types";

/** An `ImportSource` built the way the client builds one: content plus a basename, never a path. */
export function sourceFromFile(file: string): ImportSource {
    return { content: fs.readFileSync(file, "utf-8"), filename: path.basename(file) };
}

/**
 * Run a **content-shaped** exporter and write its artifact to `destFile`.
 *
 * The returned `ExportResult` is the exporter's own, unmodified, so a test can still assert on
 * `.ok` / `.error` — only the side effect (the file now exists) is added. On a failed export
 * nothing is written, so `fs.existsSync(destFile)` stays a meaningful assertion.
 */
export async function exportToFile(
    wsId: string,
    exporter: ExporterFn,
    destFile: string,
): Promise<ExportResult> {
    const res = await exporter.run(wsId);
    if (res.ok) fs.writeFileSync(destFile, res.content, "utf-8");
    return res;
}

/**
 * Run a **path-shaped** exporter (`workspace-zip` only) into `destFile`.
 *
 * The path is engine-local in production — `export.create` hands the exporter a staging handle —
 * but a file in a temp directory serves the same purpose here, and where the archive lands is not
 * what this pair is being tested for.
 */
export async function exportPathToFile(
    wsId: string,
    exporter: PathExporterFn,
    destFile: string,
): Promise<PathExportResult> {
    return exporter.run(wsId, destFile);
}

/** Preflight a file the way the client does: read it, then hand the importer content + filename. */
export function preflightFile(
    importer: { preflight(wsId: string, source: ImportSource): PreflightResult },
    wsId: string,
    file: string,
): PreflightResult {
    return importer.preflight(wsId, sourceFromFile(file));
}

/** Import a file the way the client does, after `blob.put` has staged the upload. */
export function importFile(
    importer: { run(wsId: string, source: ImportSource, strategy: CollisionStrategy): Promise<ImportResult> },
    wsId: string,
    file: string,
    strategy: CollisionStrategy,
): Promise<ImportResult> {
    return importer.run(wsId, sourceFromFile(file), strategy);
}
