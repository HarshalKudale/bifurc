/**
 * `webhook.*`, `webhookServer.*` — active-webhook registration and the local webhook listener.
 *
 * `workspaceId` follows the same rule as `server.ts` (P14 D13, WI 1): optional, omitted meaning "the active
 * workspace". It is what makes the routing key `(workspaceId, urlSuffix)` expressible on the wire — a suffix
 * registered in workspace A must be `404` in workspace B, and without the parameter a multiplexed host has no
 * way to say which one a registration belongs to.
 */
import { z } from "zod";

const WorkspaceScope = z.object({
  /** The workspace to act on. Omitted → the active workspace (the desktop's behaviour). */
  workspaceId: z.string().optional(),
});

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
