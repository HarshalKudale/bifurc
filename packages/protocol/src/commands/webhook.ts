/**
 * `webhook.*`, `webhookServer.*` — active-webhook registration and the local webhook listener.
 */
import { z } from "zod";

export const WebhookRegisterActiveParams = z.object({
  webhookId: z.string(),
  urlSuffix: z.string(),
}).strict();
export type WebhookRegisterActiveParams = z.infer<typeof WebhookRegisterActiveParams>;

export const WebhookUnregisterActiveParams = z.object({
  webhookId: z.string(),
}).strict();
export type WebhookUnregisterActiveParams = z.infer<typeof WebhookUnregisterActiveParams>;

export interface WebhookActionResult {
  ok: boolean;
}

export const WebhookServerStartParams = z.object({
  port: z.number().int().optional(),
}).strict();
export type WebhookServerStartParams = z.infer<typeof WebhookServerStartParams>;

export const WebhookServerStopParams = z.object({}).strict();
export type WebhookServerStopParams = z.infer<typeof WebhookServerStopParams>;

export const WebhookServerStatusParams = z.object({}).strict();
export type WebhookServerStatusParams = z.infer<typeof WebhookServerStatusParams>;
export interface WebhookServerStatusResult {
  running: boolean;
  port: number;
  error: string | null;
}
