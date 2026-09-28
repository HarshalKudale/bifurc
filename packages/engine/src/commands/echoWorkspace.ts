/**
 * The `workspaceId` echo for a status result, present **only when the caller supplied one**.
 *
 * ## What it is for
 *
 * `ServerStatusResult` and `WebhookServerStatusResult` both carry an optional `workspaceId` documented as
 * *"echoed so a multiplexed client can match response to request"* — and a client that is multiplexing is
 * exactly a client that sends an id. Returning it unconditionally would change the payload of every existing
 * status call **including the desktop's**, and `tests/ipc/handlers.test.ts` pins that payload with `toEqual`
 * precisely so a change to it is a decision rather than a side effect.
 *
 * ## Why it still exists now that it echoes and nothing more
 *
 * Until `plan/17` this took **two** arguments — the id the caller sent, and the id the request had *resolved
 * to* — because `runtimeRegistry.ts` routed a `workspaceId` to one of N per-workspace servers and the two
 * could differ. That registry is deleted: with one listener per port there is nothing to resolve, so the
 * resolved id and the requested id are the same value whenever one exists at all, and the second parameter
 * went with the routing.
 *
 * **The echo itself is not vestigial, though**, and that is why this is not simply deleted with the registry:
 * a multiplexed client still needs to match a response to the request that produced it, and
 * `plan/17` does not change that. What changed is only *what the id means* — from "which server" to "which
 * config context the caller was asking about".
 *
 * ## One home
 *
 * Defined here rather than in each command module because two copies of a response-shaping rule is drift:
 * `server.status` and `webhookServer.status` must agree about when the key is present, and nothing but a
 * shared function makes them.
 */
export function echoWorkspace(paramsWorkspaceId: string | undefined): { workspaceId?: string } {
  return paramsWorkspaceId === undefined ? {} : { workspaceId: paramsWorkspaceId };
}
