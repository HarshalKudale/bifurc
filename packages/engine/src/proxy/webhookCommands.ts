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
 * ## P14 WI 7 resolved a runtime from `workspaceId`; `plan/17` deletes the question
 *
 * The same gap `serverCommands.ts` records, in a second module: these called the module facade
 * (`webhookServer.ts`'s `startWebhookServer` / `isWebhookServerRunning` / `getWebhookPort`), whose lazy
 * default instance is `createWebhookServer({ workspaceId: null })` — the active workspace. WI 1's protocol
 * half gave all five commands an optional `workspaceId`; P14 made them read it, because **the two
 * `registerActive` / `unregisterActive` commands were the ones where the old behaviour was worst**: they
 * mutate a receiver's **active map**, so on a host a registration from workspace A wrote into the *active*
 * workspace's map, and a webhook registered by A could be served by B or counted against B's
 * `MAX_ACTIVE_WEBHOOKS` budget. Resolving the runtime first is what made the map the right one.
 *
 * **`plan/17` §1 removes the premise.** There is one webhook receiver per host, on a port the user owns, and
 * the active map is per *instance* — so with one instance there is again one map, and no resolution step can
 * pick the wrong one. The scope union and `runtimeServerOps.ts` are deleted with it; these bodies are direct
 * facade calls, and `workspaceId` survives only as an **echo** (`commands/echoWorkspace.ts`).
 *
 * **One consequence is worth stating rather than implying**, because deleting the routing did not remove it:
 * a single host-level active map means **switching workspace does not retire the previous workspace's active
 * webhooks**. `plan/17` §1 lists *"which webhook endpoints are in force"* among the things a workspace switch
 * changes, so this is a **known gap** rather than the intended end state — closing it needs the active map
 * keyed by workspace, which `createWebhookServer()` does not do. Recorded here and in `runtime/host.ts`
 * step 3c, so it is found rather than discovered.
 */
import type { CommandRegistry } from "../kernel/registry";
import type {
    WebhookRegisterActiveParams,
    WebhookUnregisterActiveParams,
    WebhookServerStartParams,
    WebhookServerStatusParams,
    WebhookServerStopParams,
} from "@bifurc/protocol";
import { loadConfig } from "../workspace/config";
import {
    getWebhookPort,
    getWebhookServerError,
    isWebhookServerRunning,
    registerActiveWebhook,
    startWebhookServer,
    stopWebhookServer,
    unregisterActiveWebhook,
} from "./webhookServer";
import { echoWorkspace } from "../utils/echoWorkspace";

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
        /**
         * The port is the **user's**, not the engine's — `plan/17` §1: *"the webhook does NOT scan either. It
         * is a **feature, not infrastructure**: it is **off by default**, the user starts it, and the user
         * owns the port."* So a caller-supplied port is honoured exactly, and the fallback is the setting
         * rather than an invented number.
         *
         * **There is no `?? 9101` any more, and its removal is the point.** `cfg.webhookPort` used to be a
         * fixed default that a missing key could fall through to; since `plan/17` §1 the default is a
         * **random** port (`store/appSettings.ts#randomWebhookPort`), so the chain has one link and no
         * constant to disagree with it. A fixed fallback would also have been a second answer to "which port
         * is this receiver on", which is exactly what the setting exists to be the only home of.
         */
        startWebhookServer(params.port ?? loadConfig().webhookPort);
        return { ok: true };
    });

    registry.register("webhookServer.stop", (_params: WebhookServerStopParams) => {
        stopWebhookServer();
        return { ok: true };
    });

    registry.register("webhookServer.status", (params: WebhookServerStatusParams) => ({
        running: isWebhookServerRunning(),
        port: getWebhookPort(),
        error: getWebhookServerError(),
        // Conditional echo, same rule and same reason as `server.status` — see `echoWorkspace`.
        ...echoWorkspace(params.workspaceId),
    }));
}
