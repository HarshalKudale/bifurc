/**
 * The remaining ENGINE commands that do not fit a larger domain file:
 * `script.execute`, `request.replay`, `request.parseCurl`, `healthbar.*`, `capture.shareJson`
 * (SPLIT — engine half only).
 *
 * `request.parseCurl` joined on 2026-09-30 (`plan/19` WI 5): the renderer's `lib/curlParser.ts` was
 * deleted and the parser is now `packages/engine/src/utils/curl.ts`, shared with the cURL importer. It
 * sits beside `request.replay` because both are the `request.*` commands that are not the pipeline.
 *
 * `app.checkUpdate` was here until 2026-09-20, as the protocol's only other SPLIT, with an empty
 * params schema documented as "the interim engine-half contract". **The split was resolved in favour
 * of the client, and the command was deleted rather than implemented** — see
 * `@bifurc/client`'s `local.ts#checkUpdate` for the argument. In short: `currentVersion` is the
 * version of the thing the *user* runs, and `downloadUrl` must be the asset for the *user's*
 * platform. An engine can produce neither correctly — its own version only coincides under D9
 * lockstep, and matching its own `process.platform` would hand a Windows user the AppImage. So
 * there was no engine half to contract with, and keeping a schema for one invited exactly the bug
 * it would have caused: `AppCheckUpdateResult` had no `hasUpdate`, so routing it would have left
 * the About panel reporting "up to date" forever, silently.
 */
import { z } from "zod";
import type { ArtifactResult } from "./blob";
import { WorkspaceScope } from "./scope";

const HeaderMap = z.record(z.string(), z.string());

export const ScriptExecuteParams = z.object({
  script: z.string(),
  context: z.enum(["pre", "post", "test"]),
  request: z.object({
    method: z.string(),
    url: z.string(),
    headers: HeaderMap,
    body: z.string(),
  }).optional(),
  response: z.object({
    status: z.number(),
    headers: HeaderMap,
    body: z.string(),
    responseTime: z.number().optional(),
  }).optional(),
  envVars: z.record(z.string(), z.string()),
}).strict();
export type ScriptExecuteParams = z.infer<typeof ScriptExecuteParams>;
export interface ScriptExecuteResult {
  request?: { method: string; url: string; headers: Record<string, string>; body: string };
  response?: { status: number; headers: Record<string, string>; body: string };
  envVars: Record<string, string>;
  error?: string;
  testResults?: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs?: string[];
}

export const RequestReplayParams = z.object({
  method: z.string(),
  url: z.string(),
  headers: HeaderMap,
  body: z.string(),
}).strict();
export type RequestReplayParams = z.infer<typeof RequestReplayParams>;
export interface RequestReplayResult {
  status: number;
  headers: Record<string, string>;
  body: string;
  durationMs: number;
}

/**
 * `request.parseCurl` — parse a cURL command into the request it describes (`plan/19` WI 5).
 *
 * The renderer used to own this (`packages/gui/src/lib/curlParser.ts`), and the engine owned a
 * near-verbatim second copy inside the cURL importer. Both are gone: the parser is
 * `packages/engine/src/utils/curl.ts`, the importer calls it directly, and this command is the
 * renderer's way in.
 *
 * **Headers come back unfiltered**, and that is deliberate. `SKIP_CURL_HEADERS` is a *caller's*
 * decision — the tab drops hop-by-hop headers when it imports, the importer drops them at its own
 * point in the pipeline — so a parser that dropped them would be making a policy choice on the
 * caller's behalf. The renderer keeps a copy of that list for the one thing it still needs it for
 * (filtering a stored mock's captured headers); the copy is pinned to the engine's authority by
 * `tests/gui/curlHeaderSkip.test.ts`, the `plan/19` D28 list-vs-logic split.
 */
export const RequestParseCurlParams = z.object({ curl: z.string() }).strict();
export type RequestParseCurlParams = z.infer<typeof RequestParseCurlParams>;
export interface RequestParseCurlResult {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;
}

export const HealthbarGetServicesParams = z.object({ workspaceId: z.string() }).strict();
export type HealthbarGetServicesParams = z.infer<typeof HealthbarGetServicesParams>;
export type HealthbarGetServicesResult = Record<string, unknown>[]; // HealthBarService[]

export const HealthbarSaveServicesParams = z.object({
  workspaceId: z.string(),
  services: z.array(z.record(z.string(), z.unknown())),
}).strict();
export type HealthbarSaveServicesParams = z.infer<typeof HealthbarSaveServicesParams>;
export interface HealthbarSaveServicesResult {
  ok: boolean;
}

export const HealthbarCheckUrlParams = z.object({ url: z.string() }).strict();
export type HealthbarCheckUrlParams = z.infer<typeof HealthbarCheckUrlParams>;
export interface HealthbarCheckUrlResult {
  ok: boolean;
  statusCode: number | null;
  body: string | null;
  headers: Record<string, string> | null;
  error: string | null;
  durationMs: number;
}

/**
 * SPLIT — engine serializes the entries; client owns the save dialog (egress).
 *
 * `workspaceId` (P14 WI 8) is needed for the **blob half**, not for the serialization: above
 * `BLOB_INLINE_THRESHOLD_BYTES` this stages a blob, and the client then reads it with `blob.read`
 * scoped to its own workspace. Without the field the engine staged host-wide and the client's scoped
 * read found nothing — the engine's own export unreadable by the client that requested it, which is
 * the failure `export.create` already hit and fixed (`blob/publish.ts`).
 */
export const CaptureShareJsonParams = z.object({
  entries: z.array(z.unknown()),
  suggestedName: z.string().optional(),
}).merge(WorkspaceScope).strict();
export type CaptureShareJsonParams = z.infer<typeof CaptureShareJsonParams>;
export type CaptureShareJsonResult = ArtifactResult;

// ─────────────────────────────────────────────────────────────────────────────
// capture.buildRequest / capture.buildMock — capture → entity (`plan/19` WI 5, item 2)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * `capture.buildRequest` — turn a captured log entry into a saved-request payload.
 *
 * ## Why these two are commands
 *
 * The construction decides **which headers to drop**, **how to decode a body** and **how a response is
 * stored** — engine work by `plan/19` D22's test, and until this item it lived in
 * `packages/gui/src/components/capture/captureUtils.ts` as `reqToHeadersBody` and `buildMockInitial`. The
 * renderer cannot import the engine, so the only way to stop it holding a second implementation is to ask
 * over the wire.
 *
 * ## Why two commands rather than one with a discriminator
 *
 * The two produce **different shapes** — a `requests` body and a `mocks` body — and the two call sites need
 * one each: "Save" builds a request, "Open in mocks" builds a mock, and the context menu offers them as
 * separate actions. One command with a `target` field would return a union whose every reader has to
 * narrow it, for no caller that wants both.
 *
 * ## The `entry` is deliberately loose
 *
 * `z.record(z.string(), z.unknown())`, following `capture.shareJson`'s `z.array(z.unknown())`. The protocol
 * does not model `RequestLogEntry` — it is engine-local and the remote transports project it down before it
 * goes on the wire (`kernel/telemetry.ts` says so at length) — so a schema here would be a fourth mirror of
 * a type the wire deliberately does not carry. The *results* are loose for the same reason: the entity
 * bodies are typed in `@bifurc/engine` and hand-mirrored into `packages/gui/src/types/ipc.ts`, exactly as
 * `entity.create`'s `EntityCreateResult.entity` is a `Record<string, unknown>`.
 */
export const CaptureBuildRequestParams = z.union([
  z.object({ entry: z.record(z.string(), z.unknown()) }).strict(),
  z.object({ workspaceId: z.string(), id: z.string() }).strict(),
]);
export type CaptureBuildRequestParams = z.infer<typeof CaptureBuildRequestParams>;
export interface CaptureBuildRequestResult {
  /** A `requests` entity body, ready for `entity.create`. `name` is blank — the caller labels it. */
  request: Record<string, unknown>;
}

/**
 * `capture.buildMock` — turn a captured exchange into a mock payload.
 *
 * **The captured headers are stored raw, hop-by-hop ones included**, which is the one place the two
 * builders differ: a mock replays the capture byte-for-byte, so a `host` that was actually sent is part of
 * what was captured. The mock *editor* filters them for display instead.
 */
export const CaptureBuildMockParams = z.union([
  z.object({ entry: z.record(z.string(), z.unknown()) }).strict(),
  z.object({ workspaceId: z.string(), id: z.string() }).strict(),
]);
export type CaptureBuildMockParams = z.infer<typeof CaptureBuildMockParams>;
export interface CaptureBuildMockResult {
  /** A `mocks` entity body, ready for `entity.create`. */
  mock: Record<string, unknown>;
}

/**
 * `capture.list` / `capture.clear` / `capture.remove` — the engine's captured-traffic history for a workspace.
 *
 * The engine records every exchange itself (newest first, capped), so a client reads the history rather than
 * keeping one: a UI that was closed, or on another screen, still sees what passed. Live exchanges keep arriving
 * as `event.log.entry`, each carrying the `workspaceId` it was captured under.
 */
/**
 * `engine.shutdown` — stop a standalone engine gracefully: listeners close, runtime.json is removed, the process
 * exits. Served only by a standalone engine (`bifurc-engine`); an engine embedded in another process has no
 * process of its own to stop, so it does not register it.
 */
export const EngineShutdownParams = z.object({}).strict();
export type EngineShutdownParams = z.infer<typeof EngineShutdownParams>;

export const CaptureListParams = z.object({ workspaceId: z.string(), summary: z.boolean().optional() }).strict();
export type CaptureListParams = z.infer<typeof CaptureListParams>;
export interface CaptureListResult {
  entries: Record<string, unknown>[];
  /** Whether the workspace's capture is paused — the engine is not recording. */
  paused: boolean;
}

/** `capture.setPaused` — stop or resume the engine recording a workspace's traffic. Held in memory only. */
export const CaptureSetPausedParams = z.object({ workspaceId: z.string(), paused: z.boolean() }).strict();
export type CaptureSetPausedParams = z.infer<typeof CaptureSetPausedParams>;
export interface CaptureSetPausedResult {
  paused: boolean;
}

/** `capture.get` — one full captured entry by id, or `null`. */
export const CaptureGetParams = z.object({ workspaceId: z.string(), id: z.string() }).strict();
export type CaptureGetParams = z.infer<typeof CaptureGetParams>;
export interface CaptureGetResult {
  entry: Record<string, unknown> | null;
}

export const CaptureClearParams = z.object({ workspaceId: z.string() }).strict();
export type CaptureClearParams = z.infer<typeof CaptureClearParams>;

export const CaptureRemoveParams = z.object({
  workspaceId: z.string(),
  ids: z.array(z.string()).min(1),
}).strict();
export type CaptureRemoveParams = z.infer<typeof CaptureRemoveParams>;
export interface CaptureRemoveResult {
  ok: boolean;
  removed: number;
}
