/**
 * `application.*` — run-configuration CRUD and process control. Engine-classified in full
 * because the engine is the host that spawns the process (D3) — see
 * `plan/handler-classification.md`. `processSpawner.ts` currently holds a `BrowserWindow`
 * reference that P2 must remove (checklist item "Remove the `mainWindow` reference from
 * `processSpawner.ts`").
 */
import { z } from "zod";

export const ApplicationListParams = z.object({
  workspaceId: z.string(),
}).strict();
export type ApplicationListParams = z.infer<typeof ApplicationListParams>;
export type ApplicationListResult = Record<string, unknown>[]; // ApplicationConfig[]

export const ApplicationSaveParams = z.object({
  application: z.record(z.string(), z.unknown()), // ApplicationConfig, see entity.ts note on z.record
}).strict();
export type ApplicationSaveParams = z.infer<typeof ApplicationSaveParams>;
export type ApplicationSaveResult = Record<string, unknown>;

export const ApplicationDeleteParams = z.object({
  workspaceId: z.string(),
  id: z.string(),
}).strict();
export type ApplicationDeleteParams = z.infer<typeof ApplicationDeleteParams>;
export interface ApplicationDeleteResult {
  ok: boolean;
}

export const ApplicationStartParams = z.object({
  workspaceId: z.string(),
  appId: z.string(),
  mode: z.enum(["run", "debug"]),
}).strict();
export type ApplicationStartParams = z.infer<typeof ApplicationStartParams>;

export const ApplicationStopParams = z.object({
  appId: z.string(),
}).strict();
export type ApplicationStopParams = z.infer<typeof ApplicationStopParams>;

export const ApplicationGetStateParams = z.object({
  appId: z.string(),
}).strict();
export type ApplicationGetStateParams = z.infer<typeof ApplicationGetStateParams>;

export const ApplicationGetAllStatesParams = z.object({}).strict();
export type ApplicationGetAllStatesParams = z.infer<typeof ApplicationGetAllStatesParams>;

export const ApplicationGetLogsParams = z.object({
  appId: z.string(),
}).strict();
export type ApplicationGetLogsParams = z.infer<typeof ApplicationGetLogsParams>;

export const ApplicationCheckPortParams = z.object({
  port: z.number().int(),
}).strict();
export type ApplicationCheckPortParams = z.infer<typeof ApplicationCheckPortParams>;

export const ApplicationKillPortParams = z.object({
  port: z.number().int(),
}).strict();
export type ApplicationKillPortParams = z.infer<typeof ApplicationKillPortParams>;
