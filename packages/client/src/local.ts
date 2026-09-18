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

/** Where an artifact should go. `canceled` distinguishes "user said no" from "it failed". */
export interface ArtifactWriteResult {
  ok: boolean;
  filePath?: string;
  canceled?: boolean;
  error?: string;
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
   * Write an artifact the **engine produced** to a location the **user** chooses.
   *
   * This is the client half of every egress method: `exportData`, `preflightImport`'s file write,
   * `tlsExportCert`, `exportAudit`, `exportRunnerReport`, `shareCaptureJson`. The engine returns
   * content or a blob reference; only the client can put it on a disk. `suggestedName` is the
   * engine's, and is what the picker should pre-fill.
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
   * the egress methods then reject with a clear error rather than silently resolving `{ok:false}`.
   */
  writeArtifact?(contentBase64: string, suggestedName: string, mimeType: string): Promise<ArtifactWriteResult>;

  /**
   * Ask the user for a file and return its **content**, for the import/upload paths.
   * Distinct from `openFileDialog` only in intent; the preload exposes one method for both.
   */
  readArtifactFile?(): Promise<LocalFileContent | { error: string } | null>;
}

/**
 * The subset of `ClientLocal` that is **required** — everything except the two filesystem hooks.
 *
 * Split out so `createClient` can demand the cheap, universally-answerable ones (theme, zoom,
 * `platform`) while treating the filesystem ones as capability checks. A browser client satisfies
 * the former and omits the latter.
 */
export type RequiredClientLocal = Omit<ClientLocal, "writeArtifact" | "readArtifactFile">;
