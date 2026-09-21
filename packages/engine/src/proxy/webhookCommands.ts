/**
 * The five webhook commands — `webhook.registerActive`, `webhook.unregisterActive` and
 * `webhookServer.start` / `webhookServer.status` / `webhookServer.stop`.
 *
 * ## Why these moved out of the shell (P6 finding 5)
 *
 * They were served by `ipcMain.handle` bodies in `src/ipc/handlers/crudHandlers.ts` — now
 * `crudKinds.ts`, and those bodies are gone (P6 reduction, fourth group, 2026-09-21) — which worked
 * only while the shell and the engine were the same process. They are not shell concerns: every function
 * they call already lives in `packages/engine/src/proxy/webhookServer.ts`, including the listening
 * server and the module-level `currentPort` / `lastError` state. P9's container is the case that makes
 * it concrete — a containerised engine that cannot answer `webhookServer.status` cannot be supervised,
 * and one that cannot `webhookServer.start` cannot serve webhooks at all.
 *
 * The engine's own `transport/auth/scopes.ts` had already assigned all five scopes (`write` for the
 * two register/unregister pairs, `admin` for start/stop, `read` for status) before this module existed.
 * That table is written against `@bifurc/protocol` command names, so it is independent evidence these
 * were always engine commands — the same signal `serverCommands.ts` found for its six.
 *
 * ## The bodies are moved verbatim, with one addition
 *
 * `registerActive` / `unregisterActive` are character-for-character moves, and so is `status`.
 *
 * `webhookServer.start` is the one place this file is not a literal copy. The shell body read the port
 * from `loadConfig()` (`cfg.webhookPort ?? 9101`) and **ignored** any argument — its channel took none.
 * `WebhookServerStartParams` declares `port` as `z.number().int().optional()`, so the schema promises a
 * caller may supply one. This implementation honours it and falls back to the config chain when it is
 * absent, which means every existing caller (they all pass nothing) gets exactly the old behaviour,
 * and a remote client can now do the thing the frozen schema already advertises. Honouring a declared
 * parameter is not a behaviour change; ignoring one would be a schema the implementation contradicts.
 *
 * ## What is deliberately absent
 *
 * Nothing here touches `event.webhook.payload`. That event already reaches the bus from
 * `webhookServer.ts` itself, so it needed no work in this phase — the webhook commands are request
 * paths, and the payload push is an event path.
 *
 * ## The trap that no longer applies — and why it was right while it did
 *
 * The obvious "DRY" follow-up was to replace the shell's five `ipcMain.handle` bodies with
 * `commandRegistry.invoke("<command>", params, ctx)`. **That was tried in `serverCommands.ts` and
 * reverted, and it is a real regression rather than a style question.**
 *
 * The reason was how this module gets registered. `config.get` / `env.setActive` / `workspace.setActive`
 * were registered at **module scope** in `coreHandlers.ts`, so importing that file populated them. This
 * module is registered by an **explicit call inside `registerIpcHandlers()`**, so a legacy body that
 * delegated here acquired a hidden ordering dependency on a function callers are not obliged to run —
 * and two suites already registered handler *groups* without it:
 * `tests/integration/settingsMutations.integration.test.ts`, and `tests/spike/protocolPoc.test.ts`
 * (retired 2026-09-21 with the rest of the P0 spike — see `spike/README.md`). The failure was a
 * `No handler registered` at *invocation* time, so nothing caught it at build time and the shell's own
 * handler file looked untouched.
 *
 * So the duplication was deliberate and bounded: five one-line bodies, behaviourally identical to the
 * handlers below. **It is dissolved rather than answered.** The reduction's fourth group deleted the
 * five `webhook*` channels on 2026-09-21 (`plan/07` §5b-quindecies), so there is one implementation with
 * no second entry point to keep in sync — which is what step 3 was going to achieve by deleting both
 * halves together. §5b-sedecies generalised it: **"duplication kept deliberately to avoid a hidden
 * coupling" is a temporary answer that a deletion can retire, and retiring it needs no new coupling.**
 * `coreHandlers.ts` itself became `coreKinds.ts` in the sixth group; the module-scope registration this
 * paragraph refers to no longer exists anywhere in the shell, which is §5c's whole point.
 *
 * ## P14 WI 7 — all five now resolve a runtime from `workspaceId`, and the routing maps go with it
 *
 * The same gap `serverCommands.ts` records, in a second module: these called the module facade
 * (`webhookServer.ts`'s `startWebhookServer` / `isWebhookServerRunning` / `getWebhookPort`), whose lazy
 * default instance is `createWebhookServer({ workspaceId: null })` — the active workspace. WI 1's protocol
 * half gave all five commands an optional `workspaceId`; this is what makes them read it.
 *
 * **The two `registerActive` / `unregisterActive` commands are the ones where the old behaviour was worst.**
 * They mutate a receiver's **active map**, and WI 2 deliberately left the routing key as WI 1's job rather
 * than keying it by `(workspaceId, urlSuffix)` — so on a host, `webhook.registerActive` from workspace A
 * wrote into the *active* workspace's map. A webhook registered by A could therefore be served by B, or
 * counted against B's `MAX_ACTIVE_WEBHOOKS` budget. Resolving the runtime first is what makes the map the
 * right one, because the map is per-instance (`createWebhookServer()` declares both inside the factory —
 * WI 4 re-checked that and found the receiver needed nothing further).
 *
 * `webhookServer.start` keeps its explicit-port behaviour on both paths, for the reason the section above
 * records. The difference is only in what happens when **no** port is named: the facade branch falls back
 * to the config chain as it always has, while a runtime-backed receiver refuses rather than invent one,
 * because the host allocated its port (D16). See `runtimeServerOps.ts`.
 */
import type { CommandRegistry } from "../commands/registry";
import type {
    WebhookRegisterActiveParams,
    WebhookUnregisterActiveParams,
    WebhookServerStartParams,
    WebhookServerStatusParams,
    WebhookServerStopParams,
} from "@bifurc/protocol";
import { loadConfig } from "../store/config";
import {
    getWebhookPort,
    getWebhookServerError,
    isWebhookServerRunning,
    registerActiveWebhook,
    startWebhookServer,
    stopWebhookServer,
    unregisterActiveWebhook,
} from "./webhookServer";
import { absentRuntimeError, echoWorkspace, resolveRuntimeScope } from "../runtime/runtimeRegistry";
import { startManagedServer } from "../runtime/runtimeServerOps";

/** The fallback port, matching `webhookServer.ts`'s own `currentPort` initialiser. */
const DEFAULT_WEBHOOK_PORT = 9101;

export function registerWebhookCommands(registry: CommandRegistry): void {
    registry.register("webhook.registerActive", (params: WebhookRegisterActiveParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        // The map is per-instance, so resolving the runtime *is* the fix — see the header.
        if (scope.kind === "runtime") {
            scope.runtime.webhook.registerActive(params.webhookId, params.urlSuffix);
            return { ok: true };
        }
        if (scope.kind === "facade") {
            registerActiveWebhook(params.webhookId, params.urlSuffix);
            return { ok: true };
        }
        return { ok: false };
    });

    registry.register("webhook.unregisterActive", (params: WebhookUnregisterActiveParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        if (scope.kind === "runtime") {
            scope.runtime.webhook.unregisterActive(params.webhookId);
            return { ok: true };
        }
        if (scope.kind === "facade") {
            unregisterActiveWebhook(params.webhookId);
            return { ok: true };
        }
        return { ok: false };
    });

    registry.register("webhookServer.start", (params: WebhookServerStartParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        if (scope.kind === "runtime") return startManagedServer(scope.runtime.webhook, params.port);
        if (scope.kind === "facade") {
            const cfg = loadConfig();
            startWebhookServer(params.port ?? cfg.webhookPort ?? DEFAULT_WEBHOOK_PORT);
            return { ok: true };
        }
        return { ok: false };
    });

    registry.register("webhookServer.stop", (params: WebhookServerStopParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        if (scope.kind === "runtime") {
            scope.runtime.webhook.stop();
            return { ok: true };
        }
        if (scope.kind === "facade") {
            stopWebhookServer();
            return { ok: true };
        }
        return { ok: false };
    });

    registry.register("webhookServer.status", (params: WebhookServerStatusParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        // Conditional echo, same rule and same reason as `server.status` — see `echoWorkspace`.
        const echo = echoWorkspace(params.workspaceId, scope.workspaceId);
        if (scope.kind === "runtime") {
            const webhook = scope.runtime.webhook;
            return { running: webhook.isRunning(), port: webhook.getPort(), error: webhook.getError(), ...echo };
        }
        if (scope.kind === "facade") {
            return {
                running: isWebhookServerRunning(),
                port: getWebhookPort(),
                error: getWebhookServerError(),
                ...echo,
            };
        }
        // An `absent` answer still names the workspace when the caller named it — which matters most in
        // exactly this case, where the caller has no other way to tell which of several requests came back
        // empty.
        return { running: false, port: 0, error: absentRuntimeError(scope.workspaceId), ...echo };
    });
}
