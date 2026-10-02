/**
 * `server.*`, `proxy.status`, `services.discover` — proxy server lifecycle and health.
 *
 * ## `workspaceId` — the per-workspace scoping (P14 D13, WI 1)
 *
 * A host runs N workspaces behind **one** RPC endpoint, so these commands are addressed to a workspace and
 * carry its id in their params. It is **optional**, and that is the whole compatibility story: omitted means
 * "the active workspace", which is what the single-workspace desktop means today. A host member's client
 * always sends one, because for it there is no meaningful default.
 *
 * These fields were `z.object({}).strict()`, so adding a key is a **schema change and not a widening**:
 * `.strict()` turns an unknown key into a rejection, which means the `invalid: { extra: true }` fixtures
 * (`fixtures.ts:57-62`) keep rejecting — they assert "an unknown key fails", which stays true for a key that
 * is still unknown. That is why those fixtures did **not** need re-deriving, while `config.get`'s does not
 * change either. Verify per command rather than trusting the pattern: a fixture whose invalid payload happens
 * to be the newly allowed key would silently stop testing rejection.
 *
 * `WorkspaceScope` itself lives in `./scope` — three command files need it now, and the `.merge()` ordering
 * trap that goes with it is documented there.
 */
import { z } from "zod";
import { WorkspaceScope } from "./scope";

/**
 * Which listener a `server.*` command is about. Omitted means the forward proxy, which is what these commands
 * always meant; `reverse` is the edge that serves the `.localhost` names on `settings.port`.
 */
export const ServerTarget = z.object({ target: z.enum(["forward", "reverse"]).optional() });

export const ServerStatusParams = WorkspaceScope.merge(ServerTarget).strict();
export type ServerStatusParams = z.infer<typeof ServerStatusParams>;
export interface ServerStatusResult {
  running: boolean;
  port: number;
  error: string | null;
  /** The workspace this status describes — echoed so a multiplexed client can match response to request. */
  workspaceId?: string;
}

export const ServerStartParams = WorkspaceScope.merge(ServerTarget).strict();
export type ServerStartParams = z.infer<typeof ServerStartParams>;

export const ServerStopParams = WorkspaceScope.merge(ServerTarget).strict();
export type ServerStopParams = z.infer<typeof ServerStopParams>;

export const ServerRestartParams = WorkspaceScope.merge(ServerTarget).strict();
export type ServerRestartParams = z.infer<typeof ServerRestartParams>;

export interface ServerLifecycleResult {
  ok: boolean;
}

/** Subset of `server.status`. DROP candidate flagged in `plan/handler-classification.md` —
 * kept for now pending verification no external client depends on the narrower shape. */
export const ProxyStatusParams = WorkspaceScope.strict();
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
