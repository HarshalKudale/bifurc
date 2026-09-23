/**
 * `createClient(transport, { local }) → BifurcApiFull`.
 *
 * ## The shape of this file, and why
 *
 * Every method is a **hand-written typed one-liner** whose *command* comes from `SURFACE`. That
 * split is deliberate:
 *
 * - the inventory (`surface.ts`) owns **which command** a key maps to, so the mapping cannot drift
 *   from the classification, and `assertSurfaceIsTotal()` keeps the classification honest;
 * - the one-liner owns **the payload**, because `window.api` is positional while commands take
 *   objects, and no data table can express `(wsId, kind, id) → {workspaceId, kind, id}` with type
 *   safety. The plan's "generate, don't hand-write" is satisfied where it can be — the command
 *   resolution, the classification, the key diff — and abandoned where it cannot.
 *
 * The return type is annotated as `BifurcApiFull`, so **the compiler is the shape test**: a missing
 * method or a drifted signature fails `npm run typecheck`. `tests/shape.test-d.ts` adds the
 * assignability assertion against `BifurcApi` proper.
 *
 * ## `BifurcApiFull` — why the return type is not just `BifurcApi`
 *
 * `packages/gui/src/types/window.ts` disagrees with the runtime in **two** directions, on **seven** keys in
 * total, and both directions are silent failures if left alone:
 *
 * - **Narrower — four keys missing.** `isFirstLaunch`, `completeFirstLaunch`, `getZoomLevel`,
 *   `setZoomLevel` are exposed by `src/preload.ts` and implemented in `clientHandlers.ts` but absent
 *   from the declared type. Returning plain `BifurcApi` would make them excess properties in the
 *   object literal — a compile error — and dropping them would silently shrink the runtime surface,
 *   which is exactly the failure P5 exists to prevent.
 * - **Looser — three keys optional.** `setTitleBarOverlay`, `getTheme` and `setTheme` are declared
 *   `?` although `src/preload.ts` always provides them, so a client that omitted one would satisfy
 *   `BifurcApi` and then fail the first time the renderer called it.
 *
 * So the seven are corrected here: the four added, the three tightened. `BifurcApiFull` remains
 * assignable to `BifurcApi`, so the declared contract still holds — and `tests/shape.test-d.ts`
 * asserts the difference is *exactly* those seven keys, so the gap cannot grow unnoticed.
 */

import type { Transport, EngineEvent } from "@bifurc/engine/transport/types";
import type { BifurcApi } from "../../gui/src/types/window";
// A **runtime** import, unlike everything else this file takes from the protocol: the enum is the
// single source for the `invalid_kind` guard below, so the client does not restate the six kinds.
import { EntitySetEnabledParams } from "@bifurc/protocol";
import { SURFACE } from "./surface";
import { createSubscriptionHub } from "./subscriptions";
import { DEFAULT_RETRY_POLICY, withRetry, type RetryPolicy } from "./retry";
import type { ArtifactWriteResult, ClientLocal, ReadFileOptions } from "./local";

/**
 * The three keys `packages/gui/src/types/window.ts` marks **optional** (`?`) although `src/preload.ts`
 * always provides them: `setTitleBarOverlay`, `getTheme`, `setTheme`.
 *
 * This is the *same* defect as the four missing keys below, pointing the other way — there the
 * declared type is narrower than the runtime, here it is **looser**. The consequence is identical
 * and just as quiet: a client that simply omitted `getTheme` would satisfy `BifurcApi`, pass the
 * type-level shape test, and then fail the first time the renderer called it. Optionality is the
 * more insidious of the two, because "possibly undefined" reads as defensive typing rather than as
 * a hole in the contract.
 *
 * Tightened to required in `BifurcApiFull`. The signatures are *not* restated — the mapped type
 * below reads them out of `BifurcApi`, so they cannot drift from `packages/gui/src/types/window.ts`.
 */
type OptionalInDeclaredType = "setTitleBarOverlay" | "getTheme" | "setTheme";

/**
 * Exported because `tests/shape.test-d.ts` must assert over *exactly* this set: the spike's
 * direction-2 "no signature drift" check is only valid once these three keys are excluded, since
 * `BifurcApiFull` intentionally makes them stricter than `BifurcApi`. A local copy in the test would
 * drift the moment this list changed, and the drift would silently disable the check.
 */
export type { OptionalInDeclaredType };

/** `BifurcApi` with the three loose keys tightened and the four missing keys added. */
export type BifurcApiFull = BifurcApi &
  { [K in OptionalInDeclaredType]-?: NonNullable<BifurcApi[K]> } & {
    isFirstLaunch(): Promise<boolean>;
    completeFirstLaunch(): Promise<{ ok: boolean }>;
    getZoomLevel(): Promise<number>;
    setZoomLevel(level: number): Promise<{ ok: boolean }>;
  };

export interface BifurcClient extends BifurcApiFull {
  /** Tear down every subscription this client opened. Idempotent. */
  close(): Promise<void>;
  /**
   * The highest event `seq` this client has seen, to be sent as `lastSeq` on a reconnect's `hello`.
   *
   * Declared here and made **non-enumerable** below, exactly like `close` and for the same reason:
   * `Object.keys(client)` must equal `window.api`'s keys, because that equality is the plan's own
   * regression signal for the whole critical path. The shell and the browser host hold this object
   * directly; the renderer only ever sees what `contextBridge` copies, and that walk follows enumerable
   * own properties — so the value is reachable by the two callers that reconnect and invisible to the UI.
   *
   * `0` means "I have seen nothing", which the schema accepts and which the engine reads as a first
   * connection. See `subscriptions.ts` for why the maximum is the right number rather than a count.
   */
  lastSeq(): number;
}

export interface CreateClientOptions {
  /**
   * The client-local half. Required: there is no sensible default for a native file picker or an OS
   * trust store, and a default that silently did nothing would look like a working install.
   */
  local: ClientLocal;
  /**
   * Overrides for the retry policy (`./retry.ts`). Optional because the defaults are the shipped
   * behaviour; present mainly so tests can drive the loop without sleeping, and so a caller that
   * wants no retrying at all can pass `{ attempts: 1 }`.
   */
  retry?: Partial<RetryPolicy>;
}

/**
 * The argument `onLogEntry`'s callback receives.
 *
 * Note the doubled `Parameters<>`: `BifurcApi["onLogEntry"]` is a *function that takes a callback*,
 * so the first `[0]` yields the callback and only the second yields the entry it is called with.
 */
type LogEntry = Parameters<Parameters<BifurcApi["onLogEntry"]>[0]>[0];

// ── helpers ────────────────────────────────────────────────────────────────

/** The command a key maps to, resolved through the inventory so the two cannot disagree. */
function commandFor(key: string): string {
  const entry = SURFACE[key];
  if (!entry) throw new Error(`@bifurc/client: no surface entry for "${key}"`);
  switch (entry.kind) {
    case "transport":
    case "shim":
      return entry.command;
    case "entity":
      return `entity.${entry.op}`;
    default:
      throw new Error(
        `@bifurc/client: "${key}" is a ${entry.kind} key and has no transport command — ` +
          `routing it over the transport would be a bug, not a missing case.`,
      );
  }
}

/**
 * Base64 ↔ bytes without assuming Node.
 *
 * The client has to bundle for P7's web UI, so `Buffer` is not guaranteed. `atob`/`btoa` are the
 * browser primitives; `Buffer` is the Node one. Both are reached through `globalThis` so neither
 * is a hard reference at module scope.
 *
 * **These are byte-oriented on purpose, and the earlier `decodeBase64` was not.** That one returned
 * a UTF-8 *string*, which is lossy the moment an artifact is not text: `workspace-zip` is a ZIP, and
 * decoding its bytes as UTF-8 replaces every non-ASCII byte with `U+FFFD` — a corrupt export that
 * still looks like a file. Artifacts are bytes, so the client's egress path handles bytes and only
 * re-encodes to base64 at the boundary the `writeArtifact` hook declares.
 */
function base64ToBytes(b64: string): Uint8Array {
  const g = globalThis as unknown as {
    atob?: (s: string) => string;
    Buffer?: { from(s: string, e: string): Uint8Array };
  };
  if (g.Buffer) return new Uint8Array(g.Buffer.from(b64, "base64"));
  if (typeof g.atob === "function") {
    const binary = g.atob(b64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }
  throw new Error("@bifurc/client: no base64 decoder available in this environment");
}

function bytesToBase64(bytes: Uint8Array): string {
  const g = globalThis as unknown as {
    btoa?: (s: string) => string;
    Buffer?: { from(b: Uint8Array): { toString(e: string): string } };
  };
  if (g.Buffer) return g.Buffer.from(bytes).toString("base64");
  if (typeof g.btoa === "function") {
    let binary = "";
    // Chunked: `String.fromCharCode(...bytes)` blows the argument limit on a large blob.
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return g.btoa(binary);
  }
  throw new Error("@bifurc/client: no base64 encoder available in this environment");
}

/**
 * The two `window.api` kinds whose name the **renderer** spells the engine's way.
 *
 * `EntityKind` — the wire vocabulary — renames two of the twelve CRUD kinds: the engine stores a
 * proxy rule under `"rules"` and a websocket connection under `"sockets"`, while the protocol calls
 * them `"proxyRules"` and `"wsConnections"`. Every panel passes the protocol name (`"mocks"`,
 * `"requests"`, `"webhooks"`, …) **except** two, which pass the engine's:
 *
 * ```
 * packages/gui/src/panels/ProxyRulesPanel.tsx:74,132,165   loadEntity(ws, "rules",   id)
 * packages/gui/src/panels/WebSocketsPanel.tsx:146,159,250  loadEntity(ws, "sockets", id)
 * ```
 *
 * This is the **`window.api`-fidelity half** of a translation that has two halves. The engine owns
 * the other one — `toEngineKind()` in `packages/engine/src/commands/entityKindMap.ts` — and that is
 * the one that has always been on the live path, because the client used to forward the caller's
 * `kind` verbatim and `EntityLoadParams.kind` is the strict `EntityKind` enum. So the wire rejected
 * `"rules"` with a `BAD_REQUEST`, the two panels' `.catch(() => {})` swallowed it, and their entity
 * loads silently returned nothing. The deleted `entity:load` channel was the only implementation of
 * this half — `plan/07` §5b-septies (sixth group) has the full accounting, and it is the same shape
 * as the TLS path loss in §5b-undecies.
 *
 * **Why the table is duplicated rather than imported.** `@bifurc/engine` is a *dev*Dependency of this
 * package — the client's only runtime dependency is `@bifurc/protocol`, because a client is supposed
 * to drive any engine over a transport and must not link one. So `toProtocolKind()` cannot be
 * imported here. Two entries is a cheaper cost than a runtime engine dependency, and
 * `packages/client/tests/client.test.ts` pins both directions so the two copies cannot drift
 * silently. It belongs in the client rather than the protocol for the reason above: this is a
 * statement about what `window.api`'s callers pass, not about the wire.
 */
const ENGINE_TO_PROTOCOL_KIND: Record<string, string> = {
  rules: "proxyRules",
  sockets: "wsConnections",
};

/** Identity for the other fourteen kinds — see `ENGINE_TO_PROTOCOL_KIND`. */
function toProtocolKind(engineKind: string): string {
  return ENGINE_TO_PROTOCOL_KIND[engineKind] ?? engineKind;
}

export function createClient(transport: Transport, opts: CreateClientOptions): BifurcClient {
  const { local } = opts;
  const hub = createSubscriptionHub(transport);
  const retryPolicy: RetryPolicy = { ...DEFAULT_RETRY_POLICY, ...opts.retry };

  /**
   * Set by `close()`, and read by the retry loop. A closed transport rejects every request with
   * `ENGINE_ERROR` — a *retryable* code — so without this the client would retry three times
   * against a transport it knows is gone. See `retry.ts`'s `withRetry` for why this is a guard
   * rather than the fix.
   */
  let closed = false;

  /**
   * Every request goes through here, so the retry policy cannot be bypassed by adding a method.
   *
   * Takes the **command** rather than a surface key because two internal callers (`config.get` for
   * the active workspace, `blob.read` for an artifact) are not surface methods — routing them
   * around the policy would have left the two most latency-sensitive reads unprotected.
   */
  const request = (command: string, payload: unknown): Promise<unknown> =>
    withRetry(command, () => transport.request(command, payload), retryPolicy, () => closed);

  const call = (key: string, payload: unknown): Promise<unknown> =>
    request(commandFor(key), payload);

  /**
   * `entity.list`'s `workspaceId` is **required**, but the three `window.api` listers carry none —
   * the legacy handlers read `loadConfig().activeWorkspaceId` inside the main process. The client
   * has to resolve it, and it does so by **asking the engine each time** rather than caching.
   *
   * Caching would be faster and wrong: the active workspace changes through four other calls
   * (`workspace.setActive`, `workspace.add`, `workspace.delete`, and the shell's own UI), and a
   * stale id here returns another workspace's schemas — a silent wrong answer, not an error. These
   * three listers are called from a settings panel, not a hot loop, so one extra round-trip is the
   * right trade.
   */
  const resolveWorkspaceId = async (): Promise<string> => {
    const config = (await request("config.get", {})) as { activeWorkspaceId?: string };
    const id = config?.activeWorkspaceId;
    if (!id) {
      throw new Error(
        "@bifurc/client: no active workspace — entity.list needs one and the engine reported none.",
      );
    }
    return id;
  };

  /**
   * Pull a blob in slices and return its **entire** content as base64.
   *
   * The loop is not optional and not an optimisation. `blob.read` returns at most
   * `BLOB_READ_CHUNK_BYTES` (512 KB) unless asked for more, and `eof` is the only signal that the
   * last slice has arrived — a single un-offset read silently returns the *first* slice, so any
   * artifact over 512 KB is truncated with no error anywhere. `BLOB_INLINE_THRESHOLD_BYTES` is
   * 1 MB, so "large enough to be a blob" and "larger than one chunk" overlap almost completely:
   * a real workspace export takes this path.
   *
   * Slices are accumulated as **bytes**, not by concatenating base64 strings. Each slice is encoded
   * independently and therefore padded independently, and `=` mid-stream is either a decode error
   * or a silent truncation depending on the decoder — so the join happens below the encoding.
   *
   * `offset` advances by decoded bytes rather than by base64 characters, which are 4/3 as many.
   */
  const pullBlob = async (blobId: string): Promise<string> => {
    const parts: Uint8Array[] = [];
    let total = 0;
    let offset = 0;
    try {
      for (;;) {
        const read = (await request("blob.read", { blobId, offset })) as
          | { data?: string; eof?: boolean }
          | undefined;
        const slice = typeof read?.data === "string" ? base64ToBytes(read.data) : new Uint8Array(0);
        if (slice.length === 0) break;
        parts.push(slice);
        total += slice.length;
        offset += slice.length;
        if (read?.eof) break;
      }
    } finally {
      // Best-effort, and in a `finally` for the same reason as `fileOpsClient.writeArtifact`: the
      // lease exists to survive exactly one pull, and a failed pull must not pin bytes until the
      // sweep notices. A release failure must never fail the export the user actually asked for.
      await request("blob.release", { blobId }).catch(() => undefined);
    }

    const whole = new Uint8Array(total);
    let at = 0;
    for (const part of parts) {
      whole.set(part, at);
      at += part.length;
    }
    return bytesToBase64(whole);
  };

  // ── artifact egress / ingress ─────────────────────────────────────────────
  //
  // `File_Ops_Protocol.md` §3.2's rule is "the client shows the dialog first", and the reason is not
  // cosmetic: fail fast on cancel, so the engine never renders a 200 MB workspace archive the user
  // then abandons. Whether that is *expressible* depends on where the write falls in the sequence —
  // egress is dialog → command → write, so the ask must be separable from the write; ingress is
  // dialog → read → upload → command, so the two can share one call. The hooks are shaped by that
  // difference rather than by taste, which is why `pickSavePath` has no ingress twin.

  /** A path's last segment, without assuming Node's `path` — this bundles for a browser too. */
  const baseName = (p: string): string => p.split(/[\\/]/).pop() || p;

  /**
   * `entity.create` resolves the frozen **`{id, entity}`** envelope; the surface declares the *raw*
   * entity, and every create key has to hand the raw one back.
   *
   * **This is the unwrapping the shell channels used to do, and the reason all 16 create keys were
   * wrong on the routed path.** `addMapping` / `addRule` / `addRequest` / … returned `{id, entity}`
   * while `packages/gui/src/types/window.ts` declares `Promise<LocalMapping>` and the panels consume it
   * directly — `MappingsPanel.tsx:120` pushes the result straight into `config.mappings`, and
   * `ProxyRulesPanel.tsx:155` files it as a loaded entity. The `as ReturnType<BifurcApi[…] >` cast made
   * the compiler agree with the **declaration** rather than with the runtime, so nothing failed: the
   * panels rendered objects carrying `id` and no other field. Same class as `checkUpdate` and
   * `tlsCertStatus`, and the widest instance of it. Found 2026-09-20 while splitting
   * `entityCrudFactory` (`plan/07` §5b-ter).
   *
   * The tell was already in this file: `entity.list`'s `{entities}` envelope **is** unwrapped, three
   * lines away (`listWsdls`, `listProtoFiles`, `listGraphQLSchemas`). One envelope family, half
   * handled — which is why the asymmetry survived review.
   *
   * The gate-blocked create is the one result that is not an envelope: `environments` returns
   * `{error: "limit_reached", ...gate}` with no `.entity`, and the shell passed that through
   * unchanged — so it passes through unchanged here.
   */
  const unwrapCreated = (result: unknown): unknown => {
    if (result !== null && typeof result === "object" && "entity" in result) {
      return (result as { entity: unknown }).entity;
    }
    return result;
  };

  /**
   * The dialog title's noun, per entity kind.
   *
   * A copy of the shell's `kindLabel` in `src/ipc/importExportHandlers.ts`. That file is now dead —
   * once the preload routes these methods through the client, nothing calls the three
   * `importExport:*` channels it serves — and it goes at step 3's final deletion, at which point
   * this becomes the only copy. Copied rather than moved for one commit, because moving it would
   * mean editing a file that is about to be deleted.
   */
  const KIND_LABEL: Record<string, string> = {
    workspace: "Workspace",
    requests: "Requests",
    mocks: "Mocks",
    environments: "Environments",
    mappings: "Mappings",
    proxyRules: "Proxy Rules",
    websockets: "WebSockets",
    webhooks: "Webhooks",
  };

  /** `proxyRules` → `proxy-rules`, the kebab-case the default export file name uses. */
  const safeKind = (kind: string): string => kind.replace(/([A-Z])/g, "-$1").toLowerCase();

  /**
   * The engine's format table entry for one `kind`+`format` — a dialog's filters and default
   * extension.
   *
   * Asked of the engine rather than hard-coded, because the 17 formats and their extensions are
   * `packages/engine/src/importExport/registry.ts`'s to declare, and a second list here is how the
   * two would drift. It is the same cheap `export.formats` the modal already reads when it opens.
   * It is read *before* the dialog, which has one visible consequence worth stating plainly: with no
   * engine there is no dialog — correct, because there is nothing to export either way.
   */
  const formatsFor = async (kind: string | undefined, formatId: string | undefined) => {
    const all = (await call("getImportExportFormats", {})) as
      | Record<string, { id: string; label: string; extensions: string[] }[]>
      | undefined;
    return (
      all?.[kind ?? ""]?.find((f) => f.id === formatId) ?? {
        id: formatId ?? "",
        label: "Files",
        extensions: ["json"],
      }
    );
  };

  /** The dialog chrome for an import, from the engine's own format table. */
  const importDialogOpts = async (
    kind: string | undefined,
    formatId: string | undefined,
  ): Promise<ReadFileOptions> => {
    const f = await formatsFor(kind, formatId);
    return {
      title: `Import ${KIND_LABEL[kind ?? ""] ?? kind ?? ""}`,
      filters: [
        { name: f.label, extensions: f.extensions },
        { name: "All Files", extensions: ["*"] },
      ],
    };
  };

  /**
   * Turn an `ArtifactResult` into a file on the client's disk.
   *
   * The engine decides inline-vs-blob (`File_Ops_Protocol.md` §3.2) and returns `sha256` on **both**
   * branches, so this only has to check which shape arrived. A blob is released by `pullBlob` — the
   * lease exists to survive exactly one pull, and leaving it would pin bytes for its whole TTL.
   *
   * Content stays base64 from the engine all the way to the `writeArtifact` hook. Nothing here
   * decodes it, because nothing here needs to: the hook writes bytes, and any decode-and-re-encode
   * in between is a chance to corrupt an artifact that is not valid UTF-8.
   *
   * `destPath` is the dialog-first case — a path the user already chose through `pickSavePath`, so
   * the hook writes there without asking again. Omitted, the hook shows the dialog itself and uses
   * the engine's own `suggestedName`/`mimeType` as its chrome, which is the right default for the
   * methods whose artifact is cheap and already produced.
   */
  const artifactToFile = async (
    result: unknown,
    fallbackName: string,
    destPath?: string,
  ): Promise<ArtifactWriteResult> => {
    const r = result as
      | { ok?: boolean; error?: string; canceled?: boolean; inline?: string; blobId?: string; suggestedName?: string; mimeType?: string }
      | undefined;

    if (!r || r.ok !== true) {
      return r?.canceled ? { ok: false, canceled: true } : { ok: false, error: r?.error ?? "the engine did not produce an artifact" };
    }
    if (!local.writeArtifact) {
      return { ok: false, error: "this client cannot write files (no writeArtifact hook)" };
    }

    let contentBase64: string;
    if (typeof r.inline === "string") {
      contentBase64 = r.inline;
    } else if (typeof r.blobId === "string") {
      contentBase64 = await pullBlob(r.blobId);
    } else {
      return { ok: false, error: "artifact carried neither inline content nor a blobId" };
    }

    return local.writeArtifact(
      contentBase64,
      r.suggestedName ?? fallbackName,
      r.mimeType ?? "application/octet-stream",
      destPath,
    );
  };

  /**
   * Read a file locally, upload it, and return the blob id the command needs.
   *
   * `opts.path` is the reuse case — `importData` reading the file `preflightImport` already chose.
   * With no path the hook shows the dialog, so one call expresses both.
   */
  const uploadLocalFile = async (
    opts?: ReadFileOptions,
  ): Promise<{ blobId: string; filePath: string } | { error: string } | { canceled: true }> => {
    if (!local.readArtifactFile) return { error: "this client cannot read files (no readArtifactFile hook)" };
    const file = await local.readArtifactFile(opts);
    if (!file) return { canceled: true };
    if ("error" in file) return { error: file.error };
    const put = (await request("blob.put", {
      filename: file.name,
      mimeType: file.mimeType,
      size: file.size,
      data: file.base64,
    })) as { blobId?: string } | undefined;
    if (!put?.blobId) return { error: "blob.put did not return a blobId" };
    return { blobId: put.blobId, filePath: file.filePath };
  };

  /**
   * The renderer identifies an upload by the local path it came from, so the client has to remember
   * which blob that path produced.
   *
   * This is what makes `File_Ops_Protocol.md` §5's **uploaded ONCE** true across the two-step
   * import: `preflightImport` stages the blob, `importData` reuses it, and the bytes cross the wire
   * once. Bounded by construction — one entry per file picked since the last preflight, cleared on
   * commit. The shell used to own this map (`importExportHandlers.ts`); it dies with that file.
   */
  const pendingUploads = new Map<string, string>();

  const releaseQuietly = (blobId: string): void => {
    void request("blob.release", { blobId }).catch(() => undefined);
  };

  const rememberUpload = (filePath: string, blobId: string): void => {
    const previous = pendingUploads.get(filePath);
    // Re-running preflight on the same file (the user changed the format and tried again) replaces
    // the entry; release the one it displaces rather than leaving it for the sweep.
    if (previous && previous !== blobId) releaseQuietly(previous);
    pendingUploads.set(filePath, blobId);
  };

  const forgetUpload = (filePath: string): void => {
    pendingUploads.delete(filePath);
  };

  /** The remembered blob for a path, or a fresh read plus upload. */
  const uploadOnce = async (
    opts: ReadFileOptions,
  ): Promise<{ blobId: string; filePath: string } | { error: string } | { canceled: true }> => {
    if (opts.path) {
      const remembered = pendingUploads.get(opts.path);
      if (remembered) return { blobId: remembered, filePath: opts.path };
    }
    return uploadLocalFile(opts);
  };

  /**
   * `onLogEntry` is the one subscription that is not a straight pass-through.
   *
   * `eventPump.ts` coalesces `log.entry` and delivers batches **under the same wire name**
   * (`COALESCED_EVENT = "event.log.entry"`), so the payload is either a single entry or
   * `{entries: [...]}`. `toLogEntryBatch` is idempotent, which is what makes this unwrap safe to
   * apply unconditionally. Without it a burst of traffic would hand the renderer one object shaped
   * like `{entries}` where it expects a log entry, and every panel would blank at exactly the moment
   * there was most to show.
   */
  const onLogEntry = (cb: (entry: LogEntry) => void): (() => void) =>
    hub.subscribe("event.log.entry", (payload) => {
      const p = payload as { entries?: unknown[] } | undefined;
      if (p && Array.isArray(p.entries)) {
        for (const entry of p.entries) cb(entry as LogEntry);
      } else {
        cb(payload as LogEntry);
      }
    });

  /** A plain subscription: the wire payload *is* the callback argument. */
  const plain = <K extends keyof BifurcApi>(event: string, cb: (payload: never) => void): (() => void) =>
    hub.subscribe(event, cb as (payload: unknown) => void);

  const api: BifurcClient = {
    // ── app / config ────────────────────────────────────────────────────────
    // `checkUpdate` used to be first in this block, as `call("checkUpdate", {})`. It is **not** here
    // because it is not a transport method: it moved to the client-local group below on 2026-09-20,
    // and `packages/protocol` no longer declares `app.checkUpdate`. The cast it used to carry —
    // `as ReturnType<BifurcApi["checkUpdate"]>` — was a lie in the direction that fails silently:
    // `AppCheckUpdateResult` had no `hasUpdate`, so routing it would have left the renderer's
    // `updateResult.hasUpdate` undefined and made the About panel report "up to date" forever, with
    // no error anywhere. See `local.ts`'s `checkUpdate` and `surface.ts`.
    getConfig: () => call("getConfig", {}) as ReturnType<BifurcApi["getConfig"]>,
    // Reclassified `transport` → `shim` on 2026-09-21, when the P6 reduction's sixth group deleted the
    // `entity:load` channel. That channel applied `toProtocolKind()` before invoking the command; the
    // client forwarded the caller's `kind` verbatim, so the two panels that name an engine kind got a
    // `BAD_REQUEST` they swallowed. See `ENGINE_TO_PROTOCOL_KIND` above.
    loadEntity: (wsId, kind, id) =>
      call("loadEntity", { workspaceId: wsId, kind: toProtocolKind(kind), id }) as ReturnType<BifurcApi["loadEntity"]>,
    // Reclassified `transport` → `shim` in the same group, for the other half of the same channel pair.
    // `entity:setEnabled`'s channel kept an explicit guard *ahead* of `commandRegistry.invoke()`, because
    // `EntitySetEnabledParams.kind` is a strict six-value enum: an out-of-enum kind would otherwise throw
    // `BAD_REQUEST` instead of resolving the `{ok: false, error: "invalid_kind"}` the method has always
    // returned. The guard is checked against the protocol's **own** schema rather than a second copy of
    // the six names, so a widening there cannot leave this behind.
    //
    // Unlike the `loadEntity` case this one is latent, not live — every caller passes a kind the enum
    // accepts (the mocks panel's `resolveEntityKind` yields only `mocks` / `graphqlMocks` / `grpcMocks` /
    // `soapMocks` in mock mode, and the two panels that toggle pass the literals `"mappings"` and
    // `"rules"`). It is carried over anyway because it is a *documented* client-facing behaviour of the
    // surface being reconstructed, and `packages/gui/src/types/window.ts`'s `Promise<{ok: boolean; error?:
    // string}>` is the contract that says so.
    setEntityEnabled: (wsId, kind, id, enabled) => {
      if (!EntitySetEnabledParams.shape.kind.safeParse(kind).success) {
        return Promise.resolve({ ok: false, error: "invalid_kind" }) as ReturnType<BifurcApi["setEntityEnabled"]>;
      }
      return call("setEntityEnabled", { workspaceId: wsId, kind, id, enabled }) as ReturnType<BifurcApi["setEntityEnabled"]>;
    },
    saveConfig: (config) => call("saveConfig", { config }) as ReturnType<BifurcApi["saveConfig"]>,

    // ── import / export ─────────────────────────────────────────────────────
    getImportExportFormats: () => call("getImportExportFormats", {}) as ReturnType<BifurcApi["getImportExportFormats"]>,
    exportData: async (req) => {
      // `wsId` on the wire, `workspaceId` in the command — `window.api` is positional-ish and the
      // frozen schema is not. Read through the declared type rather than a cast: the previous
      // version of this method cast to `{ workspaceId?: string }`, which is a field the renderer
      // never sends, and nothing noticed because the preload overrode the method entirely.
      const { kind, format, wsId } = req;
      if (!local.pickSavePath || !local.writeArtifact) {
        return { ok: false, error: "this client cannot write files (no filesystem hooks)" };
      }

      // ── 1. Dialog FIRST ────────────────────────────────────────────────────
      // `File_Ops_Protocol.md` §3.2: fail fast on cancel, so the engine never renders a 200 MB
      // workspace archive the user then abandons. This is the ordering the bundled `writeArtifact`
      // could not express, and it is why `pickSavePath` exists.
      const f = await formatsFor(kind, format);
      const dest = await local.pickSavePath({
        title: `Export ${KIND_LABEL[kind] ?? kind}`,
        defaultName: `${safeKind(kind)}-export.${f.extensions[0] ?? "json"}`,
        filters: [
          { name: f.label, extensions: f.extensions },
          { name: "All Files", extensions: ["*"] },
        ],
      });
      if (!dest) return { ok: false, canceled: true };

      // ── 2. The engine renders it ───────────────────────────────────────────
      // Only the *name* crosses, never the path — the engine has no use for a path it cannot act on.
      const created = await call("exportData", {
        kind,
        format,
        workspaceId: wsId,
        filename: baseName(dest),
      });

      // ── 3. The client writes the bytes ─────────────────────────────────────
      try {
        return await artifactToFile(created, baseName(dest), dest);
      } catch (err) {
        return { ok: false, error: `Could not write ${baseName(dest)}: ${String(err)}` };
      }
    },
    preflightImport: async (req) => {
      const { kind, format, wsId } = req;
      const uploaded = await uploadLocalFile(await importDialogOpts(kind, format));
      if ("canceled" in uploaded) return { ok: false, canceled: true };
      if ("error" in uploaded) return { ok: false, error: uploaded.error };

      const pf = (await call("preflightImport", {
        kind,
        format,
        workspaceId: wsId,
        blobId: uploaded.blobId,
      })) as
        | { ok: true; itemCount: number; collisionIds: string[] }
        | { ok: false; error: string }
        | undefined;
      if (!pf?.ok) {
        releaseQuietly(uploaded.blobId);
        return { ok: false, error: pf?.error ?? "the engine could not read the import" };
      }

      rememberUpload(uploaded.filePath, uploaded.blobId);

      // `filePath` is echoed because the renderer threads it from here into `importData` — and it is
      // now the path the **client** chose. That is the fix for the second dialog: the shell used to
      // have to guess, because `dialog:openFile` returns no path at all, so `importData` re-asked.
      return { ok: true, filePath: uploaded.filePath, itemCount: pf.itemCount, collisionIds: pf.collisionIds };
    },
    importData: async (req) => {
      const { kind, format, wsId, filePath, collisionStrategy } = req;

      // `filePath` is what `preflightImport` returned. Reusing it is the whole point: asking again
      // would be a second dialog for a file the user already picked, and re-reading it would
      // re-upload bytes the engine already has. It is declared required on the renderer's
      // `ImportRequest`, so an empty one means a caller that skipped preflight — ask rather than
      // read `undefined`, which is what the old path did and why the collision branch could hand
      // `undefined` to `applyImport`.
      const uploaded = await uploadOnce(
        filePath ? { path: filePath } : await importDialogOpts(kind, format),
      );
      if ("canceled" in uploaded) return { ok: false, canceled: true };
      if ("error" in uploaded) return { ok: false, error: uploaded.error };

      try {
        return (await call("importData", {
          kind,
          format,
          workspaceId: wsId,
          blobId: uploaded.blobId,
          collisionStrategy,
        })) as ReturnType<BifurcApi["importData"]>;
      } finally {
        // Imports are transform-and-discard (`File_Ops_Protocol.md` §5): the source file never
        // persists on the engine. Released on failure too — a failed commit is retried from the
        // start (the renderer returns to its idle state), which re-runs preflight and re-uploads.
        forgetUpload(uploaded.filePath);
        releaseQuietly(uploaded.blobId);
      }
    },
    discoverServices: () => call("discoverServices", {}) as ReturnType<BifurcApi["discoverServices"]>,

    // ── the CRUD collapse ───────────────────────────────────────────────────
    //
    // Every `create` here goes through `unwrapCreated`, because `entity.create` answers the frozen
    // `{id, entity}` envelope and the surface declares the raw entity — see the helper's note. The
    // `update` / `delete` / `list` keys need no unwrap: `entity.update` and `entity.delete` return
    // what their channels always did, and `entity.list`'s `{entities}` envelope is unwrapped inline
    // below, as it always has been.
    addMapping: async (m) => unwrapCreated(await call("addMapping", { kind: "mappings", entity: m })) as Awaited<ReturnType<BifurcApi["addMapping"]>>,
    updateMapping: (m) => call("updateMapping", { kind: "mappings", entity: m }) as ReturnType<BifurcApi["updateMapping"]>,
    deleteMapping: (id) => call("deleteMapping", { kind: "mappings", id }) as ReturnType<BifurcApi["deleteMapping"]>,
    addRule: async (r) => unwrapCreated(await call("addRule", { kind: "proxyRules", entity: r })) as Awaited<ReturnType<BifurcApi["addRule"]>>,
    updateRule: (r) => call("updateRule", { kind: "proxyRules", entity: r }) as ReturnType<BifurcApi["updateRule"]>,
    deleteRule: (id) => call("deleteRule", { kind: "proxyRules", id }) as ReturnType<BifurcApi["deleteRule"]>,
    addMock: async (m) => unwrapCreated(await call("addMock", { kind: "mocks", entity: m })) as Awaited<ReturnType<BifurcApi["addMock"]>>,
    updateMock: (m) => call("updateMock", { kind: "mocks", entity: m }) as ReturnType<BifurcApi["updateMock"]>,
    deleteMock: (id) => call("deleteMock", { kind: "mocks", id }) as ReturnType<BifurcApi["deleteMock"]>,
    addRequest: async (r) => unwrapCreated(await call("addRequest", { kind: "requests", entity: r })) as Awaited<ReturnType<BifurcApi["addRequest"]>>,
    updateRequest: (r) => call("updateRequest", { kind: "requests", entity: r }) as ReturnType<BifurcApi["updateRequest"]>,
    deleteRequest: (id) => call("deleteRequest", { kind: "requests", id }) as ReturnType<BifurcApi["deleteRequest"]>,
    addWsConnection: async (c) => unwrapCreated(await call("addWsConnection", { kind: "wsConnections", entity: c })) as Awaited<ReturnType<BifurcApi["addWsConnection"]>>,
    updateWsConnection: (c) => call("updateWsConnection", { kind: "wsConnections", entity: c }) as ReturnType<BifurcApi["updateWsConnection"]>,
    deleteWsConnection: (id) => call("deleteWsConnection", { kind: "wsConnections", id }) as ReturnType<BifurcApi["deleteWsConnection"]>,
    addWebhook: async (h) => unwrapCreated(await call("addWebhook", { kind: "webhooks", entity: h })) as Awaited<ReturnType<BifurcApi["addWebhook"]>>,
    updateWebhook: (h) => call("updateWebhook", { kind: "webhooks", entity: h }) as ReturnType<BifurcApi["updateWebhook"]>,
    deleteWebhook: (id) => call("deleteWebhook", { kind: "webhooks", id }) as ReturnType<BifurcApi["deleteWebhook"]>,
    addGraphQLRequest: async (r) => unwrapCreated(await call("addGraphQLRequest", { kind: "graphqlRequests", entity: r })) as Awaited<ReturnType<BifurcApi["addGraphQLRequest"]>>,
    updateGraphQLRequest: (r) => call("updateGraphQLRequest", { kind: "graphqlRequests", entity: r }) as ReturnType<BifurcApi["updateGraphQLRequest"]>,
    deleteGraphQLRequest: (id) => call("deleteGraphQLRequest", { kind: "graphqlRequests", id }) as ReturnType<BifurcApi["deleteGraphQLRequest"]>,
    addGraphQLMock: async (m) => unwrapCreated(await call("addGraphQLMock", { kind: "graphqlMocks", entity: m })) as Awaited<ReturnType<BifurcApi["addGraphQLMock"]>>,
    updateGraphQLMock: (m) => call("updateGraphQLMock", { kind: "graphqlMocks", entity: m }) as ReturnType<BifurcApi["updateGraphQLMock"]>,
    deleteGraphQLMock: (id) => call("deleteGraphQLMock", { kind: "graphqlMocks", id }) as ReturnType<BifurcApi["deleteGraphQLMock"]>,
    addGraphQLSchema: async (s) => unwrapCreated(await call("addGraphQLSchema", { kind: "graphqlSchemas", entity: s })) as Awaited<ReturnType<BifurcApi["addGraphQLSchema"]>>,
    deleteGraphQLSchema: (id) => call("deleteGraphQLSchema", { kind: "graphqlSchemas", id }) as ReturnType<BifurcApi["deleteGraphQLSchema"]>,
    listGraphQLSchemas: async () =>
      ((await call("listGraphQLSchemas", { kind: "graphqlSchemas", workspaceId: await resolveWorkspaceId() })) as { entities: unknown[] }).entities as Awaited<ReturnType<BifurcApi["listGraphQLSchemas"]>>,
    addGrpcRequest: async (r) => unwrapCreated(await call("addGrpcRequest", { kind: "grpcRequests", entity: r })) as Awaited<ReturnType<BifurcApi["addGrpcRequest"]>>,
    updateGrpcRequest: (r) => call("updateGrpcRequest", { kind: "grpcRequests", entity: r }) as ReturnType<BifurcApi["updateGrpcRequest"]>,
    deleteGrpcRequest: (id) => call("deleteGrpcRequest", { kind: "grpcRequests", id }) as ReturnType<BifurcApi["deleteGrpcRequest"]>,
    addGrpcMock: async (m) => unwrapCreated(await call("addGrpcMock", { kind: "grpcMocks", entity: m })) as Awaited<ReturnType<BifurcApi["addGrpcMock"]>>,
    updateGrpcMock: (m) => call("updateGrpcMock", { kind: "grpcMocks", entity: m }) as ReturnType<BifurcApi["updateGrpcMock"]>,
    deleteGrpcMock: (id) => call("deleteGrpcMock", { kind: "grpcMocks", id }) as ReturnType<BifurcApi["deleteGrpcMock"]>,
    addProtoFile: async (p) => unwrapCreated(await call("addProtoFile", { kind: "protoFiles", entity: p })) as Awaited<ReturnType<BifurcApi["addProtoFile"]>>,
    deleteProtoFile: (id) => call("deleteProtoFile", { kind: "protoFiles", id }) as ReturnType<BifurcApi["deleteProtoFile"]>,
    listProtoFiles: async () =>
      ((await call("listProtoFiles", { kind: "protoFiles", workspaceId: await resolveWorkspaceId() })) as { entities: unknown[] }).entities as Awaited<ReturnType<BifurcApi["listProtoFiles"]>>,
    addSoapRequest: async (r) => unwrapCreated(await call("addSoapRequest", { kind: "soapRequests", entity: r })) as Awaited<ReturnType<BifurcApi["addSoapRequest"]>>,
    updateSoapRequest: (r) => call("updateSoapRequest", { kind: "soapRequests", entity: r }) as ReturnType<BifurcApi["updateSoapRequest"]>,
    deleteSoapRequest: (id) => call("deleteSoapRequest", { kind: "soapRequests", id }) as ReturnType<BifurcApi["deleteSoapRequest"]>,
    addSoapMock: async (m) => unwrapCreated(await call("addSoapMock", { kind: "soapMocks", entity: m })) as Awaited<ReturnType<BifurcApi["addSoapMock"]>>,
    updateSoapMock: (m) => call("updateSoapMock", { kind: "soapMocks", entity: m }) as ReturnType<BifurcApi["updateSoapMock"]>,
    deleteSoapMock: (id) => call("deleteSoapMock", { kind: "soapMocks", id }) as ReturnType<BifurcApi["deleteSoapMock"]>,
    addWsdl: async (w) => unwrapCreated(await call("addWsdl", { kind: "wsdls", entity: w })) as Awaited<ReturnType<BifurcApi["addWsdl"]>>,
    deleteWsdl: (id) => call("deleteWsdl", { kind: "wsdls", id }) as ReturnType<BifurcApi["deleteWsdl"]>,
    listWsdls: async () =>
      ((await call("listWsdls", { kind: "wsdls", workspaceId: await resolveWorkspaceId() })) as { entities: unknown[] }).entities as Awaited<ReturnType<BifurcApi["listWsdls"]>>,
    addEnvironment: async (e) => unwrapCreated(await call("addEnvironment", { kind: "environments", entity: e })) as Awaited<ReturnType<BifurcApi["addEnvironment"]>>,
    updateEnvironment: (e) => call("updateEnvironment", { kind: "environments", entity: e }) as ReturnType<BifurcApi["updateEnvironment"]>,
    deleteEnvironment: (id) => call("deleteEnvironment", { kind: "environments", id }) as ReturnType<BifurcApi["deleteEnvironment"]>,

    // ── folders ─────────────────────────────────────────────────────────────
    addFolder: (kind, folder) =>
      call("addFolder", {
        kind,
        name: (folder as { name?: string }).name,
        parentId: (folder as { parentId?: string | null }).parentId ?? null,
      }) as ReturnType<BifurcApi["addFolder"]>,
    renameFolder: (kind, id, name) => call("renameFolder", { kind, id, name }) as ReturnType<BifurcApi["renameFolder"]>,
    moveFolder: (kind, id, parentId) => call("moveFolder", { kind, id, parentId }) as ReturnType<BifurcApi["moveFolder"]>,
    deleteFolder: (kind, id) => call("deleteFolder", { kind, id }) as ReturnType<BifurcApi["deleteFolder"]>,

    // ── environments / workspaces ───────────────────────────────────────────
    setActiveEnvironment: (id) => call("setActiveEnvironment", { id }) as ReturnType<BifurcApi["setActiveEnvironment"]>,
    addWorkspace: (name) => call("addWorkspace", { name }) as ReturnType<BifurcApi["addWorkspace"]>,
    renameWorkspace: (id, name) => call("renameWorkspace", { id, name }) as ReturnType<BifurcApi["renameWorkspace"]>,
    deleteWorkspace: (id) => call("deleteWorkspace", { id }) as ReturnType<BifurcApi["deleteWorkspace"]>,
    setActiveWorkspace: (id) => call("setActiveWorkspace", { id }) as ReturnType<BifurcApi["setActiveWorkspace"]>,

    // ── server / proxy ──────────────────────────────────────────────────────
    replayRequest: (method, url, headers, body) =>
      call("replayRequest", { method, url, headers, body }) as ReturnType<BifurcApi["replayRequest"]>,
    proxyStatus: () => call("proxyStatus", {}) as ReturnType<BifurcApi["proxyStatus"]>,
    serverStatus: () => call("serverStatus", {}) as ReturnType<BifurcApi["serverStatus"]>,
    restartServer: () => call("restartServer", {}) as ReturnType<BifurcApi["restartServer"]>,
    stopServer: () => call("stopServer", {}) as ReturnType<BifurcApi["stopServer"]>,
    startServer: () => call("startServer", {}) as ReturnType<BifurcApi["startServer"]>,

    // ── audit / history ─────────────────────────────────────────────────────
    listAudit: (o) => call("listAudit", o ?? {}) as ReturnType<BifurcApi["listAudit"]>,
    auditDiff: (commitHash, entity, entityId, wsId) =>
      call("auditDiff", { commitHash, entity, entityId, workspaceId: wsId }) as ReturnType<BifurcApi["auditDiff"]>,
    exportAudit: async (format) => {
      // Dialog FIRST, then the engine queries — `File_Ops_Protocol.md` §3.2, and the whole reason P3
      // changed this channel: `audit.export` runs `queryLog({ limit: 0 })`, the entire `git log`, so an
      // artifact-first ordering makes cancelling pay for the full query. That is precisely the cost P3
      // removed, and the shell body this replaces is dialog-first.
      //
      // This call used to be `artifactToFile(await call("exportAudit", { format }), …)` with no
      // `destPath`, which silently took `artifactToFile`'s artifact-first default. Found 2026-09-20 by
      // the §5b-bis twin audit in `plan/07`; the ordering is chosen by *omitting an argument*, which is
      // why it read as ordinary.
      if (!local.pickSavePath || !local.writeArtifact) {
        return { ok: false };
      }
      const dest = await local.pickSavePath({
        title: "Export Audit Log",
        defaultName: `audit-log.${format}`,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      // A bare `{ok:false}` on cancel, matching the shell body and the declared `{ok: boolean}`.
      if (!dest) return { ok: false };
      const r = await artifactToFile(await call("exportAudit", { format }), baseName(dest), dest);
      return r.ok ? { ok: true } : { ok: false };
    },
    listHistory: (o) => call("listHistory", o ?? {}) as ReturnType<BifurcApi["listHistory"]>,
    historyDiff: (commitHash, filePath, wsId) =>
      call("historyDiff", { commitHash, filePath, workspaceId: wsId }) as ReturnType<BifurcApi["historyDiff"]>,

    // ── sync / git ──────────────────────────────────────────────────────────
    syncSetRemote: (wsId, remote, branch) =>
      call("syncSetRemote", { workspaceId: wsId, remote, branch }) as ReturnType<BifurcApi["syncSetRemote"]>,
    syncDisconnect: (wsId) => call("syncDisconnect", { workspaceId: wsId }) as ReturnType<BifurcApi["syncDisconnect"]>,
    syncPush: (wsId) => call("syncPush", { workspaceId: wsId }) as ReturnType<BifurcApi["syncPush"]>,
    syncPull: (wsId) => call("syncPull", { workspaceId: wsId }) as ReturnType<BifurcApi["syncPull"]>,
    syncGetState: (wsId) => call("syncGetState", { workspaceId: wsId }) as ReturnType<BifurcApi["syncGetState"]>,
    syncSetAutoSync: (wsId, enabled) =>
      call("syncSetAutoSync", { workspaceId: wsId, enabled }) as ReturnType<BifurcApi["syncSetAutoSync"]>,
    publishEntity: (wsId, paths, message) =>
      call("publishEntity", { workspaceId: wsId, paths, message }) as ReturnType<BifurcApi["publishEntity"]>,
    publishFolder: (wsId, kind, folderName) =>
      call("publishFolder", { workspaceId: wsId, kind, folderName }) as ReturnType<BifurcApi["publishFolder"]>,
    restoreEntity: (wsId, relPath) => call("restoreEntity", { workspaceId: wsId, relPath }) as ReturnType<BifurcApi["restoreEntity"]>,
    gitGetDiff: (wsId, relPath) => call("gitGetDiff", { workspaceId: wsId, relPath }) as ReturnType<BifurcApi["gitGetDiff"]>,
    gitDiscard: (wsId, relPath) => call("gitDiscard", { workspaceId: wsId, relPath }) as ReturnType<BifurcApi["gitDiscard"]>,
    gitSync: (wsId, paths, message) => call("gitSync", { workspaceId: wsId, paths, message }) as ReturnType<BifurcApi["gitSync"]>,
    gitGetHistory: (wsId, relPath, o) =>
      call("gitGetHistory", { workspaceId: wsId, relPath, ...(o ?? {}) }) as ReturnType<BifurcApi["gitGetHistory"]>,
    getEntitySyncStatus: (wsId) => call("getEntitySyncStatus", { workspaceId: wsId }) as ReturnType<BifurcApi["getEntitySyncStatus"]>,
    executeScript: (o) => call("executeScript", o) as ReturnType<BifurcApi["executeScript"]>,

    // ── healthbar ───────────────────────────────────────────────────────────
    healthbarGetServices: (wsId) => call("healthbarGetServices", { workspaceId: wsId }) as ReturnType<BifurcApi["healthbarGetServices"]>,
    healthbarSaveServices: (wsId, services) =>
      call("healthbarSaveServices", { workspaceId: wsId, services }) as ReturnType<BifurcApi["healthbarSaveServices"]>,
    healthbarCheckUrl: (url) => call("healthbarCheckUrl", { url }) as ReturnType<BifurcApi["healthbarCheckUrl"]>,

    // ── webhook server ──────────────────────────────────────────────────────
    registerActiveWebhook: (webhookId, urlSuffix) =>
      call("registerActiveWebhook", { webhookId, urlSuffix }) as ReturnType<BifurcApi["registerActiveWebhook"]>,
    unregisterActiveWebhook: (webhookId) =>
      call("unregisterActiveWebhook", { webhookId }) as ReturnType<BifurcApi["unregisterActiveWebhook"]>,
    getWebhookServerStatus: () => call("getWebhookServerStatus", {}) as ReturnType<BifurcApi["getWebhookServerStatus"]>,
    webhookServerStatus: () => call("webhookServerStatus", {}) as ReturnType<BifurcApi["webhookServerStatus"]>,
    startWebhookServer: (port) => call("startWebhookServer", port === undefined ? {} : { port }) as ReturnType<BifurcApi["startWebhookServer"]>,
    stopWebhookServer: () => call("stopWebhookServer", {}) as ReturnType<BifurcApi["stopWebhookServer"]>,

    // ── SOAP / GraphQL / gRPC ───────────────────────────────────────────────
    soapFetchWsdl: (url) => call("soapFetchWsdl", { url }) as ReturnType<BifurcApi["soapFetchWsdl"]>,
    soapExecute: (endpointUrl, soapAction, headers, body) =>
      call("soapExecute", { endpointUrl, soapAction, headers, body }) as ReturnType<BifurcApi["soapExecute"]>,
    graphqlIntrospect: (endpointUrl, headers) =>
      call("graphqlIntrospect", { url: endpointUrl, headers }) as ReturnType<BifurcApi["graphqlIntrospect"]>,
    graphqlExecute: (endpointUrl, headers, query, variables, operationName) =>
      call("graphqlExecute", { url: endpointUrl, headers, query, variables, operationName }) as ReturnType<BifurcApi["graphqlExecute"]>,
    grpcExecute: (serverAddress, serviceName, methodName, requestBody, metadata, protoFileId, useReflection) =>
      call("grpcExecute", { serverAddress, serviceName, methodName, requestBody, metadata, protoFileId, useReflection }) as ReturnType<BifurcApi["grpcExecute"]>,
    grpcReflect: (serverAddress) => call("grpcReflect", { serverAddress }) as ReturnType<BifurcApi["grpcReflect"]>,
    grpcMockServerStatus: () => call("grpcMockServerStatus", {}) as ReturnType<BifurcApi["grpcMockServerStatus"]>,
    grpcStartMockServer: () => call("grpcStartMockServer", {}) as ReturnType<BifurcApi["grpcStartMockServer"]>,
    grpcStopMockServer: () => call("grpcStopMockServer", {}) as ReturnType<BifurcApi["grpcStopMockServer"]>,

    // ── TLS ─────────────────────────────────────────────────────────────────
    //
    // Three shims rather than three transport calls, and the extra step is a path the engine will
    // not report. `File_Ops_Protocol.md` §8 keeps the engine's directory layout out of the protocol,
    // so `tls.generate` answers `{ok, fingerprint}` and `tls.importCert` / `tls.importKey` answer
    // `{ok}`. The renderer needs a path regardless — `TlsSettingsSection.tsx` writes `tlsCaCertPath`
    // / `tlsCaKeyPath` from this result and treats a missing field as "not configured", so without
    // it the user generates a CA and the settings never learn it.
    //
    // The path is the **shell's** fact and the shell already reports it: `tlsCertStatus` is a
    // client-local hook whose `certPath` / `keyPath` are `caCertPath()` / `caKeyPath()` in
    // `certLifecycle.ts`. Asking it here keeps one implementation rather than a second copy — the
    // copy this replaces was the inline composition inside the legacy `tls:*` channels.
    //
    // These were plain `call(...)` passthroughs until 2026-09-20, which is exactly how the field went
    // missing when P6 step 3b-1 routed them. `surface.ts` now classifies all three as `shim`, so the
    // 1:1 mapping that caused it cannot be re-asserted by a later reader.
    tlsImportCert: async () => {
      const up = await uploadLocalFile();
      if ("canceled" in up) return { ok: false };
      if ("error" in up) return { ok: false, error: up.error };
      const r = (await call("tlsImportCert", { blobId: up.blobId })) as { ok: boolean; error?: string };
      if (!r.ok) return r;
      const { certPath } = await local.tlsCertStatus();
      return { ...r, path: certPath ?? undefined };
    },
    tlsImportKey: async () => {
      const up = await uploadLocalFile();
      if ("canceled" in up) return { ok: false };
      if ("error" in up) return { ok: false, error: up.error };
      const r = (await call("tlsImportKey", { blobId: up.blobId })) as { ok: boolean; error?: string };
      if (!r.ok) return r;
      const { keyPath } = await local.tlsCertStatus();
      return { ...r, path: keyPath ?? undefined };
    },
    tlsGenerate: async () => {
      const r = (await call("tlsGenerate", {})) as { ok: boolean; fingerprint?: string; error?: string };
      if (!r.ok) return r;
      const { certPath, keyPath } = await local.tlsCertStatus();
      return { ...r, certPath: certPath ?? undefined, keyPath: keyPath ?? undefined };
    },
    tlsExportCert: async () => {
      const r = await artifactToFile(await call("tlsExportCert", {}), "bifurc-ca.crt");
      return r.ok ? { ok: true, filePath: r.filePath } : { ok: false, canceled: r.canceled, error: r.error };
    },

    // ── client-local: delegated, never routed ───────────────────────────────
    checkUpdate: () => local.checkUpdate(),
    openExternal: (url) => local.openExternal(url),
    setTitleBarOverlay: (color, symbolColor) => local.setTitleBarOverlay(color, symbolColor),
    getTheme: () => local.getTheme(),
    setTheme: (themeId) => local.setTheme(themeId),
    tlsInstallCA: () => local.tlsInstallCA(),
    // Reclassified `transport` → `local` on 2026-09-20 (`plan/07` §5b-bis). Both are two-sided —
    // engine half plus the client's own OS trust store — so neither can be answered by routing, and
    // the routed forms delivered the engine half while silently dropping the other. `tlsRemoveCert`
    // was the live one: it left the OS store trusting a CA whose keypair it had just deleted.
    tlsRemoveCert: () => local.tlsRemoveCert(),
    tlsCertStatus: () => local.tlsCertStatus(),
    openFileDialog: () => local.openFileDialog(),
    pickFilePath: (title, filters) => local.pickFilePath(title, filters),
    pickFolderPath: (title) => local.pickFolderPath(title),
    platform: local.platform,
    isFirstLaunch: () => local.isFirstLaunch(),
    completeFirstLaunch: () => local.completeFirstLaunch(),
    getZoomLevel: () => local.getZoomLevel(),
    setZoomLevel: (level) => local.setZoomLevel(level),

    // ── collection runner ───────────────────────────────────────────────────
    saveRunnerReport: (wsId, report) => call("saveRunnerReport", { workspaceId: wsId, report }) as ReturnType<BifurcApi["saveRunnerReport"]>,
    getRunHistory: (wsId, folderId) => call("getRunHistory", { workspaceId: wsId, folderId }) as ReturnType<BifurcApi["getRunHistory"]>,
    exportRunnerReport: async (report) => {
      const r = report as { folderName?: string; startedAt?: number } | undefined;
      if (!local.pickSavePath || !local.writeArtifact) {
        return { ok: false, error: "this client cannot write files (no filesystem hooks)" };
      }

      // Dialog FIRST, then the engine renders — `File_Ops_Protocol.md` §3.2. The default path is
      // built here from the report, because the renderer sends the report and the dialog has to be
      // shown before the command runs. This is also the starkest case of the split: the chosen
      // **extension decides the format**, so the dialog cannot run after the render — by then the
      // format is already fixed. The client used to hardcode `json`, which silently removed HTML
      // export; that is what this replaces.
      const folderName = r?.folderName ?? "collection";
      const ts = new Date(r?.startedAt ?? Date.now()).toISOString().replace(/[:.]/g, "-");
      const dest = await local.pickSavePath({
        title: "Export Runner Report",
        defaultName: `${folderName}-report-${ts}.html`,
        filters: [
          { name: "HTML Report", extensions: ["html"] },
          { name: "JSON Report", extensions: ["json"] },
        ],
      });
      // A bare `{ok:false}` on cancel, not `{ok:false, canceled:true}`: the declared type carries no
      // `canceled` for this key, so the renderer cannot branch on one. The shell's legacy body
      // returned the same shape.
      if (!dest) return { ok: false };

      try {
        const artifact = await call("exportRunnerReport", {
          report,
          format: dest.toLowerCase().endsWith(".json") ? "json" : "html",
        });
        const res = await artifactToFile(artifact, baseName(dest), dest);
        return res.ok ? { ok: true, filePath: res.filePath } : { ok: false, error: res.error };
      } catch (err) {
        return { ok: false, error: (err as Error)?.message };
      }
    },
    saveRunnerConfig: (wsId, folderId, config) =>
      call("saveRunnerConfig", { workspaceId: wsId, folderId, config }) as ReturnType<BifurcApi["saveRunnerConfig"]>,
    loadRunnerConfig: (wsId, folderId) => call("loadRunnerConfig", { workspaceId: wsId, folderId }) as ReturnType<BifurcApi["loadRunnerConfig"]>,
    listRunnerFolderIds: (wsId) => call("listRunnerFolderIds", { workspaceId: wsId }) as ReturnType<BifurcApi["listRunnerFolderIds"]>,
    shareCaptureJson: async (entries, suggestedName) => {
      const r = await artifactToFile(
        await call("shareCaptureJson", suggestedName === undefined ? { entries } : { entries, suggestedName }),
        suggestedName ?? "capture.json",
      );
      return r.ok ? { ok: true, filePath: r.filePath } : { ok: false, canceled: r.canceled, error: r.error };
    },

    // ── the 7 subscriptions ─────────────────────────────────────────────────
    onSyncStatus: (cb) => plain("event.sync.status", cb as (p: never) => void),
    onEntitySyncStatus: (cb) => plain("event.sync.entityStatus", cb as (p: never) => void),
    onLogEntry,
    onLogChunk: (cb) => plain("event.log.chunk", cb as (p: never) => void),
    onServerError: (cb) => plain("event.server.error", cb as (p: never) => void),
    onWebhookPayload: (cb) => plain("event.webhook.payload", cb as (p: never) => void),
    /**
     * `event.entity.changed` carries what changed; the renderer's `onCompanionRefresh` only wants
     * the signal. Discarding the payload here matches what the legacy `companion:refresh` channel
     * did, so the renderer's behaviour is unchanged — it re-fetches either way.
     */
    onCompanionRefresh: (cb) => hub.subscribe("event.entity.changed", () => cb()),

    // ── lifecycle (not part of window.api) ──────────────────────────────────
    close: async () => {
      // Set *before* awaiting, so a request already in the retry loop stops retrying rather than
      // racing the teardown.
      closed = true;
      hub.close();
      await transport.close();
    },
    /**
     * Read through the hub rather than from a counter here, because the hub is where delivery happens —
     * a second counter would have to be updated at the same moment and could drift from it.
     */
    lastSeq: () => hub.lastSeq(),
  };

  /**
   * `close()` is **non-enumerable** on purpose, and this line is load-bearing rather than tidiness.
   *
   * The plan's regression signal for the whole critical path is *"`Object.keys(oldApi)` vs
   * `Object.keys(newApi)` reports zero differences"* — and `Object.keys` enumerates **enumerable own**
   * properties. So the plan's own chosen instrument defines the contract: the API surface *is* the
   * enumerable own keys. `window.api` has no `close`, therefore neither may the client, or the
   * criterion fails the moment P6 hands this object to the renderer.
   *
   * It still has to exist: the **shell** owns the client's lifetime and calls `close()` on quit. The
   * shell holds this object directly in the main process, whereas the renderer only ever sees what
   * `contextBridge.exposeInMainWorld` copies — and that walk follows enumerable own properties. So a
   * non-enumerable `close` is reachable by the process that needs it and invisible to the one that
   * must not see it. Keeping it enumerable would instead push a "strip `close` before exposing" step
   * into P6, i.e. move the correctness burden onto the exact phase that must change nothing.
   *
   * Note the descriptor carries only `enumerable`: per spec, redefining an *existing* property leaves
   * the attributes you omit untouched, so `value`/`writable`/`configurable` survive from the literal
   * above. That also keeps the literal annotated as `BifurcClient`, so the compiler is still the
   * shape test — no cast is needed to hide a property from a type.
   */
  Object.defineProperty(api, "close", { enumerable: false });
  Object.defineProperty(api, "lastSeq", { enumerable: false });

  return api;
}

/** Re-exported so a consumer can name the event shape without reaching into the engine. */
export type { EngineEvent };

/**
 * The client-local vocabulary, re-exported.
 *
 * `src/preload.ts` and the shell's handler module both implement these hooks, so both need to name
 * the option and result shapes. Without this they would reach for `@bifurc/client/dist/local`, i.e.
 * depend on the package's internal file layout rather than on its API.
 */
export type {
  ArtifactWriteResult,
  ClientLocal,
  FileFilterSpec,
  LocalFileContent,
  ReadFileOptions,
  ReadFileResult,
  RequiredClientLocal,
  SavePathOptions,
} from "./local";
