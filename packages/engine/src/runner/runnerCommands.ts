/**
 * The `runner.*` commands whose implementation is the engine's: `saveReport`, `saveConfig` and
 * `loadConfig`. `runner.exportReport` had already moved (P2/P3), so this file completes the runner's
 * request paths.
 *
 * (`runner.getHistory` and `runner.listFolderIds` are registered in the *shell* —
 * `src/ipc/handlers/runnerHandlers.ts` — at module scope. They are not moved here for the reason
 * `plan/07` §5c records: a module-scope registration takes effect on import, so relocating one behind
 * an explicit `registerRunnerCommands()` call would silently unregister it for the suites that import
 * the handler group without running `registerIpcHandlers()`.)
 *
 * ## The two config commands were `BLOCKED`; the widening is what unblocked them
 *
 * The step-3b ratchet filed `runner.saveConfig` / `runner.loadConfig` as `BLOCKED` rather than
 * unimplemented, with a **passing test** as the evidence: `RunnerConfigSchema` required
 * `requestOrder`, while `tests/integration/runnerStorage.integration.test.ts:207` round-trips
 * `{delayMs, stopOnFailure, iterations}`. The ratchet asserted that block *directly* — parsing the real
 * config through the frozen schema had to fail — so that widening the schema would fail the test and
 * say these had become movable, rather than leaving them filed as "unimplemented" forever.
 *
 * That is exactly what happened on 2026-09-19: the test failed, the schema was widened to the shape
 * the store actually persists, and these two moved. The reasoning lives in
 * `packages/protocol/src/commands/runner.ts`; the short version is that the frozen schema was wrong
 * twice over — it **rejected** the payload a passing test saves, and would have **stripped** fields
 * from any payload that did pass, because `z.object()` drops unknown keys.
 *
 * ## The move is safe, and the reason is worth recording
 *
 * `RunReport` is a plain `z.object()`, and zod's `z.object()` **strips unknown keys**. So the
 * interesting question is not "does the schema accept the payload" — a permissive-enough schema
 * accepting a report is not the same as it *preserving* one — but **"does parsing lose any field the
 * renderer sends?"**. It does not, and that was checked rather than assumed:
 *
 * | Type | Fields | In `RunReport`? |
 * | --- | --- | --- |
 * | `CollectionRunReport` (`renderer/lib/collectionRunner.ts`) | 9 | all 9 |
 * | `RunnerRequestResult` | 11 | all 11 |
 *
 * That table is the whole reason this is `MOVABLE` rather than `NARROWED`. The P1 schema was written
 * from the HTML renderer's field *usage*, which is why it dropped `requestId` / `url` / `testLogs` /
 * `preScriptError` / `postScriptError`; P3 widened `RunResult` to add them back precisely because
 * `z.object()` strips silently and the HTML renderer would not have noticed. A report truncated here
 * would be a data-loss bug in saved run history, not a type error.
 *
 * ## Why the HTML is rendered here rather than in the shell
 *
 * `renderRunnerHtml` lives in `packages/engine/src/runner/reportHtml.ts` because it is a generated
 * artifact and `runner.exportReport` — already routed through the registry — needs the same renderer.
 * Writing `report.html` next to `report.json` is what the shell body did, and it stays.
 *
 * ## The trap: do NOT re-point the legacy channel at this module yet
 *
 * The obvious "DRY" follow-up is to replace the shell's `runner:saveReport` body with
 * `commandRegistry.invoke("runner.saveReport", params, ctx)`. **That was tried in `serverCommands.ts`
 * and reverted, and it is a real regression rather than a style question.**
 *
 * The reason is how this module gets registered. `config.get` / `env.setActive` / `workspace.setActive`
 * are registered at **module scope** in `coreHandlers.ts`, so importing that file populates them. This
 * module is registered by an **explicit call inside `registerIpcHandlers()`**, so a legacy body that
 * delegates here acquires a hidden ordering dependency on a function callers are not obliged to run —
 * and two suites already register handler *groups* without it. The failure is a `No handler registered`
 * at *invocation* time, so nothing catches it at build time and the shell's own handler file looks
 * untouched.
 *
 * So the duplication is deliberate and bounded. **Step 3** removes it by deleting
 * `registerIpcHandlers()` and the shell's handler groups together — one implementation, no second
 * entry point to keep in sync.
 */
import * as fs from "fs";
import * as path from "path";
import type {
    RunnerLoadConfigParams,
    RunnerSaveConfigParams,
    RunnerSaveReportParams,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";
import { wsDir } from "../store/workspaceFs";
import { renderRunnerHtml } from "./reportHtml";

export function registerRunnerCommands(registry: CommandRegistry): void {
    registry.register("runner.saveReport", (params: RunnerSaveReportParams) => {
        try {
            const folderId = params.report.folderId;
            const ts = params.report.startedAt;
            const runDir = path.join(
                wsDir(params.workspaceId),
                "requests",
                ".runs",
                folderId,
                String(ts),
            );
            fs.mkdirSync(runDir, { recursive: true });
            fs.writeFileSync(
                path.join(runDir, "report.json"),
                JSON.stringify(params.report, null, 2),
                "utf-8",
            );

            // The HTML is generated by the engine, because `runner.exportReport` needs the same
            // renderer and a second copy would drift.
            const html = renderRunnerHtml(params.report);
            fs.writeFileSync(path.join(runDir, "report.html"), html, "utf-8");
            return { ok: true };
        } catch (err) {
            // Resolved, not thrown: the renderer branches on `{ok:false}`, and a throw would surface
            // as an `EngineError` with the message flattened — losing the `ok` flag entirely.
            return { ok: false, error: (err as Error)?.message ?? "Failed to save report" };
        }
    });

    /**
     * A folder's runner configuration, written verbatim.
     *
     * `JSON.stringify(config, null, 2)` and nothing else — no validation, no defaults, no field
     * picking. That is deliberate and it is the whole point of the `z.record` widening: the config is
     * the *caller's* shape, so anything that reads it back out and re-writes a subset of it would be
     * the data-loss bug the old schema would have caused.
     *
     * `{ ok: false }` with no `error` on failure is preserved from the shell body rather than improved.
     * Adding a message would be a contract change smuggled in under a move, and the renderer's
     * declared result is `{ ok: boolean }`.
     */
    registry.register("runner.saveConfig", (params: RunnerSaveConfigParams) => {
        try {
            const runsDir = path.join(wsDir(params.workspaceId), "requests", ".runs", params.folderId);
            fs.mkdirSync(runsDir, { recursive: true });
            fs.writeFileSync(
                path.join(runsDir, "runner.json"),
                JSON.stringify(params.config, null, 2),
                "utf-8",
            );
            return { ok: true };
        } catch {
            return { ok: false };
        }
    });

    /**
     * Read it back, or `null` when the folder has no saved configuration.
     *
     * `null` and `{}` mean different things to the caller — `runner.listFolderIds` uses the file's
     * *presence* to decide whether a folder has a configuration at all — so a missing or unparseable
     * file must stay `null` rather than becoming an empty object. A corrupt `runner.json` reads as
     * "not configured", which is the pre-existing behaviour.
     */
    registry.register("runner.loadConfig", (params: RunnerLoadConfigParams) => {
        try {
            const file = path.join(
                wsDir(params.workspaceId),
                "requests",
                ".runs",
                params.folderId,
                "runner.json",
            );
            if (!fs.existsSync(file)) return null;
            return JSON.parse(fs.readFileSync(file, "utf-8")) as Record<string, unknown>;
        } catch {
            return null;
        }
    });
}
