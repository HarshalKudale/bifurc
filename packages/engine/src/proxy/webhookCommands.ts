/**
 * The five webhook commands — `webhook.registerActive`, `webhook.unregisterActive` and
 * `webhookServer.start` / `webhookServer.status` / `webhookServer.stop`.
 *
 * ## Why these moved out of the shell (P6 finding 5)
 *
 * They were served by `ipcMain.handle` bodies in `src/ipc/handlers/crudHandlers.ts`, which works only
 * while the shell and the engine are the same process. They are not shell concerns: every function
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
 * ## The trap: do NOT re-point the legacy channels at this module yet
 *
 * The obvious "DRY" follow-up is to replace the shell's five `ipcMain.handle` bodies with
 * `commandRegistry.invoke("<command>", params, ctx)`. **That was tried in `serverCommands.ts` and
 * reverted, and it is a real regression rather than a style question.**
 *
 * The reason is how this module gets registered. `config.get` / `env.setActive` / `workspace.setActive`
 * are registered at **module scope** in `coreHandlers.ts`, so importing that file populates them. This
 * module is registered by an **explicit call inside `registerIpcHandlers()`**, so a legacy body that
 * delegates here acquires a hidden ordering dependency on a function callers are not obliged to run —
 * and two suites (`tests/spike/protocolPoc.test.ts`,
 * `tests/integration/settingsMutations.integration.test.ts`) already register handler *groups* without
 * it. The failure is a `No handler registered` at *invocation* time, so nothing catches it at build
 * time and the shell's own handler file looks untouched.
 *
 * So the duplication is deliberate and bounded: five one-line bodies, behaviourally identical to the
 * handlers below. **Step 3** removes it by deleting `registerIpcHandlers()` and the shell's handler
 * groups together — one implementation, no second entry point to keep in sync.
 */
import type { CommandRegistry } from "../commands/registry";
import type {
    WebhookRegisterActiveParams,
    WebhookUnregisterActiveParams,
    WebhookServerStartParams,
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

/** The fallback port, matching `webhookServer.ts`'s own `currentPort` initialiser. */
const DEFAULT_WEBHOOK_PORT = 9101;

export function registerWebhookCommands(registry: CommandRegistry): void {
    registry.register("webhook.registerActive", (params: WebhookRegisterActiveParams) => {
        registerActiveWebhook(params.webhookId, params.urlSuffix);
        return { ok: true };
    });

    registry.register("webhook.unregisterActive", (params: WebhookUnregisterActiveParams) => {
        unregisterActiveWebhook(params.webhookId);
        return { ok: true };
    });

    registry.register("webhookServer.start", (params: WebhookServerStartParams) => {
        const cfg = loadConfig();
        startWebhookServer(params.port ?? cfg.webhookPort ?? DEFAULT_WEBHOOK_PORT);
        return { ok: true };
    });

    registry.register("webhookServer.stop", () => {
        stopWebhookServer();
        return { ok: true };
    });

    registry.register("webhookServer.status", () => ({
        running: isWebhookServerRunning(),
        port: getWebhookPort(),
        error: getWebhookServerError(),
    }));
}
