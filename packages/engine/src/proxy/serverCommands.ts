/**
 * The six proxy/server-lifecycle commands the **engine** owns — `server.status`, `server.start`,
 * `server.stop`, `server.restart`, `proxy.status` and `services.discover`.
 *
 * ## Why these moved out of the shell (P6 finding 5)
 *
 * They were served by `ipcMain.handle` bodies in `src/ipc/handlers/systemHandlers.ts` and
 * `coreHandlers.ts` — the three `systemHandlers` ones are **deleted as of 2026-09-21** (P6
 * reduction, fifth group), which worked only while the shell and the engine were the same process.
 * They are not shell concerns: the proxy server, its port and its error state all live in
 * `packages/engine/src/proxy/`, and `discoverServices()` is an engine-side network scan. P9's
 * container is the case that makes it concrete — a containerised engine that cannot answer
 * `server.status` cannot be supervised.
 *
 * ## The bodies are moved verbatim, and that is the requirement
 *
 * Each handler below reproduces the shell body it replaces, character for character in behaviour:
 * `start` reads the port from `loadConfig()` rather than taking it as a parameter, `restart` is
 * `stop` then `start` (not a distinct code path), and `status` and `proxy.status` are two views of
 * the same `isRunning()` with **different payloads** — `server.status` carries `port` and `error`,
 * `proxy.status` carries only `running`.
 *
 * That last pair is worth stating because it looks like a duplicate and is not. Both channels are on
 * `window.api` (`serverStatus` and `proxyStatus`), so collapsing them into one command would change
 * the surface of one of them. The protocol lists them separately for the same reason.
 *
 * ## What is deliberately absent: `app.checkUpdate` — resolved, and not by moving it
 *
 * It was the one command in the step-3b inventory that might genuinely belong to the shell rather
 * than the engine: it reads `app.getVersion()` from Electron and asks GitHub for a release.
 * `plan/07`'s local-handlers table called it a "shell half" while `@bifurc/client` classified it
 * `{kind: "transport"}`, and this header used to record that disagreement without resolving it.
 *
 * **It was resolved on 2026-09-20, and not by moving it.** The command left `@bifurc/protocol`
 * entirely and the surface key became `local`, because `currentVersion` is the version of the thing
 * the *user* runs and the download URL must be the asset for the *user's* platform — so there was no
 * engine half to contract with. `packages/client/src/local.ts` carries the argument. The shell
 * channel `app:checkUpdate` therefore **stays**, and is now the one channel `systemHandlers.ts` still
 * registers.
 *
 * ## The trap that no longer applies — and why it was right while it did
 *
 * The obvious "DRY" follow-up was to replace the shell's `ipcMain.handle` bodies with
 * `commandRegistry.invoke("<command>", {}, ctx)`, the way `config:get` in `coreHandlers.ts` already
 * does. **That was tried and reverted, and it was a real regression rather than a style question.**
 *
 * The reason was the difference between how `coreHandlers.ts` registers and how this module is
 * registered. `config.get` / `env.setActive` / `workspace.setActive` are registered at **module
 * scope**, so importing `coreHandlers.ts` is enough to populate them. This module is registered by an
 * **explicit call inside `registerIpcHandlers()`**, so a legacy body that delegated here acquired a
 * hidden ordering dependency on a function in a different file — and, worse, on a function that
 * callers were not obliged to run.
 *
 * They were not obliged to run it, and two suites didn't. `tests/integration/settingsMutations.integration.test.ts`
 * and `tests/spike/protocolPoc.test.ts` deliberately registered handler *groups*
 * (`registerCoreHandlers()` / `registerSystemHandlers()`) without calling `registerIpcHandlers()`.
 * The spike half of that pair was **retired 2026-09-21** (`spike/README.md`), which does not change the
 * lesson: the coupling was real while it existed, and the lesson is about the shape of the dependency
 * rather than about the suites that happened to expose it.
 * Re-pointing broke six of their tests with `UNKNOWN_COMMAND` — `server.status` among them — because
 * the registry they reached was empty. The failure was a `No handler registered` at *invocation*
 * time, so nothing caught it at build time and the shell's own handler file looked untouched.
 *
 * **The trap is dissolved, not answered, and the distinction is worth keeping.** The reduction's
 * fifth group deleted the three `server:*` channels on 2026-09-21, so the duplication the trap warned
 * about is gone and there is now one implementation with no second entry point to keep in sync — which
 * is what step 3 was going to achieve by deleting both halves together. **The suites did not have to
 * change their registration shape for this**: they kept registering the same handler groups, and the
 * three `server:*` calls in `settingsMutations` were re-pointed at the commands in that suite's own
 * `invoke`. The lesson is that "duplication kept deliberately to avoid a hidden coupling" is a
 * *temporary* answer that a deletion can retire, and that retiring it needs no new coupling at all.
 *
 * ## P14 WI 7 — these now resolve a runtime from `workspaceId` instead of the facade
 *
 * WI 1's protocol half made the six commands *accept* an optional `workspaceId`. It did not make them
 * *read* it, and this module was the reason: it called the module facade
 * (`server.ts`'s `isRunning` / `getPort` / `startServer`), whose lazy default instance is
 * `createProxyServer({ workspaceId: null })` — the **active** workspace. On a host serving two workspaces
 * that produced a `server.status` of `{ running: false, port: 0 }` while both members were listening, and
 * a `server.start { workspaceId: "A" }` that returned **`{ ok: true }`** after binding a *third* server on
 * the active workspace's configured port. **The failure succeeded**, which is why every desktop suite
 * stayed green through it.
 *
 * `resolveRuntimeScope()` (`runtime/runtimeRegistry.ts`) is the fix, and its third case is the load-bearing
 * one: a workspace that is explicitly named but has no registered runtime is reported as **`absent`**, not
 * silently redirected to the facade. Redirecting would have re-created the same bug — acting on one
 * workspace while the caller asked for another — with a success code.
 *
 * **The desktop is untouched, and that is the acceptance criterion.** With no runtimes registered, a
 * request resolving to the active workspace still takes the facade branch, which is exactly what
 * `createEngine()` binds. P6's rule for this work item is *zero user-visible change*.
 *
 * `services.discover` is deliberately **not** scoped: it reports the ports this host is listening on,
 * which is a property of the host and not of any workspace. Asserted in `index.test.ts` so it reads as a
 * decision rather than an oversight.
 */
import type { CommandRegistry } from "../commands/registry";
import type {
    ProxyStatusParams,
    ServerRestartParams,
    ServerStartParams,
    ServerStatusParams,
    ServerStopParams,
} from "@bifurc/protocol";
import { loadConfig } from "../store/config";
import { discoverServices } from "./service-discovery";
import { getPort, getServerError, isRunning, startServer, stopServer } from "./server";
import { absentRuntimeError, echoWorkspace, resolveRuntimeScope } from "../runtime/runtimeRegistry";
import { restartManagedServer, startManagedServer } from "../runtime/runtimeServerOps";

export function registerServerCommands(registry: CommandRegistry): void {
    registry.register("server.status", (params: ServerStatusParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        const echo = echoWorkspace(params.workspaceId, scope.workspaceId);
        if (scope.kind === "runtime") {
            const proxy = scope.runtime.proxy;
            return { running: proxy.isRunning(), port: proxy.getPort(), error: proxy.getError(), ...echo };
        }
        if (scope.kind === "facade") {
            return { running: isRunning(), port: getPort(), error: getServerError(), ...echo };
        }
        return { running: false, port: 0, error: absentRuntimeError(scope.workspaceId), ...echo };
    });

    registry.register("server.start", (params: ServerStartParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        // The host allocated this workspace's port and started it (D16), so a command reports rather than
        // binds — see `runtimeServerOps.ts`'s header for why that is not merely tidiness.
        if (scope.kind === "runtime") return startManagedServer(scope.runtime.proxy);
        if (scope.kind === "facade") {
            startServer(loadConfig().port);
            return { ok: true };
        }
        return { ok: false };
    });

    registry.register("server.stop", (params: ServerStopParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        if (scope.kind === "runtime") {
            scope.runtime.proxy.stop();
            return { ok: true };
        }
        if (scope.kind === "facade") {
            stopServer();
            return { ok: true };
        }
        return { ok: false };
    });

    registry.register("server.restart", (params: ServerRestartParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        if (scope.kind === "runtime") return restartManagedServer(scope.runtime.proxy);
        if (scope.kind === "facade") {
            // Stop then start, exactly as the runtime branch does — `restart` is not a distinct code path,
            // which is the property the module header records for the pre-P14 body.
            const cfg = loadConfig();
            stopServer();
            startServer(cfg.port);
            return { ok: true };
        }
        return { ok: false };
    });

    registry.register("proxy.status", (params: ProxyStatusParams) => {
        const scope = resolveRuntimeScope(params.workspaceId);
        if (scope.kind === "runtime") return { running: scope.runtime.proxy.isRunning() };
        if (scope.kind === "facade") return { running: isRunning() };
        // `ProxyStatusResult` is `{ running }` and carries no error field — unlike `server.status`, which
        // is why the two are separate commands. `false` is the truthful answer for a workspace this host
        // does not serve: nothing is running *for it*.
        return { running: false };
    });

    registry.register("services.discover", () => discoverServices());
}
