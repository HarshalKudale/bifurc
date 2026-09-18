/**
 * The six proxy/server-lifecycle commands the **engine** owns — `server.status`, `server.start`,
 * `server.stop`, `server.restart`, `proxy.status` and `services.discover`.
 *
 * ## Why these moved out of the shell (P6 finding 5)
 *
 * They were served by `ipcMain.handle` bodies in `src/ipc/handlers/systemHandlers.ts` and
 * `coreHandlers.ts`, which works only while the shell and the engine are the same process. They are
 * not shell concerns: the proxy server, its port and its error state all live in
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
 * ## What is deliberately absent: `app.checkUpdate`
 *
 * It is the one command in the step-3b inventory that may genuinely belong to the shell rather than
 * the engine — it reads `app.getVersion()` from Electron and asks GitHub for a release.
 * `plan/07`'s local-handlers table calls it a "shell half" while `@bifurc/client` classifies it
 * `{kind: "transport"}`. That disagreement is recorded in `tests/ipc/handlers.test.ts`'s
 * "registry coverage" block and is **not** resolved here, because resolving it is a decision about
 * where the version number comes from on a remote engine, not a mechanical move.
 *
 * ## The trap: do NOT re-point the legacy channels at this module yet
 *
 * The obvious "DRY" follow-up is to replace the shell's six `ipcMain.handle` bodies with
 * `commandRegistry.invoke("<command>", {}, ctx)`, the way `config:get` in `coreHandlers.ts` already
 * does. **That was tried and reverted, and it is a real regression rather than a style question.**
 *
 * The reason is the difference between how `coreHandlers.ts` registers and how this module is
 * registered. `config.get` / `env.setActive` / `workspace.setActive` are registered at **module
 * scope**, so importing `coreHandlers.ts` is enough to populate them. This module is registered by
 * an **explicit call inside `registerIpcHandlers()`**, so a legacy body that delegates here acquires
 * a hidden ordering dependency on a function in a different file — and, worse, on a function that
 * callers are not obliged to run.
 *
 * They are not obliged to run it, and two suites already didn't. `tests/spike/protocolPoc.test.ts`
 * and `tests/integration/settingsMutations.integration.test.ts` deliberately register handler
 * *groups* (`registerCoreHandlers()` / `registerSystemHandlers()`) without calling
 * `registerIpcHandlers()`. Re-pointing broke six of their tests with `UNKNOWN_COMMAND` —
 * `server.status` among them — because the registry they reached was empty. The failure is a
 * `No handler registered` at *invocation* time, so nothing catches it at build time and the shell's
 * own handler file looks untouched.
 *
 * So the duplication is deliberate for now, and bounded: six one-line bodies, behaviourally identical
 * to the handlers below. It is removed by **step 3**, which deletes `registerIpcHandlers()` and the
 * shell's handler groups together — at which point there is one implementation and no second entry
 * point to keep in sync. Re-pointing early trades a temporary, visible duplication for a permanent,
 * invisible coupling.
 */
import type { CommandRegistry } from "../commands/registry";
import { loadConfig } from "../store/config";
import { discoverServices } from "./service-discovery";
import { getPort, getServerError, isRunning, startServer, stopServer } from "./server";

export function registerServerCommands(registry: CommandRegistry): void {
    registry.register("server.status", () => ({
        running: isRunning(),
        port: getPort(),
        error: getServerError(),
    }));

    registry.register("server.start", () => {
        const cfg = loadConfig();
        startServer(cfg.port);
        return { ok: true };
    });

    registry.register("server.stop", () => {
        stopServer();
        return { ok: true };
    });

    registry.register("server.restart", () => {
        const cfg = loadConfig();
        stopServer();
        startServer(cfg.port);
        return { ok: true };
    });

    registry.register("proxy.status", () => ({ running: isRunning() }));

    registry.register("services.discover", () => discoverServices());
}
