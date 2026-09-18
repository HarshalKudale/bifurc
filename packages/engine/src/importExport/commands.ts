/**
 * P3 work items 3–4 — `export.create` / `import.preflight` / `import.commit`, bound to the blob
 * store.
 *
 * This is where the egress and ingress flows of `File_Ops_Protocol.md` §4–5 actually happen. The
 * engine renders an artifact and hands back a reference (inline or blob); the client uploads bytes
 * once and the engine reads that one staged copy twice (preflight, then commit) instead of the old
 * model where `preflight` read the file and `run` read it again from the same path.
 *
 * ## What this module deliberately does not do
 *
 * **It never shows a dialog and never writes to the user's filesystem.** That is the whole point of
 * the split: `File_Ops_Protocol.md` §1 says the engine owns its data dir and generated artifacts,
 * and the client owns the user's machine. The old `importExport:export` handler called
 * `dialog.showSaveDialog` and passed the resulting path straight to the exporter; the dialog now
 * belongs to the client, which is `src/ipc/importExportHandlers.ts` in the Electron shell.
 *
 * **It does not release the blob after commit.** `File_Ops_Protocol.md` §5 says imports are
 * transform-and-discard and that the *client* calls `blob.release`. Auto-releasing here would break
 * the retry path — the user resolves collisions between preflight and commit, and a failed commit
 * must be retryable without re-uploading. `BLOB_TTL_MS` plus the sweep is the safety net for a
 * client that forgets.
 *
 * ## Why a function and not an import side effect
 *
 * Same reason as `blob/commands.ts`: `createEngine()` registers **no** commands, and that is a
 * documented, tested invariant. The consumer registers what it is willing to serve. Note that this
 * module *does* register `export.formats` — the shell used to register that one itself, and it
 * moved here so all four `export.*`/`import.*` commands have a single owner and a single call site.
 */
import * as fs from "fs";
import {
  type ExportCreateParams,
  type ExportCreateResult,
  type ImportCommitParams,
  type ImportCommitResult,
  type ImportPreflightParams,
  type ImportPreflightResult,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";
import {
  blobContentPath,
  createStaging,
  describeBlobError,
  statBlob,
} from "../blob/store";
// The inline-vs-blob decision is shared with the five other egress artifact commands
// (`fileOps/commands.ts`) rather than owned here — `File_Ops_Protocol.md` §4 gives all six the same
// contract, so they must all make the same choice the same way.
import { publishArtifact } from "../blob/publish";
import { getAllFormats, getEntry, type FormatEntry } from "./registry";
import type { CollisionStrategy, EntityKind } from "./types";

// ─────────────────────────────────────────────────────────────────────────────
// Registration
// ─────────────────────────────────────────────────────────────────────────────

export function registerImportExportCommands(registry: CommandRegistry): void {
  registry.register("export.formats", () => getAllFormats());
  registry.register("export.create", (params: ExportCreateParams) => exportCreate(params));
  registry.register("import.preflight", (params: ImportPreflightParams) => importPreflight(params));
  registry.register("import.commit", (params: ImportCommitParams) => importCommit(params));
}

// ─────────────────────────────────────────────────────────────────────────────
// export.create — egress
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Render a workspace's entities and return either the bytes inline or a blob reference.
 *
 * The inline/blob decision is the engine's, not the client's, and it is made in exactly one place:
 * here. Below `BLOB_INLINE_THRESHOLD_BYTES` the artifact never touches the store, so the common case
 * — a small JSON export — is one round-trip. Above it, the artifact is written to a blob and the
 * client pulls it with `blob.read`.
 */
async function exportCreate(params: ExportCreateParams): Promise<ExportCreateResult> {
  const entry = getEntry(params.kind as EntityKind, params.format);
  if (!entry) {
    return { ok: false, error: `No export format "${params.format}" for kind "${params.kind}".` };
  }

  const mimeType = mimeTypeFor(entry);

  try {
    if (entry.exporter) {
      const res = await entry.exporter.run(params.workspaceId);
      if (!res.ok) return { ok: false, error: res.error };
      // The client's filename wins when it supplied one: it is what the user typed into the save
      // dialog. The exporter's `suggestedName` is the engine's default, for a client that does not
      // show a native dialog (P7's web UI downloads instead).
      return publishArtifact(res.content, params.filename || res.suggestedName, mimeType);
    }

    if (entry.pathExporter) {
      return await publishStaged(entry, params, mimeType);
    }

    return { ok: false, error: `Format "${params.format}" cannot be exported.` };
  } catch (err) {
    return { ok: false, error: describeBlobError(err) };
  }
}

/**
 * Stream-shaped (`workspace-zip` only): the exporter pipes into a destination, so the engine has to
 * hand it one first.
 *
 * A staging handle is created up front and **always** settled — `commit()` on success, `discard()`
 * on any failure. An unsettled handle would leave a partial archive in `<blobs>/.staging/` until the
 * sweep noticed it, and the sweep is a safety net rather than a cleanup strategy.
 *
 * Note this path never returns inline, even for an archive small enough to qualify. Staging exists
 * because the producer cannot hand over a buffer, and reading the staged file back to satisfy the
 * inline branch would undo the reason it streamed in the first place. A workspace zip is the one
 * artifact where the extra `blob.read` round-trip is not worth avoiding.
 */
async function publishStaged(
  entry: FormatEntry,
  params: ExportCreateParams,
  mimeType: string,
): Promise<ExportCreateResult> {
  const pathExporter = entry.pathExporter!;
  const staging = createStaging();

  try {
    const res = await pathExporter.run(params.workspaceId, staging.path);
    if (!res.ok) {
      staging.discard();
      return { ok: false, error: res.error };
    }

    // Measured before `commit()`, which renames the file out of staging and so cannot be stat-ed
    // afterwards without a second lookup.
    const size = fs.statSync(staging.path).size;
    const suggestedName = params.filename || defaultNameFor(params, entry);
    const { blobId, sha256 } = staging.commit({ filename: suggestedName, mimeType });

    return { ok: true, blobId, suggestedName, size, mimeType, sha256 };
  } catch (err) {
    // `commit()` sets its own settled flag before it can throw, so this is a no-op on that path and
    // the real cleanup on every other one.
    staging.discard();
    throw err;
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// import.preflight / import.commit — ingress
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Read the uploaded blob and report what is in it, without changing anything.
 *
 * This and `importCommit` are the two halves of the "upload once, reference twice" property. Both
 * resolve the same `blobId`, so the client's file is transferred exactly once no matter how many
 * times the user changes their mind about the collision strategy.
 */
function importPreflight(params: ImportPreflightParams): ImportPreflightResult {
  const entry = getEntry(params.kind as EntityKind, params.format);
  if (!entry) {
    return { ok: false, error: `No import format "${params.format}" for kind "${params.kind}".` };
  }

  try {
    // `statBlob` is what supplies the client's filename, which the content-shaped importers need
    // (dotenv names the new environment after it; OpenAPI picks YAML vs JSON by its extension).
    // It also gives a clean `blob-not-found` for an id that was released or swept.
    const meta = statBlob(params.blobId);

    if (entry.importer) {
      return entry.importer.preflight(params.workspaceId, {
        content: readBlobText(params.blobId),
        filename: meta.filename,
      });
    }

    if (entry.pathImporter) {
      return entry.pathImporter.preflight(params.workspaceId, blobContentPath(params.blobId));
    }

    return { ok: false, error: `Format "${params.format}" cannot be imported.` };
  } catch (err) {
    return { ok: false, error: describeBlobError(err) };
  }
}

/**
 * Apply the import for real.
 *
 * `collisionStrategy` defaults to `"keep"` — the non-destructive choice, and the one the protocol
 * schema's doc comment says a client that has not yet asked the user should send.
 */
async function importCommit(params: ImportCommitParams): Promise<ImportCommitResult> {
  const entry = getEntry(params.kind as EntityKind, params.format);
  if (!entry) {
    return { ok: false, error: `No import format "${params.format}" for kind "${params.kind}".` };
  }

  const strategy: CollisionStrategy = params.collisionStrategy ?? "keep";

  try {
    const meta = statBlob(params.blobId);

    if (entry.importer) {
      return await entry.importer.run(
        params.workspaceId,
        { content: readBlobText(params.blobId), filename: meta.filename },
        strategy,
      );
    }

    if (entry.pathImporter) {
      return await entry.pathImporter.run(
        params.workspaceId,
        blobContentPath(params.blobId),
        strategy,
      );
    }

    return { ok: false, error: `Format "${params.format}" cannot be imported.` };
  } catch (err) {
    return { ok: false, error: describeBlobError(err) };
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Read a blob's bytes as UTF-8, for an in-process importer.
 *
 * Deliberately `readFileSync` on the blob's content file rather than `blob.read`. That primitive is
 * the *transport* read: it chunks at `BLOB_READ_CHUNK_BYTES`, base64-encodes each slice and slides
 * the lease. Doing that here would mean encoding a 5 MB HAR to base64 in 512 KB pieces and decoding
 * it again one stack frame later. The trade-off is that this read does not refresh the lease, which
 * is noted on `blobContentPath`.
 */
function readBlobText(blobId: string): string {
  return fs.readFileSync(blobContentPath(blobId), "utf-8");
}

/** `<kind-kebab>-export.<first extension>`. Mirrors the default the shell's save dialog used to
 *  build, so a client that supplies no filename gets the same name a user saw before P3. */
function defaultNameFor(params: ExportCreateParams, entry: FormatEntry): string {
  const kindKebab = params.kind.replace(/([A-Z])/g, "-$1").toLowerCase();
  return `${kindKebab}-export.${entry.definition.extensions[0] ?? "bin"}`;
}

/**
 * The MIME type recorded in blob metadata and returned to the client.
 *
 * Derived from the format's first extension rather than added to `FormatDefinition`, because
 * `export.formats` is a pure-data command whose result shape is mirrored by hand in the renderer —
 * a field only the egress path needs does not belong on it. The set is closed: these are the
 * extensions `registry.ts` registers, and nothing else can reach here.
 */
const MIME_BY_EXTENSION: Record<string, string> = {
  json: "application/json",
  har: "application/json",
  zip: "application/zip",
  yaml: "application/yaml",
  yml: "application/yaml",
  env: "text/plain",
  txt: "text/plain",
  sh: "text/plain",
  curl: "text/plain",
};

function mimeTypeFor(entry: FormatEntry): string {
  return MIME_BY_EXTENSION[entry.definition.extensions[0] ?? ""] ?? "application/octet-stream";
}
