/**
 * Runner report **persistence** — the two `window.api` wrappers the runner tab needs.
 *
 * ## What left this file, and why
 *
 * `generateHtmlReport` was here until 2026-09-30 (`plan/19` WI 4): a **second** HTML report renderer, the
 * engine's being `runner/reportHtml.ts`, which both `runner.saveReport` and `runner.exportReport` use. It was
 * row 5 of the plan's duplication census, and the boundary ratchet carried it as
 * `lib/runnerReport.ts#reportRenderer` until the item that owned it deleted it.
 *
 * What is left is not a renderer at all — it is two calls to the **storage** commands, which are unchanged and
 * stay: a finished run is still persisted into the folder's `.runs/` history by the engine, and this is the
 * UI's handle on that. So the *file* survives while the *construct* the ratchet tracked does not, which is
 * exactly the distinction that ratchet is keyed on (`file#construct`, not file).
 */
import { CollectionRunReport } from "@/types";

/** Save a runner report to the filesystem, through `runner.saveReport`. */
export async function saveRunnerReport(wsId: string, report: CollectionRunReport): Promise<{ ok: boolean; error?: string }> {
    return window.api.saveRunnerReport(wsId, report);
}

/** Get saved run history for a folder, through `runner.getHistory`. */
export async function getRunHistory(wsId: string, folderId: string): Promise<{ timestamp: number; summary: { total: number; passed: number; failed: number } }[]> {
    return window.api.getRunHistory(wsId, folderId);
}
