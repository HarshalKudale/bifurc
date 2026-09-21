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
