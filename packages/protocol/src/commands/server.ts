/**
 * `server.*`, `proxy.status`, `services.discover` — proxy server lifecycle and health.
 */
import { z } from "zod";

export const ServerStatusParams = z.object({}).strict();
export type ServerStatusParams = z.infer<typeof ServerStatusParams>;
export interface ServerStatusResult {
  running: boolean;
  port: number;
  error: string | null;
}

export const ServerStartParams = z.object({}).strict();
export type ServerStartParams = z.infer<typeof ServerStartParams>;

export const ServerStopParams = z.object({}).strict();
export type ServerStopParams = z.infer<typeof ServerStopParams>;

export const ServerRestartParams = z.object({}).strict();
export type ServerRestartParams = z.infer<typeof ServerRestartParams>;

export interface ServerLifecycleResult {
  ok: boolean;
}

/** Subset of `server.status`. DROP candidate flagged in `plan/handler-classification.md` —
 * kept for now pending verification no external client depends on the narrower shape. */
export const ProxyStatusParams = z.object({}).strict();
export type ProxyStatusParams = z.infer<typeof ProxyStatusParams>;
export interface ProxyStatusResult {
  running: boolean;
}

export const ServicesDiscoverParams = z.object({}).strict();
export type ServicesDiscoverParams = z.infer<typeof ServicesDiscoverParams>;
export interface ServiceInfo {
  name: string;
  port: number;
  pid?: number;
}
export type ServicesDiscoverResult = ServiceInfo[];
