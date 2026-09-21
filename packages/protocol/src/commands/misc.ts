/**
 * The remaining ENGINE commands that do not fit a larger domain file:
 * `script.execute`, `request.replay`, `healthbar.*`, `capture.shareJson` (SPLIT — engine half only).
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
