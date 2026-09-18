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
 * `renderer/types/window.ts` disagrees with the runtime in **two** directions, on **seven** keys in
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
import type { BifurcApi } from "../../../renderer/types/window";
import { SURFACE } from "./surface";
import { createSubscriptionHub } from "./subscriptions";
import { DEFAULT_RETRY_POLICY, withRetry, type RetryPolicy } from "./retry";
import type { ArtifactWriteResult, ClientLocal, LocalFileContent } from "./local";

/**
 * The three keys `renderer/types/window.ts` marks **optional** (`?`) although `src/preload.ts`
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
 * below reads them out of `BifurcApi`, so they cannot drift from `renderer/types/window.ts`.
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
   */
  const artifactToFile = async (result: unknown, fallbackName: string): Promise<ArtifactWriteResult> => {
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

    return local.writeArtifact(contentBase64, r.suggestedName ?? fallbackName, r.mimeType ?? "application/octet-stream");
  };

  /** Ask for a file locally, upload it, and return the blob id the command needs. */
  const uploadLocalFile = async (): Promise<{ blobId: string; file: LocalFileContent } | { error: string } | { canceled: true }> => {
    if (!local.readArtifactFile) return { error: "this client cannot read files (no readArtifactFile hook)" };
    const file = await local.readArtifactFile();
    if (!file) return { canceled: true };
    if ("error" in file) return { error: file.error };
    const put = (await request("blob.put", {
      filename: file.name,
      mimeType: file.mimeType,
      size: file.size,
      data: file.base64,
    })) as { blobId?: string } | undefined;
    if (!put?.blobId) return { error: "blob.put did not return a blobId" };
    return { blobId: put.blobId, file };
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
    checkUpdate: () => call("checkUpdate", {}) as ReturnType<BifurcApi["checkUpdate"]>,
    getConfig: () => call("getConfig", {}) as ReturnType<BifurcApi["getConfig"]>,
    loadEntity: (wsId, kind, id) =>
      call("loadEntity", { workspaceId: wsId, kind, id }) as ReturnType<BifurcApi["loadEntity"]>,
    setEntityEnabled: (wsId, kind, id, enabled) =>
      call("setEntityEnabled", { workspaceId: wsId, kind, id, enabled }) as ReturnType<BifurcApi["setEntityEnabled"]>,
    saveConfig: (config) => call("saveConfig", { config }) as ReturnType<BifurcApi["saveConfig"]>,

    // ── import / export ─────────────────────────────────────────────────────
    getImportExportFormats: () => call("getImportExportFormats", {}) as ReturnType<BifurcApi["getImportExportFormats"]>,
    exportData: async (req) => {
      const result = await call("exportData", {
        kind: (req as { kind?: string }).kind,
        format: (req as { format?: string }).format,
        workspaceId: (req as { workspaceId?: string }).workspaceId,
        filename: (req as { filename?: string }).filename,
      });
      return artifactToFile(result, "export");
    },
    preflightImport: async (req) => {
      const uploaded = await uploadLocalFile();
      if ("canceled" in uploaded) return { ok: false, canceled: true };
      if ("error" in uploaded) return { ok: false, error: uploaded.error };
      return call("preflightImport", {
        kind: (req as { kind?: string }).kind,
        format: (req as { format?: string }).format,
        workspaceId: (req as { workspaceId?: string }).workspaceId,
        blobId: uploaded.blobId,
      }) as ReturnType<BifurcApi["preflightImport"]>;
    },
    importData: async (req) => {
      const uploaded = await uploadLocalFile();
      if ("canceled" in uploaded) return { ok: false, canceled: true };
      if ("error" in uploaded) return { ok: false, error: uploaded.error };
      return call("importData", {
        kind: (req as { kind?: string }).kind,
        format: (req as { format?: string }).format,
        workspaceId: (req as { workspaceId?: string }).workspaceId,
        blobId: uploaded.blobId,
        collisionStrategy: (req as { collisionStrategy?: string }).collisionStrategy,
      }) as ReturnType<BifurcApi["importData"]>;
    },
    discoverServices: () => call("discoverServices", {}) as ReturnType<BifurcApi["discoverServices"]>,

    // ── the CRUD collapse ───────────────────────────────────────────────────
    addMapping: (m) => call("addMapping", { kind: "mappings", entity: m }) as ReturnType<BifurcApi["addMapping"]>,
    updateMapping: (m) => call("updateMapping", { kind: "mappings", entity: m }) as ReturnType<BifurcApi["updateMapping"]>,
    deleteMapping: (id) => call("deleteMapping", { kind: "mappings", id }) as ReturnType<BifurcApi["deleteMapping"]>,
    addRule: (r) => call("addRule", { kind: "proxyRules", entity: r }) as ReturnType<BifurcApi["addRule"]>,
    updateRule: (r) => call("updateRule", { kind: "proxyRules", entity: r }) as ReturnType<BifurcApi["updateRule"]>,
    deleteRule: (id) => call("deleteRule", { kind: "proxyRules", id }) as ReturnType<BifurcApi["deleteRule"]>,
    addMock: (m) => call("addMock", { kind: "mocks", entity: m }) as ReturnType<BifurcApi["addMock"]>,
    updateMock: (m) => call("updateMock", { kind: "mocks", entity: m }) as ReturnType<BifurcApi["updateMock"]>,
    deleteMock: (id) => call("deleteMock", { kind: "mocks", id }) as ReturnType<BifurcApi["deleteMock"]>,
    addRequest: (r) => call("addRequest", { kind: "requests", entity: r }) as ReturnType<BifurcApi["addRequest"]>,
    updateRequest: (r) => call("updateRequest", { kind: "requests", entity: r }) as ReturnType<BifurcApi["updateRequest"]>,
    deleteRequest: (id) => call("deleteRequest", { kind: "requests", id }) as ReturnType<BifurcApi["deleteRequest"]>,
    addWsConnection: (c) => call("addWsConnection", { kind: "wsConnections", entity: c }) as ReturnType<BifurcApi["addWsConnection"]>,
    updateWsConnection: (c) => call("updateWsConnection", { kind: "wsConnections", entity: c }) as ReturnType<BifurcApi["updateWsConnection"]>,
    deleteWsConnection: (id) => call("deleteWsConnection", { kind: "wsConnections", id }) as ReturnType<BifurcApi["deleteWsConnection"]>,
    addWebhook: (h) => call("addWebhook", { kind: "webhooks", entity: h }) as ReturnType<BifurcApi["addWebhook"]>,
    updateWebhook: (h) => call("updateWebhook", { kind: "webhooks", entity: h }) as ReturnType<BifurcApi["updateWebhook"]>,
    deleteWebhook: (id) => call("deleteWebhook", { kind: "webhooks", id }) as ReturnType<BifurcApi["deleteWebhook"]>,
    addGraphQLRequest: (r) => call("addGraphQLRequest", { kind: "graphqlRequests", entity: r }) as ReturnType<BifurcApi["addGraphQLRequest"]>,
    updateGraphQLRequest: (r) => call("updateGraphQLRequest", { kind: "graphqlRequests", entity: r }) as ReturnType<BifurcApi["updateGraphQLRequest"]>,
    deleteGraphQLRequest: (id) => call("deleteGraphQLRequest", { kind: "graphqlRequests", id }) as ReturnType<BifurcApi["deleteGraphQLRequest"]>,
    addGraphQLMock: (m) => call("addGraphQLMock", { kind: "graphqlMocks", entity: m }) as ReturnType<BifurcApi["addGraphQLMock"]>,
    updateGraphQLMock: (m) => call("updateGraphQLMock", { kind: "graphqlMocks", entity: m }) as ReturnType<BifurcApi["updateGraphQLMock"]>,
    deleteGraphQLMock: (id) => call("deleteGraphQLMock", { kind: "graphqlMocks", id }) as ReturnType<BifurcApi["deleteGraphQLMock"]>,
    addGraphQLSchema: (s) => call("addGraphQLSchema", { kind: "graphqlSchemas", entity: s }) as ReturnType<BifurcApi["addGraphQLSchema"]>,
    deleteGraphQLSchema: (id) => call("deleteGraphQLSchema", { kind: "graphqlSchemas", id }) as ReturnType<BifurcApi["deleteGraphQLSchema"]>,
    listGraphQLSchemas: async () =>
      ((await call("listGraphQLSchemas", { kind: "graphqlSchemas", workspaceId: await resolveWorkspaceId() })) as { entities: unknown[] }).entities as Awaited<ReturnType<BifurcApi["listGraphQLSchemas"]>>,
    addGrpcRequest: (r) => call("addGrpcRequest", { kind: "grpcRequests", entity: r }) as ReturnType<BifurcApi["addGrpcRequest"]>,
    updateGrpcRequest: (r) => call("updateGrpcRequest", { kind: "grpcRequests", entity: r }) as ReturnType<BifurcApi["updateGrpcRequest"]>,
    deleteGrpcRequest: (id) => call("deleteGrpcRequest", { kind: "grpcRequests", id }) as ReturnType<BifurcApi["deleteGrpcRequest"]>,
    addGrpcMock: (m) => call("addGrpcMock", { kind: "grpcMocks", entity: m }) as ReturnType<BifurcApi["addGrpcMock"]>,
    updateGrpcMock: (m) => call("updateGrpcMock", { kind: "grpcMocks", entity: m }) as ReturnType<BifurcApi["updateGrpcMock"]>,
    deleteGrpcMock: (id) => call("deleteGrpcMock", { kind: "grpcMocks", id }) as ReturnType<BifurcApi["deleteGrpcMock"]>,
    addProtoFile: (p) => call("addProtoFile", { kind: "protoFiles", entity: p }) as ReturnType<BifurcApi["addProtoFile"]>,
    deleteProtoFile: (id) => call("deleteProtoFile", { kind: "protoFiles", id }) as ReturnType<BifurcApi["deleteProtoFile"]>,
    listProtoFiles: async () =>
      ((await call("listProtoFiles", { kind: "protoFiles", workspaceId: await resolveWorkspaceId() })) as { entities: unknown[] }).entities as Awaited<ReturnType<BifurcApi["listProtoFiles"]>>,
    addSoapRequest: (r) => call("addSoapRequest", { kind: "soapRequests", entity: r }) as ReturnType<BifurcApi["addSoapRequest"]>,
    updateSoapRequest: (r) => call("updateSoapRequest", { kind: "soapRequests", entity: r }) as ReturnType<BifurcApi["updateSoapRequest"]>,
    deleteSoapRequest: (id) => call("deleteSoapRequest", { kind: "soapRequests", id }) as ReturnType<BifurcApi["deleteSoapRequest"]>,
    addSoapMock: (m) => call("addSoapMock", { kind: "soapMocks", entity: m }) as ReturnType<BifurcApi["addSoapMock"]>,
    updateSoapMock: (m) => call("updateSoapMock", { kind: "soapMocks", entity: m }) as ReturnType<BifurcApi["updateSoapMock"]>,
    deleteSoapMock: (id) => call("deleteSoapMock", { kind: "soapMocks", id }) as ReturnType<BifurcApi["deleteSoapMock"]>,
    addWsdl: (w) => call("addWsdl", { kind: "wsdls", entity: w }) as ReturnType<BifurcApi["addWsdl"]>,
    deleteWsdl: (id) => call("deleteWsdl", { kind: "wsdls", id }) as ReturnType<BifurcApi["deleteWsdl"]>,
    listWsdls: async () =>
      ((await call("listWsdls", { kind: "wsdls", workspaceId: await resolveWorkspaceId() })) as { entities: unknown[] }).entities as Awaited<ReturnType<BifurcApi["listWsdls"]>>,
    addEnvironment: (e) => call("addEnvironment", { kind: "environments", entity: e }) as ReturnType<BifurcApi["addEnvironment"]>,
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
      const r = await artifactToFile(await call("exportAudit", { format }), `audit.${format}`);
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
    tlsImportCert: async () => {
      const up = await uploadLocalFile();
      if ("canceled" in up) return { ok: false };
      if ("error" in up) return { ok: false };
      return call("tlsImportCert", { blobId: up.blobId }) as ReturnType<BifurcApi["tlsImportCert"]>;
    },
    tlsImportKey: async () => {
      const up = await uploadLocalFile();
      if ("canceled" in up) return { ok: false };
      if ("error" in up) return { ok: false };
      return call("tlsImportKey", { blobId: up.blobId }) as ReturnType<BifurcApi["tlsImportKey"]>;
    },
    tlsRemoveCert: () => call("tlsRemoveCert", {}) as ReturnType<BifurcApi["tlsRemoveCert"]>,
    tlsGenerate: () => call("tlsGenerate", {}) as ReturnType<BifurcApi["tlsGenerate"]>,
    tlsExportCert: async () => {
      const r = await artifactToFile(await call("tlsExportCert", {}), "bifurc-ca.crt");
      return r.ok ? { ok: true, filePath: r.filePath } : { ok: false, canceled: r.canceled, error: r.error };
    },
    tlsCertStatus: () => call("tlsCertStatus", {}) as ReturnType<BifurcApi["tlsCertStatus"]>,

    // ── client-local: delegated, never routed ───────────────────────────────
    openExternal: (url) => local.openExternal(url),
    setTitleBarOverlay: (color, symbolColor) => local.setTitleBarOverlay(color, symbolColor),
    getTheme: () => local.getTheme(),
    setTheme: (themeId) => local.setTheme(themeId),
    tlsInstallCA: () => local.tlsInstallCA(),
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
      const r = await artifactToFile(await call("exportRunnerReport", { report, format: "json" }), "runner-report.json");
      return r.ok ? { ok: true, filePath: r.filePath } : { ok: false, canceled: r.canceled, error: r.error };
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
     * the signal. Discarding the payload here is what `src/ipc/eventBridge.ts` already does for the
     * legacy IPC channel, so the renderer's behaviour is unchanged — it re-fetches either way.
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

  return api;
}

/** Re-exported so a consumer can name the event shape without reaching into the engine. */
export type { EngineEvent };
