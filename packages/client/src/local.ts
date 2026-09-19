/**
 * The **client-local** half of `window.api` — the operations that must never cross the transport.
 *
 * ## Why this is an injected interface rather than an implementation
 *
 * Thirteen of the 144 keys are not RPC at all. They act on **the machine the client is running on**:
 * native file pickers, the OS trust store, Electron's `BrowserWindow`/`nativeTheme`, shell launch
 * state. On a remote engine they are meaningless at best and a hole at worst — `tlsInstallCA` mutates
 * the *client's* OS trust store, and routing it would let a server install a root certificate on a
 * user's machine.
 *
 * P6's Electron shell, P7's web UI and P8's CLI each answer these differently (a browser has no
 * native picker and no trust store), so `createClient()` takes them as a dependency rather than
 * guessing. That is also what makes the P5 acceptance criterion checkable: the client cannot
 * accidentally route one over the transport if it does not know how to.
 *
 * ## The four methods `BifurcApi` does not declare
 *
 * `isFirstLaunch`, `completeFirstLaunch`, `getZoomLevel` and `setZoomLevel` are exposed by
 * `src/preload.ts` and implemented in `src/ipc/handlers/clientHandlers.ts`, but absent from
 * `renderer/types/window.ts`. They are declared here, and `BifurcApiFull` (see `index.ts`) re-adds
 * them to the returned type — so the client satisfies the declared type *and* keeps the four keys
 * the renderer can actually call. See `surface.ts`'s header for the full argument.
 */

import type { BifurcApi } from "../../../renderer/types/window";

/** A file the client has read, in the shape the preload's `openFileDialog` already returns. */
export interface LocalFileContent {
  name: string;
  size: number;
  base64: string;
  mimeType: string;
}

/**
 * A file the client read **and where it read it from**.
 *
 * The path is the whole reason this type exists rather than `LocalFileContent`: `preflightImport`
 * has to hand the renderer something it can thread back into `importData`, and the only stable
 * handle for "the file the user already picked" is its path. `dialog:openFile` does not return one,
 * which is why the import pair could not be routed before.
 */
export interface ReadFileResult extends LocalFileContent {
  /** The local path the bytes came from. */
  filePath: string;
}

/** Where an artifact should go. `canceled` distinguishes "user said no" from "it failed". */
export interface ArtifactWriteResult {
  ok: boolean;
  filePath?: string;
  canceled?: boolean;
  error?: string;
}

/** A native file-type filter, in Electron's `FileFilter` shape. */
export interface FileFilterSpec {
  name: string;
  extensions: string[];
}

/**
 * What a save dialog should show.
 *
 * Deliberately *presentation only*: the client knows what it is exporting and the OS dialog does
 * not, so the client names it. A browser client (P7) ignores the filters and uses `defaultName` as
 * the download name — which is why they are here rather than buried in a shell handler.
 */
export interface SavePathOptions {
  /** Dialog title, e.g. `Export Workspace`. */
  title?: string;
  /** Pre-filled file name. */
  defaultName?: string;
  /** Filters, most specific first. */
  filters?: FileFilterSpec[];
}

/**
 * What an open dialog should show, and which file to read.
 *
 * `path` is the reuse case: the user already chose this file in `preflightImport`, so reading it
 * again must **not** ask again. `title`/`filters` are ignored when `path` is present.
 */
export interface ReadFileOptions {
  /** Reuse this path instead of showing a dialog. */
  path?: string;
  /** Dialog title, e.g. `Import Workspace`. Defaults to `Select File`. */
  title?: string;
  /** Filters, most specific first. Omitted means the OS shows every file. */
  filters?: FileFilterSpec[];
}

export interface ClientLocal {
  // ── the nine declared in BifurcApi ───────────────────────────────────────
  openExternal: BifurcApi["openExternal"];
  setTitleBarOverlay: NonNullable<BifurcApi["setTitleBarOverlay"]>;
  getTheme: NonNullable<BifurcApi["getTheme"]>;
  setTheme: NonNullable<BifurcApi["setTheme"]>;
  tlsInstallCA: BifurcApi["tlsInstallCA"];
  openFileDialog: BifurcApi["openFileDialog"];
  pickFilePath: BifurcApi["pickFilePath"];
  pickFolderPath: BifurcApi["pickFolderPath"];
  platform: BifurcApi["platform"];

  // ── the four BifurcApi under-declares ────────────────────────────────────
  /** `clientHandlers.ts`'s `app:isFirstLaunch` → `!settings.hasSeenWelcome`. */
  isFirstLaunch(): Promise<boolean>;
  /** `clientHandlers.ts`'s `app:completeFirstLaunch` → sets `hasSeenWelcome`, resolves `{ok:true}`. */
  completeFirstLaunch(): Promise<{ ok: boolean }>;
  /** `clientHandlers.ts`'s `zoom:get` → the persisted zoom level, defaulting to 0. */
  getZoomLevel(): Promise<number>;
  /** `clientHandlers.ts`'s `zoom:set` → clamps to [-5, 9] and persists. */
  setZoomLevel(level: number): Promise<{ ok: boolean }>;

  /**
   * Write an artifact the **engine produced** to disk.
   *
   * **Two forms, and the difference is *when* the dialog runs.**
   *
   * - **`destPath` omitted — ask and write.** The implementation shows the save dialog itself,
   *   pre-filled from `suggestedName` and filtered by `mimeType`. This is the form for artifacts
   *   whose dialog chrome comes from the *engine's own suggestion*, which by definition is not known
   *   until the command has run.
   * - **`destPath` given — write only.** Write the bytes there and show nothing; the user already
   *   chose, via `pickSavePath`, and asking twice is the bug this form exists to prevent.
   *
   * The first three arguments always describe the artifact, so a caller never has to know which form
   * it is using: `suggestedName` and `mimeType` are simply unused when `destPath` is present.
   *
   * Why both forms rather than one dialog-first path for everything: `File_Ops_Protocol.md` §3.2
   * requires the dialog before the command *because the command renders something expensive the user
   * might abandon* — a 200 MB workspace archive, a runner report. That applies to `exportData` and
   * `exportRunnerReport`, which is why those two pass `destPath`. The other egress methods
   * (`exportAudit`, `tlsExportCert`, `shareCaptureJson`) hand over bytes the engine has already
   * produced, so reordering them buys nothing and would cost the engine's own suggestion as the
   * dialog's default name — a change the user can see.
   *
   * **`contentBase64` is base64, not a decoded string.** The engine's artifacts are bytes —
   * `workspace-zip` is a ZIP, and a `.pem` is only accidentally text — and base64 is the one
   * representation that survives both. An earlier version of this hook took a decoded `string` and
   * the client got there by base64-decoding into utf-8, which silently corrupts any artifact that
   * is not valid utf-8: every byte outside the ASCII range becomes `U+FFFD` or worse. Taking
   * base64 means the client never has to decode at all, and the implementer writes
   * `Buffer.from(contentBase64, "base64")` (or the platform's equivalent) straight to disk.
   *
   * This is also why the hook is byte-oriented rather than text-oriented even though most artifacts
   * today happen to be JSON: the six callers share one primitive, and one of them being binary is
   * enough to make "always bytes" the only safe default.
   *
   * Optional so a client with no filesystem (P7's web UI, which downloads instead) can omit it —
   * the egress methods then fail with a clear error rather than silently resolving `{ok:false}`.
   */
  writeArtifact?(
    contentBase64: string,
    suggestedName: string,
    mimeType: string,
    destPath?: string,
  ): Promise<ArtifactWriteResult>;

  /**
   * Ask the user **where** an artifact should go, and return the path. Writes nothing.
   *
   * The hook that has to run *first*. `File_Ops_Protocol.md` §3.2 requires the dialog before the
   * command — fail fast on cancel, so the engine never renders a 200 MB workspace archive the user
   * then abandons — and the bundled `writeArtifact` cannot express that ordering, because it forces
   * the dialog *after* the render. Splitting the ask out is what lets `exportData` and
   * `exportRunnerReport` move off the shell's legacy channels.
   *
   * Returns `null` when the user cancels. A client with no filesystem (P7's web UI) omits it, or
   * returns a synthetic path; the egress methods then fail with a clear error rather than silently
   * resolving `{ok:false}`.
   */
  pickSavePath?(opts: SavePathOptions): Promise<string | null>;

  /**
   * Read a file for the import/upload paths — from `opts.path` if given, otherwise by asking.
   *
   * **Ask and read are one hook here, unlike the egress pair, and the ordering is why.** Ingress
   * runs *dialog → read → upload → command*, so the read is early and the two steps can share a
   * call. Egress runs *dialog → command → write*, so the write is last and cannot be bundled with
   * the ask without forcing the dialog after the render.
   *
   * The result carries `filePath` so the caller can hand it to the renderer and reuse it later —
   * that round trip is what stops `importData` opening a **second** dialog for a file the user
   * already picked. Returns `null` on cancel, or `{ error }` when the file cannot be read.
   *
   * Distinct from `openFileDialog` in intent and in limits: that one caps at 1 MB because it feeds
   * binary request bodies, whereas imports are capped by the protocol's own
   * `BLOB_MAX_INGRESS_BYTES`.
   */
  readArtifactFile?(opts?: ReadFileOptions): Promise<ReadFileResult | { error: string } | null>;
}

/**
 * The subset of `ClientLocal` that is **required** — everything except the three filesystem hooks.
 *
 * Split out so `createClient` can demand the cheap, universally-answerable ones (theme, zoom,
 * `platform`) while treating the filesystem ones as capability checks. A browser client satisfies
 * the former and omits the latter.
 */
export type RequiredClientLocal = Omit<
  ClientLocal,
  "pickSavePath" | "writeArtifact" | "readArtifactFile"
>;
