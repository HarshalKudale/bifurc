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
 * ## P14 WI 7 resolved a runtime from `workspaceId`; `plan/17` deletes the question
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
 * P14's fix was `resolveRuntimeScope()` (`runtime/runtimeRegistry.ts`), a three-way union whose load-bearing
 * case was **`absent`**: a workspace explicitly named but not served by this host was reported, not silently
 * redirected to the facade.
 *
 * **`plan/17` §1 makes that union unanswerable and therefore unnecessary.** There is now exactly **one**
 * forward proxy per host, on a port the user owns, serving whichever workspace is active — so a
 * `workspaceId` no longer selects a server, and "which of the N?" is not a question the code can ask. The
 * registry, the union and the `absent` case are all deleted with it, and these bodies become direct facade
 * calls again.
 *
 * **The bug P14 fixed cannot come back, and it is worth saying why rather than trusting the deletion.** Its
 * shape was *"the command acted on the active workspace while the caller named another"* — which needed
 * **per-workspace servers** to be wrong about. With one server there is no other workspace's server to act
 * on instead: `server.start` starts the host's proxy, and that is the only thing it could mean. The
 * `workspaceId` parameter survives as an **echo** (see `commands/echoWorkspace.ts`) because a multiplexed
 * client still has to match response to request.
 *
 * **What did *not* survive the deletion is the guard on the active workspace.** The edge's `resolveConfig`
 * still asks "is the active workspace one of ours?" — that question survives, because `--workspace` can
 * still filter the host down to a subset. See `runtime/host.ts` step 4.
 *
 * `services.discover` is deliberately **not** scoped: it reports the ports this host is listening on,
 * which is a property of the host and not of any workspace. Asserted in `index.test.ts` so it reads as a
 * decision rather than an oversight.
 */
import type { CommandRegistry } from "../kernel/registry";
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
import { echoWorkspace } from "../utils/echoWorkspace";

/**
 * The port a command re-binds when the proxy is not running.
 *
 * **`getPort()` first, and that is deliberate.** `proxy/server.ts` keeps `currentPort` across `stop()`, so a
 * proxy that has been started once reports the port it really bound; that is a stronger answer than a
 * setting, and `tests/proxy/server.test.ts` pins the retention. The config is the fallback for the one caller
 * that never started a proxy at all — a process that asks this command first, where there is no resolved port
 * to remember.
 *
 * **It reads `forwardProxyPort`, not `port`.** `settings.port` is the **edge's** port (`plan/15` §3.1): the
 * host binds the edge on it at boot, so a proxy bound there would fail with `EADDRINUSE` against the engine's
 * own listener — a failure that would read as a busy machine rather than as a stale read. `plan/17` §1 gives
 * the forward proxy its own setting, which is what this now names.
 */
function proxyPortInForce(): number {
    return getPort() || loadConfig().forwardProxyPort;
}

export function registerServerCommands(registry: CommandRegistry): void {
    registry.register("server.status", (params: ServerStatusParams) => ({
        running: isRunning(),
        port: getPort(),
        error: getServerError(),
        ...echoWorkspace(params.workspaceId),
    }));

    registry.register("server.start", (_params: ServerStartParams) => {
        /**
         * **No `workspaceId` is read, and that is the change `plan/17` makes rather than a regression.**
         * There is one proxy; the parameter used to pick between N and now has nothing to pick.
         *
         * `startServer()` re-binds — `proxy/server.ts`'s `start()` stops first — so calling this on a running
         * proxy restarts it on the same port. That is the pre-P14 body's behaviour, kept deliberately: the
         * alternative (report `{ ok: true }` and do nothing) would make the command's name untrue for the one
         * caller that uses it to recover a proxy that failed to bind.
         */
        startServer(proxyPortInForce());
        return { ok: true };
    });

    registry.register("server.stop", (_params: ServerStopParams) => {
        stopServer();
        return { ok: true };
    });

    registry.register("server.restart", (_params: ServerRestartParams) => {
        // Stop then start, exactly as before — `restart` is not a distinct code path, which is the property
        // the module header records for the original body. The port is read **before** the stop only because
        // `stopServer()` retains it; reading it after would work for the same reason, but reading it first is
        // what makes this identical in shape to the body it replaces.
        const port = proxyPortInForce();
        stopServer();
        startServer(port);
        return { ok: true };
    });

    registry.register("proxy.status", (_params: ProxyStatusParams) => ({ running: isRunning() }));

    registry.register("services.discover", () => discoverServices());
}
