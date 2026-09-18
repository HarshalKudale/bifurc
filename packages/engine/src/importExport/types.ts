/**
 * P3 work item 2 — the import/export interface, converted from **paths to content**.
 *
 * `File_Ops_Protocol.md` §2.1 is the finding this conversion rests on: `filePath` was a pure
 * transport detail. Every exporter built a string and wrote it to that path; every importer read a
 * string back from it. Nothing about the path mattered except that both sides agreed on it — which
 * they cannot do once the engine is a separate process (P4), and cannot do at all in Docker (P9),
 * where the client's path does not exist on the engine's filesystem.
 *
 * The replacement is the rendered content plus a **client-supplied `filename`**. The engine must
 * never derive a display name from a path: under a remote engine that either produces garbage or
 * leaks a fragment of the user's local filesystem into an entity name (`File_Ops_Protocol.md` §8).
 *
 * ## The two exceptions
 *
 * `workspace-zip` (both directions) is genuinely stream-based — `archiver` pipes into a
 * `WriteStream` and `unzipper.Open.file()` will not accept a buffer — so it keeps a path-based
 * signature. Those two live behind `PathExporterFn` / `PathImporterFn` in `./registry` rather than
 * being forced into a shape they cannot honour. The paths involved are engine-local staging paths
 * and never cross the wire.
 *
 * ## Why the results are discriminated unions
 *
 * `ok` is the discriminant rather than an optional-field bag, so a caller that has checked `ok` gets
 * `content` / `itemCount` without a non-null assertion. The previous shape made every one of those
 * fields optional, which meant `{ ok: true }` alone type-checked — the compiler could not tell a
 * complete result from a truncated one.
 */

export type EntityKind =
  | "workspace"
  | "requests"
  | "mocks"
  | "environments"
  | "mappings"
  | "proxyRules"
  | "websockets"
  | "webhooks";

export type CollisionStrategy = "keep" | "override" | "new";

export interface FormatDefinition {
  id: string;
  label: string;
  /** File extensions accepted/produced (without leading dot) */
  extensions: string[];
  supportsExport: boolean;
  supportsImport: boolean;
}

// ─────────────────────────────────────────────────────────────────────────────
// Export — the engine renders, the client saves
// ─────────────────────────────────────────────────────────────────────────────

/**
 * An exporter's output. It no longer names a destination: the engine decides inline-vs-blob
 * (`BLOB_INLINE_THRESHOLD_BYTES`) and the client decides where the bytes land on its own disk.
 */
export type ExportResult =
  | {
      ok: true;
      /** The rendered artifact. */
      content: string;
      /** Default name for the client's save dialog. Never derived from a path. */
      suggestedName: string;
    }
  | { ok: false; error: string };

/**
 * The result of a **path-shaped** export (`workspace-zip` only).
 *
 * It carries no content because there is none to carry: the exporter streamed the archive into
 * the destination path it was handed. `{ ok: true }` means "the bytes are at the path you gave
 * me", and the caller commits its staging handle. See `PathExporterFn` in `./registry`.
 */
export type PathExportResult = { ok: true } | { ok: false; error: string };

// ─────────────────────────────────────────────────────────────────────────────
// Import — the client uploads, the engine reads
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Where an importer reads from. The bytes arrive already decoded, so an importer never touches the
 * filesystem — which is what makes the same code correct for a local and a remote engine.
 *
 * `filename` is not decoration. Two importers genuinely need it:
 *
 *  - `importers/environments-dotenv.ts` derives the new environment's *name* from it. It used to
 *    call `filePath.split(/[/\\]/).pop()`, which under a remote engine would name the environment
 *    after a fragment of a path that does not exist engine-side.
 *  - `importers/requests-openapi.ts` picks YAML or JSON by extension. A staged blob has no
 *    extension, so this would silently always take the JSON branch and fail on every YAML spec.
 */
export interface ImportSource {
  content: string;
  filename: string;
}

/**
 * Preflight is a dry run: how many items the file holds, and which of them already exist. It
 * deliberately reports no path — the client already knows where its own file is, and echoing a
 * path back is the leak `File_Ops_Protocol.md` §8 lists first.
 */
export type PreflightResult =
  | { ok: true; itemCount: number; collisionIds: string[] }
  | { ok: false; error: string };

export type ImportResult =
  | { ok: true; imported: number; skipped?: number }
  | { ok: false; error: string };

export interface FormatsMap {
  [kind: string]: FormatDefinition[];
}
