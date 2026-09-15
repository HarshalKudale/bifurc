/**
 * `sync.*`, `git.*` — git-backed workspace sync. `entity.publish`/`entity.restore` and
 * `folder.publish` are kept as their existing dedicated names (they already delegate to the
 * generic git ops internally, per `src/ipc/handlers/syncHandlers.ts`) rather than folded into
 * `git.*`, to avoid a rename with no behavioural benefit.
 */
import { z } from "zod";

export const SyncSetRemoteParams = z.object({
  workspaceId: z.string(),
  remote: z.string(),
  branch: z.string(),
}).strict();
export type SyncSetRemoteParams = z.infer<typeof SyncSetRemoteParams>;
export interface SyncSetRemoteResult {
  ok: boolean;
  cloned?: boolean;
  adoptedId?: string;
  error?: string;
}

export const SyncDisconnectParams = z.object({ workspaceId: z.string() }).strict();
export type SyncDisconnectParams = z.infer<typeof SyncDisconnectParams>;

export const SyncPushParams = z.object({ workspaceId: z.string() }).strict();
export type SyncPushParams = z.infer<typeof SyncPushParams>;
export interface SyncPushResult {
  ok: boolean;
  error?: string;
}

export const SyncPullParams = z.object({ workspaceId: z.string() }).strict();
export type SyncPullParams = z.infer<typeof SyncPullParams>;
export interface SyncPullResult {
  ok: boolean;
  updated?: boolean;
  error?: string;
}

export const SyncGetStateParams = z.object({ workspaceId: z.string() }).strict();
export type SyncGetStateParams = z.infer<typeof SyncGetStateParams>;
export type SyncGetStateResult = Record<string, unknown>; // SyncState

export const SyncSetAutoSyncParams = z.object({
  workspaceId: z.string(),
  enabled: z.boolean(),
}).strict();
export type SyncSetAutoSyncParams = z.infer<typeof SyncSetAutoSyncParams>;
export interface SyncSetAutoSyncResult {
  ok: boolean;
}

export const SyncGetEntityStatusParams = z.object({ workspaceId: z.string() }).strict();
export type SyncGetEntityStatusParams = z.infer<typeof SyncGetEntityStatusParams>;
export type SyncGetEntityStatusResult = Record<string, "clean" | "modified" | "new" | "deleted">;

export const GitDiffParams = z.object({
  workspaceId: z.string(),
  relPath: z.string(),
}).strict();
export type GitDiffParams = z.infer<typeof GitDiffParams>;
export interface GitDiffResult {
  hasDiff: boolean;
  status: "clean" | "modified" | "new" | "deleted";
  diff?: string;
  original?: string | null;
  current?: string | null;
}

export const GitDiscardParams = z.object({
  workspaceId: z.string(),
  relPath: z.string(),
}).strict();
export type GitDiscardParams = z.infer<typeof GitDiscardParams>;
export interface GitActionResult {
  ok: boolean;
  error?: string;
}

export const GitSyncParams = z.object({
  workspaceId: z.string(),
  paths: z.array(z.string()),
  message: z.string().optional(),
}).strict();
export type GitSyncParams = z.infer<typeof GitSyncParams>;

export const GitHistoryParams = z.object({
  workspaceId: z.string(),
  relPath: z.string(),
  limit: z.number().int().optional(),
  offset: z.number().int().optional(),
}).strict();
export type GitHistoryParams = z.infer<typeof GitHistoryParams>;

export const EntityPublishParams = z.object({
  workspaceId: z.string(),
  paths: z.array(z.string()),
  message: z.string().optional(),
}).strict();
export type EntityPublishParams = z.infer<typeof EntityPublishParams>;

export const EntityRestoreParams = z.object({
  workspaceId: z.string(),
  relPath: z.string(),
}).strict();
export type EntityRestoreParams = z.infer<typeof EntityRestoreParams>;
