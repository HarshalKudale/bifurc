/**
 * The three `healthbar.*` commands — `healthbar.getServices`, `healthbar.saveServices` and
 * `healthbar.checkUrl`.
 *
 * ## Where this file came from
 *
 * It is one of the three destinations of `packages/engine/src/miscCommands.ts`, which
 * `Engine_Module_Plan.md` §4.5 dissolved in the engine-layout's phase 7. That file held five commands
 * that shared nothing but their protocol grouping (`packages/protocol/src/commands/misc.ts`), and its
 * own docblock argued the grouping was honest precisely because no subsystem owned all five. Once the
 * feature modules existed, each group had an obvious home; this is the `workspace/` third.
 *
 * The other two: `request.replay` → `rest/commands.ts`, `script.execute` → `mocks/commands.ts`.
 *
 * ## Why `workspace/` — and why that forced a cache move
 *
 * `getServices` / `saveServices` read and write `healthbar/services.json` under the workspace
 * directory, so `workspace/` is the module that owns them. That was §4.5's assignment, and it is the
 * reason phase 7 also moved the per-workspace status cache down into `workspace/statusCache.ts`:
 * `saveServices` calls `invalidateCache()`, which used to live in `gitops/statusTracker.ts`, and
 * `workspace/` importing `gitops/` would violate §5.1/§6. See that file for the full rationale.
 *
 * `checkUrl` is grouped here because the protocol groups it here, not because it is workspace file IO —
 * it is an outbound HTTP probe with no filesystem involvement at all. The grouping is the client-facing
 * contract; splitting it would have meant inventing a module for one command.
 *
 * ## Why these moved out of the shell (P6 finding 5)
 *
 * All three were `ipcMain.handle` bodies in `src/ipc/handlers/coreHandlers.ts`, reachable only while
 * the shell and the engine are one process. Every function they call already lives in
 * `packages/engine`: `invalidateCache` (`workspace/statusCache.ts`) and `emitEntityStatus`
 * (`runtime/entityStatus.ts` — it lived in `kernel/eventBus.ts` until engine-layout phase 8's follow-up). `transport/auth/scopes.ts` had already assigned all three scopes
 * (`read` / `write` / `execute`) before the module existed — the same independent evidence
 * `serverCommands.ts` found for its six.
 *
 * ## The bodies are moved verbatim, with one deliberate exception
 *
 * `healthbar.checkUrl` is copied exactly **except** for how it reaches Node's HTTP modules. The shell
 * body used `require("https")` / `require("http")` inside the handler; `require` does not exist in the
 * engine's **ESM** build output (`tsup` emits `.mjs` alongside `.cjs`), so a verbatim copy would have
 * thrown at call time on the ESM entry point while typechecking and every unit test passed. Both
 * modules are imported statically instead. They are Node builtins with no cycle and no cost, and
 * `utils/replay.ts` and `webhooks/webhookServer.ts` already import them the same way.
 *
 * ## The trap that no longer applies — and why it was right while it did
 *
 * The obvious "DRY" follow-up was to replace the shell's `ipcMain.handle` bodies with
 * `commandRegistry.invoke("<command>", params, ctx)`. **That was tried in `serverCommands.ts` and
 * reverted, and it is a real regression rather than a style question.** The reason was how the module
 * got registered: the `config.*` commands were registered at **module scope**, so importing that file
 * populated them, while this module is registered by an **explicit call** — so a legacy body that
 * delegated here acquired a hidden ordering dependency on a function callers are not obliged to run.
 * The failure was a `No handler registered` at *invocation* time, invisible to typecheck, to the build,
 * and to any test that runs `registerIpcHandlers()`.
 *
 * That duplication was dissolved rather than answered: the P6 reduction's sixth group deleted all
 * seventeen of `coreHandlers.ts`'s channels on 2026-09-21 (`plan/07` §5b-septendecies) and the file
 * became `coreKinds.ts`, taking these three channels with it. The generalisable half is worth carrying:
 * **the trap was about the *ordering* of an explicit call, and it survived every rewrite of the
 * bodies** — only re-pointing them and watching two suites fail with `UNKNOWN_COMMAND` revealed it.
 */
import * as fs from "fs";
import * as path from "path";
import * as http from "http";
import * as https from "https";
import type {
    HealthbarCheckUrlParams,
    HealthbarGetServicesParams,
    HealthbarSaveServicesParams,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { wsDir } from "../workspace/workspaceFs";
import { invalidateCache } from "../workspace/statusCache";
import { emitEntityStatus } from "../runtime/entityStatus";

/** The healthbar's own subdirectory inside a workspace, and the file it keeps its services in. */
const HEALTHBAR_DIR = "healthbar";
const HEALTHBAR_FILE = "services.json";

export function registerHealthbarCommands(registry: CommandRegistry): void {
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
}
