/**
 * `webhook.*`, `webhookServer.*` — active-webhook registration and the local webhook listener.
 *
 * `workspaceId` follows the same rule as `server.ts` (P14 D13, WI 1): optional, omitted meaning "the active
 * workspace". It is what makes the routing key `(workspaceId, urlSuffix)` expressible on the wire — a suffix
 * registered in workspace A must be `404` in workspace B, and without the parameter a multiplexed host has no
 * way to say which one a registration belongs to.
 *
 * **`WorkspaceScope` is imported, not redeclared (2026-09-21).** This file used to carry its own copy of the
 * object, which made two definitions of one contract — the drift hazard `./scope`'s own header names, and
 * the reason that module exists. Found while wiring P14 WI 7 (which had to read the field in
 * `webhookCommands.ts`); the two copies were still identical, so this is a consolidation rather than a bug
 * fix, but identical-and-separate is exactly the state that precedes identical-no-longer.
 */
import { z } from "zod";
import { WorkspaceScope } from "./scope";

export const WebhookRegisterActiveParams = z.object({
  webhookId: z.string(),
  urlSuffix: z.string(),
}).merge(WorkspaceScope).strict();
export type WebhookRegisterActiveParams = z.infer<typeof WebhookRegisterActiveParams>;

export const WebhookUnregisterActiveParams = z.object({
  webhookId: z.string(),
}).merge(WorkspaceScope).strict();
export type WebhookUnregisterActiveParams = z.infer<typeof WebhookUnregisterActiveParams>;

export interface WebhookActionResult {
  ok: boolean;
}

export const WebhookServerStartParams = z.object({
  port: z.number().int().optional(),
}).merge(WorkspaceScope).strict();
export type WebhookServerStartParams = z.infer<typeof WebhookServerStartParams>;

export const WebhookServerStopParams = WorkspaceScope.strict();
export type WebhookServerStopParams = z.infer<typeof WebhookServerStopParams>;

export const WebhookServerStatusParams = WorkspaceScope.strict();
export type WebhookServerStatusParams = z.infer<typeof WebhookServerStatusParams>;
export interface WebhookServerStatusResult {
  running: boolean;
  port: number;
  error: string | null;
  /** The workspace this status describes — echoed so a multiplexed client can match response to request. */
  workspaceId?: string;
}
