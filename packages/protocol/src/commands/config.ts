/**
 * `config.*`, `workspace.*`, `env.setActive` — top-level config and workspace lifecycle.
 * Config itself stays `z.record` at the wire boundary (see the note in `entity.ts`); it is a
 * large, evolving shape (`AppConfig`) that today's engine already owns and validates on load.
 */
import { z } from "zod";

export const ConfigGetParams = z.object({}).strict();
export type ConfigGetParams = z.infer<typeof ConfigGetParams>;
export type ConfigGetResult = Record<string, unknown>; // AppConfig

export const ConfigSaveParams = z.object({
  config: z.record(z.string(), z.unknown()),
});
export type ConfigSaveParams = z.infer<typeof ConfigSaveParams>;
export interface ConfigSaveResult {
  ok: boolean;
}
// NOTE (work item 1, SPLIT): the legacy `config:save` also calls `updateTrayMenu()` — a
// shell-only side effect. The ENGINE half is config persistence + server restart; the shell
// subscribes to `event.config.changed` (new, added alongside `entity.changed`) to update its
// own tray instead of the engine reaching into `@/main`.

export const WorkspaceAddParams = z.object({
  name: z.string(),
});
export type WorkspaceAddParams = z.infer<typeof WorkspaceAddParams>;
export interface WorkspaceAddResult {
  id: string;
  name: string;
  createdAt: number;
  activeEnvironmentId: string | null;
}

export const WorkspaceRenameParams = z.object({
  id: z.string(),
  name: z.string(),
});
export type WorkspaceRenameParams = z.infer<typeof WorkspaceRenameParams>;
export interface WorkspaceRenameResult {
  ok: boolean;
}

export const WorkspaceDeleteParams = z.object({
  id: z.string(),
});
export type WorkspaceDeleteParams = z.infer<typeof WorkspaceDeleteParams>;
export interface WorkspaceDeleteResult {
  ok: boolean;
}

export const WorkspaceSetActiveParams = z.object({
  id: z.string(),
});
export type WorkspaceSetActiveParams = z.infer<typeof WorkspaceSetActiveParams>;
export interface WorkspaceSetActiveResult {
  ok: boolean;
  config: Record<string, unknown>;
}

export const EnvSetActiveParams = z.object({
  id: z.string().nullable(),
});
export type EnvSetActiveParams = z.infer<typeof EnvSetActiveParams>;
export interface EnvSetActiveResult {
  ok: boolean;
}
