/**
 * The remaining ENGINE and SPLIT commands that do not fit a larger domain file:
 * `script.execute`, `request.replay`, `healthbar.*`, `app.checkUpdate` (SPLIT — engine half
 * only), `capture.shareJson` (SPLIT — engine half only).
 */
import { z } from "zod";
import type { ArtifactResult } from "./blob";

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

/** SPLIT (work item 1): the GitHub API fetch is engine-safe outbound network; the
 * `process.platform`/`app.getVersion()` comparison is client-local. P12 moves this fully
 * client-side per the checklist ("`app:checkUpdate` moved client-side") — this schema is the
 * interim engine-half contract. */
export const AppCheckUpdateParams = z.object({}).strict();
export type AppCheckUpdateParams = z.infer<typeof AppCheckUpdateParams>;
export interface AppCheckUpdateResult {
  ok: boolean;
  tagName?: string;
  releaseName?: string;
  releaseNotes?: string;
  publishedAt?: string;
  htmlUrl?: string;
  downloadUrl?: string;
  assetName?: string;
  error?: string;
}

/** SPLIT — engine serializes the entries; client owns the save dialog (egress). */
export const CaptureShareJsonParams = z.object({
  entries: z.array(z.unknown()),
  suggestedName: z.string().optional(),
}).strict();
export type CaptureShareJsonParams = z.infer<typeof CaptureShareJsonParams>;
export type CaptureShareJsonResult = ArtifactResult;
