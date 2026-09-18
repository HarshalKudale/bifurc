/**
 * The five commands `packages/protocol/src/commands/misc.ts` declares that the shell was serving on
 * its own — `healthbar.getServices` / `healthbar.saveServices` / `healthbar.checkUrl`,
 * `request.replay` and `script.execute`.
 *
 * ## Why one file for three unrelated groups
 *
 * Because that is how the protocol groups them. `healthbar.*` is workspace file IO, `request.replay`
 * is an outbound HTTP call and `script.execute` is a sandboxed `vm` run — they share no subsystem, so
 * there is no engine package that honestly owns all three. Inventing one would be worse than mirroring
 * `misc.ts`, which is the grouping every client already reads off the wire. The file is named after
 * that grouping rather than after a subsystem it does not have.
 *
 * ## Why these moved out of the shell (P6 finding 5)
 *
 * All five were `ipcMain.handle` bodies in `src/ipc/handlers/coreHandlers.ts`, reachable only while
 * the shell and the engine are one process. Every function they call already lives in
 * `packages/engine`: `replayRequest` (`proxy/serverReplay.ts`), `executeIpcScript`
 * (`proxy/scriptExecutor.ts`), `invalidateCache` (`sync/statusTracker.ts`) and `emitEntityStatus`
 * (`eventBus.ts`). `scopes.ts` had already assigned all five scopes (`read` / `write` / `execute`)
 * before this module existed — the same independent evidence `serverCommands.ts` found for its six.
 *
 * ## The bodies are moved verbatim, with one deliberate exception
 *
 * `healthbar.checkUrl` is copied exactly **except** for how it reaches Node's HTTP modules. The shell
 * body used `require("https")` / `require("http")` inside the handler; `require` does not exist in the
 * engine's **ESM** build output (`tsup` emits `.mjs` alongside `.cjs`), so a verbatim copy would have
 * thrown at call time on the ESM entry point while typechecking and every unit test passed. Both
 * modules are imported statically instead. They are Node builtins with no cycle and no cost, and
 * `serverReplay.ts` and `proxy/webhookServer.ts` already import them the same way.
 *
 * ## The one gap carried over rather than fixed
 *
 * `RequestReplayResult` declares `durationMs`, and `replayRequest` does not return it — so this
 * command's result has never carried the field its own frozen contract advertises. It is moved as-is
 * for the same reason `configCommands.ts` left `workspace.add`'s odd failure shape alone: changing a
 * command's result shape while moving it hides the move behind a contract change. The renderer does
 * not read it (it times the call itself — see `restTabReducer.ts`'s own `durationMs`), so this is
 * latent, and it is recorded here rather than silently patched.
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
 * So the duplication is deliberate and bounded: five bodies, behaviourally identical to the handlers
 * below. **Step 3** removes it by deleting `registerIpcHandlers()` and the shell's handler groups
 * together — one implementation, no second entry point to keep in sync.
 */
import * as fs from "fs";
import * as path from "path";
import * as http from "http";
import * as https from "https";
import type {
    HealthbarCheckUrlParams,
    HealthbarGetServicesParams,
    HealthbarSaveServicesParams,
    RequestReplayParams,
    ScriptExecuteParams,
} from "@bifurc/protocol";
import type { CommandRegistry } from "./commands/registry";
import { wsDir } from "./store/workspaceFs";
import { invalidateCache } from "./sync/statusTracker";
import { emitEntityStatus } from "./eventBus";
import { replayRequest } from "./proxy/serverReplay";
import { executeIpcScript } from "./proxy/scriptExecutor";

/** The healthbar's own subdirectory inside a workspace, and the file it keeps its services in. */
const HEALTHBAR_DIR = "healthbar";
const HEALTHBAR_FILE = "services.json";

export function registerMiscCommands(registry: CommandRegistry): void {
    registry.register("healthbar.getServices", (params: HealthbarGetServicesParams) => {
        const file = path.join(wsDir(params.workspaceId), HEALTHBAR_DIR, HEALTHBAR_FILE);
        if (!fs.existsSync(file)) return [];
        try {
            return JSON.parse(fs.readFileSync(file, "utf-8"));
        } catch {
            return [];
        }
    });

    registry.register("healthbar.saveServices", (params: HealthbarSaveServicesParams) => {
        const dir = path.join(wsDir(params.workspaceId), HEALTHBAR_DIR);
        fs.mkdirSync(dir, { recursive: true });
        fs.writeFileSync(
            path.join(dir, HEALTHBAR_FILE),
            JSON.stringify(params.services, null, 2),
            "utf-8",
        );
        invalidateCache(params.workspaceId);
        emitEntityStatus(params.workspaceId);
        return { ok: true };
    });

    /**
     * An outbound probe, not a proxy request — it bypasses the proxy deliberately, which is why it
     * builds its own module choice from the URL scheme rather than reusing `replayRequest`.
     *
     * `rejectUnauthorized: false` is carried over from the shell body unchanged. It is what makes the
     * healthbar able to probe a host serving a self-signed certificate, which is the ordinary case for
     * the local services this feature exists to monitor.
     */
    registry.register("healthbar.checkUrl", (params: HealthbarCheckUrlParams) => {
        const start = Date.now();
        return new Promise<{
            ok: boolean;
            statusCode: number | null;
            body: string | null;
            headers: Record<string, string> | null;
            error: string | null;
            durationMs: number;
        }>((resolve) => {
            try {
                const parsedUrl = new URL(params.url);
                const mod = parsedUrl.protocol === "https:" ? https : http;
                const req = mod.get(
                    params.url,
                    { timeout: 10000, rejectUnauthorized: false },
                    (res) => {
                        const chunks: Buffer[] = [];
                        res.on("data", (chunk: Buffer) => chunks.push(chunk));
                        res.on("end", () => {
                            const rawBody = Buffer.concat(chunks).toString("utf-8");
                            resolve({
                                ok: true,
                                statusCode: res.statusCode as number,
                                body: rawBody.slice(0, 10000),
                                headers: res.headers as Record<string, string>,
                                error: null,
                                durationMs: Date.now() - start,
                            });
                        });
                        res.on("error", (err: Error) => {
                            resolve({
                                ok: false,
                                statusCode: null,
                                body: null,
                                headers: null,
                                error: err.message,
                                durationMs: Date.now() - start,
                            });
                        });
                    },
                );
                req.on("error", (err: Error) => {
                    resolve({
                        ok: false,
                        statusCode: null,
                        body: null,
                        headers: null,
                        error: err.message,
                        durationMs: Date.now() - start,
                    });
                });
                req.on("timeout", () => {
                    req.destroy();
                    resolve({
                        ok: false,
                        statusCode: null,
                        body: null,
                        headers: null,
                        error: "Request timed out",
                        durationMs: Date.now() - start,
                    });
                });
            } catch (err) {
                resolve({
                    ok: false,
                    statusCode: null,
                    body: null,
                    headers: null,
                    error: (err as Error)?.message ?? "Invalid URL",
                    durationMs: Date.now() - start,
                });
            }
        });
    });

    registry.register("request.replay", (params: RequestReplayParams) =>
        replayRequest(params.method, params.url, params.headers, params.body),
    );

    registry.register("script.execute", (params: ScriptExecuteParams) => executeIpcScript(params));
}
