/**
 * `sync.*`, `git.*` — git-backed workspace sync. `entity.publish`/`entity.restore` and
 * `folder.publish` are kept as their existing dedicated names (they already delegate to the
 * generic git ops internally, per the shell's `syncHandlers.ts` — since deleted by the P6 reduction)
 * rather than folded into `git.*`, to avoid a rename with no behavioural benefit.
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

/** One entity file's position relative to HEAD. */
export type EntitySyncStatusValue = "clean" | "modified" | "new" | "deleted";

/**
 * The workspace's position relative to its **remote** — the second axis of "is this entity synced?".
 *
 * `EntitySyncStatusValue` only ever answers "does this file differ from HEAD?", which cannot
 * distinguish *committed and pushed* from *committed and not pushed*. Without that distinction a
 * committed-but-unpushed entity reads `clean` and the UI offers it no button at all, so it can only
 * be pushed from the panel-level footer control. Measured locally, never by fetching — see
 * `packages/engine/src/sync/gitState.ts` for why, and for what staleness that accepts.
 */
export interface WorkspaceGitState {
  /** An `origin` remote is configured. `ahead`/`behind` are both `false` when it is not. */
  hasRemote: boolean;
  branch: string | null;
  /** HEAD has commits the remote-tracking ref does not — there is something to push. */
  ahead: boolean;
  /** The remote-tracking ref has commits HEAD does not — there is something to pull. */
  behind: boolean;
}

/**
 * Both axes in one payload, and deliberately **one** command rather than two.
 *
 * They are read at the same instant from the same repo, so splitting them would let the renderer hold
 * a file map and a git state that disagree — which is exactly the class of bug this work removes.
 */
export type SyncGetEntityStatusResult = {
  files: Record<string, EntitySyncStatusValue>;
  git: WorkspaceGitState;
};

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
