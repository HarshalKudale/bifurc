/**
 * `folder.*` — not collapsed into the generic entity CRUD because folders have their own
 * config keys and a distinct singular-kind discriminator (`FolderKind` in
 * `src/ipc/handlers/folderHandlers.ts`). See the P1 naming-pass note in
 * `plan/handler-classification.md` about the asymmetry with `entity.setEnabled`'s plural kind.
 */
import { z } from "zod";

export const FolderKind = z.enum([
  "mock",
  "request",
  "ws",
  "webhook",
  "rule",
  "graphqlRequest",
  "graphqlMock",
  "grpcRequest",
  "grpcMock",
  "soapRequest",
  "soapMock",
]);
export type FolderKindValue = z.infer<typeof FolderKind>;

export const FolderAddParams = z.object({
  kind: FolderKind,
  name: z.string(),
  parentId: z.string().nullable(),
  workspaceId: z.string().optional(),
});
export type FolderAddParams = z.infer<typeof FolderAddParams>;
export interface FolderAddResult {
  id: string;
  name: string;
  parentId: string | null;
  createdAt: number;
  workspaceId: string;
}

export const FolderRenameParams = z.object({
  kind: FolderKind,
  id: z.string(),
  name: z.string(),
});
export type FolderRenameParams = z.infer<typeof FolderRenameParams>;
export interface FolderRenameResult {
  ok: boolean;
}

export const FolderMoveParams = z.object({
  kind: FolderKind,
  id: z.string(),
  parentId: z.string().nullable(),
});
export type FolderMoveParams = z.infer<typeof FolderMoveParams>;
export interface FolderMoveResult {
  ok: boolean;
}

export const FolderDeleteParams = z.object({
  kind: FolderKind,
  id: z.string(),
});
export type FolderDeleteParams = z.infer<typeof FolderDeleteParams>;
export interface FolderDeleteResult {
  ok: boolean;
}

/** SPLIT-adjacent: publishes a folder's contents to the configured git remote. ENGINE-side. */
export const FolderPublishParams = z.object({
  workspaceId: z.string(),
  kind: z.string(),
  folderName: z.string().nullable(),
});
export type FolderPublishParams = z.infer<typeof FolderPublishParams>;
export interface FolderPublishResult {
  ok: boolean;
  error?: string;
}
