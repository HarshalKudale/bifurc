/**
 * Every `runner.*` command whose implementation is the engine's: `saveReport`, `saveConfig`,
 * `loadConfig`, `getHistory` and `listFolderIds`. `runner.exportReport` had already moved (P2/P3), so
 * this file now owns the whole `runner.*` namespace.
 *
 * ## `getHistory` / `listFolderIds` were the §5c pair (moved 2026-09-19, step 3e slice 2)
 *
 * These two were registered at **module scope** in `src/ipc/handlers/runnerHandlers.ts` — the §5c
 * defect — so a containerised engine answered `UNKNOWN_COMMAND` for two commands the protocol declares,
 * `scopes.ts` scopes as `read`, and `@bifurc/client` advertises as `getRunHistory` /
 * `listRunnerFolderIds`. `runnerHandlers.ts`'s own comment called them "the reason they cannot move into
 * `packages/engine`", and named the cost: that file's channel adapters for these two are thin
 * `commandRegistry.invoke()` delegations, so a suite importing the handler group without running
 * `registerIpcHandlers()` would reach an empty registry.
 *
 * That cost turned out to be **zero**, and it is worth recording why, because §5c's other slices may not
 * be as lucky: `tests/integration/runnerStorage.integration.test.ts` is the only file that imports
 * `runnerHandlers`, and it already called `registerRunnerCommands(commandRegistry)` — added in step 3d
 * for the `saveConfig` / `loadConfig` block. So no suite needed a new call site, and this slice is a
 * pure move.
 *
 * Bodies are verbatim. The only edits are `workspaceDir` → `wsDir` (the same function, already imported
 * here) and this file's `(params: X)` signature style.
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
 * | `CollectionRunReport` (`packages/gui/src/lib/collectionRunner.ts`) | 9 | all 9 |
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
 * ## The trap that still applies to `runner.saveReport`
 *
 * The obvious "DRY" follow-up is to replace the shell's `runner:saveReport` body with
 * `commandRegistry.invoke("runner.saveReport", params, ctx)`. **That was tried in `serverCommands.ts`
 * and reverted, and it is a real regression rather than a style question.**
 *
 * The reason is how this module gets registered. `config.get` / `env.setActive` / `workspace.setActive`
 * *were* registered at **module scope** in `coreHandlers.ts`, so importing that file populated them —
 * **that file is now `coreKinds.ts` and registers no channels at all** (P6 reduction, sixth group,
 * 2026-09-21; `plan/07` §5b-septendecies), so the contrast this paragraph drew is historical. This
 * module is registered by an **explicit call inside `registerIpcHandlers()`**, so a legacy body that
 * delegates here acquires a hidden ordering dependency on a function callers are not obliged to run —
 * and two suites already register handler *groups* without it. The failure is a `No handler registered`
 * at *invocation* time, so nothing catches it at build time and the shell's own handler file looks
 * untouched.
 *
 * So the duplication is deliberate and bounded. **Step 3** removes it by deleting
 * `registerIpcHandlers()` and the shell's handler groups together — one implementation, no second
 * entry point to keep in sync. Note the trap is still **live** for this file specifically: unlike the
 * four groups already deleted, `runnerHandlers.ts` has not been reduced yet, so `runner:saveReport`
 * still has a shell body and the warning still binds.
 */
import * as fs from "fs";
import * as path from "path";
import type {
    RunnerExportReportParams,
    RunnerExportReportResult,
    RunnerGetHistoryParams,
    RunnerListFolderIdsParams,
    RunnerLoadConfigParams,
    RunnerSaveConfigParams,
    RunnerSaveReportParams,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../kernel/registry";
import { wsDir } from "../workspace/workspaceFs";
import { renderRunnerHtml } from "./reportHtml";
import { publishArtifact } from "../workspace/blob/publish";

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
    // `runner.exportReport` came here from `fileops/commands.ts` in the layout cleanup: it renders the
    // report through `renderRunnerHtml` — the same function `runner.saveReport` above uses — and
    // `fileops/` was an aggregator holding three domains' commands. It is the runner's.
    registry.register("runner.exportReport", (p: RunnerExportReportParams) => runnerExportReport(p));

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

    /**
     * A folder's run history, newest first.
     *
     * Read straight off the `.runs/<folderId>/<timestamp>/report.json` layout that
     * `runner.saveReport` writes, so the two must agree on that path. Every failure mode resolves
     * rather than throwing — a missing directory, a directory that is not a run, a `report.json` that
     * does not parse — because the renderer's contract here is a list, and `[]` is what it renders for
     * "no runs yet". The nested `try` is load-bearing: a single corrupt `report.json` must drop that
     * one entry, not fail the whole folder.
     *
     * `Number(ts)` is not redundant with the directory name: the name is a string, and the sort below
     * is numeric.
     */
    registry.register("runner.getHistory", (params: RunnerGetHistoryParams) => {
        try {
            const runsDir = path.join(wsDir(params.workspaceId), "requests", ".runs", params.folderId);
            if (!fs.existsSync(runsDir)) return [];
            const entries = fs.readdirSync(runsDir).filter((d) => {
                return fs.statSync(path.join(runsDir, d)).isDirectory();
            });
            return entries.map((ts) => {
                const reportFile = path.join(runsDir, ts, "report.json");
                if (!fs.existsSync(reportFile)) return null;
                try {
                    const data = JSON.parse(fs.readFileSync(reportFile, "utf-8"));
                    return {
                        timestamp: Number(ts),
                        summary: {
                            total: data.totalTests ?? 0,
                            passed: data.passedTests ?? 0,
                            failed: data.failedTests ?? 0,
                        },
                    };
                } catch { return null; }
            }).filter(Boolean).sort((a: any, b: any) => b.timestamp - a.timestamp);
        } catch {
            return [];
        }
    });

    /**
     * The folders that have a runner configuration, i.e. those with a `.runs/<folderId>/runner.json`.
     *
     * This is why `runner.loadConfig` must return `null` rather than `{}` for a missing file: the
     * *presence* of `runner.json` is what makes a folder a runner folder, so an empty-object answer
     * would make every folder look configured.
     */
    registry.register("runner.listFolderIds", (params: RunnerListFolderIdsParams) => {
        try {
            const runsDir = path.join(wsDir(params.workspaceId), "requests", ".runs");
            if (!fs.existsSync(runsDir)) return [];
            return fs.readdirSync(runsDir, { withFileTypes: true })
                .filter((e) => e.isDirectory() && fs.existsSync(path.join(runsDir, e.name, "runner.json")))
                .map((e) => e.name);
        } catch {
            return [];
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// runner.exportReport
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Render a collection-run report as HTML or JSON.
 *
 * `format` is supplied by the **client**, because only the client knows what the user chose — the
 * pre-P3 shell decided it by testing whether the chosen path ended in `.json`, and that test has to
 * happen before the command is called, not after.
 *
 * The report itself arrives over the wire rather than being read back from the workspace: the
 * renderer can export a run it has not saved yet. That is why `RunReport` is widened in
 * `@bifurc/protocol` to carry every field the renderer's real `RunnerRequestResult` has — a schema
 * narrower than its only caller truncates the JSON export without saying so.
 */
function runnerExportReport(params: RunnerExportReportParams): RunnerExportReportResult {
  const { report, format } = params;
  const folderName = report.folderName ?? "collection";
  const ts = new Date(report.startedAt).toISOString().replace(/[:.]/g, "-");

  const content = format === "json" ? JSON.stringify(report, null, 2) : renderRunnerHtml(report);
  const mimeType = format === "json" ? "application/json" : "text/html";

  // `params.workspaceId` (P14 WI 8) is for the blob half only: above `BLOB_INLINE_THRESHOLD_BYTES` a
  // report stages a blob, and the client reads it scoped to its own workspace. A *large* report is
  // exactly the case the threshold exists for, so omitting it made the engine's own export unreadable
  // by the client that requested it — see `blob/publish.ts`.
  return publishArtifact(content, `${folderName}-report-${ts}.${format}`, mimeType, params.workspaceId);
}
