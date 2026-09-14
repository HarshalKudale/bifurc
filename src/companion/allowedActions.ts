/**
 * Allowlist of IPC actions that the companion browser extension is permitted to invoke.
 * Only safe, additive operations are exposed — no deletes, no settings changes.
 */
export const ALLOWED_ACTIONS: ReadonlySet<string> = new Set([
    "mock:add",
    "request:add",
    "folder:add",
    "config:get",
]);
