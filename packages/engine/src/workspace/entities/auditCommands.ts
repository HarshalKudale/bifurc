/**
 * `audit.list` — the audit-log query.
 *
 * ## Why this needed a protocol change rather than a move
 *
 * It was classified `NARROWED` in the step-3b ratchet: `AuditListParams` was `.strict()` and lacked
 * `filePath` / `fromTs` / `toTs`, while `QueryLogOptions` — the type the handler actually takes — has
 * all three. So the schema was not merely narrower than the handler; `.strict()` made the difference
 * a `BAD_REQUEST` rather than a silently dropped filter.
 *
 * The ratchet called that narrowing "**latent, not live**", on the grounds that no renderer code calls
 * `listAudit`. **That was wrong, and the way it was wrong is worth keeping.** The check was a grep for
 * the *method name*; `packages/gui/src/panels/AuditLogPanel.tsx:61–62` reaches the same command through
 * `window.api` and sets `fromTs`/`toTs` whenever its date filter is used. Routing this command before
 * widening the schema would have broken the audit panel's date filter — a user-visible regression
 * that every unit test would have passed through, because the tests drive the *handler*, not the
 * panel.
 *
 * The schema was widened on 2026-09-19 (`AuditListParams`), and this module is the engine half.
 *
 * ## Two fields the schema deliberately leaves loose
 *
 * `entity` and `action` are `z.string()` in the protocol, while `queryLog` narrows them to the
 * `AuditEntity` / `AuditAction` unions. That is the pre-existing shape and it stays: the shell handler
 * took `QueryLogOptions` and cast, and the renderer's `AuditListOptions` already types both as the
 * unions. Tightening the schema here would *narrow* what a client may send — a behaviour change
 * smuggled in under a widening — so the cast below is the honest expression of "the protocol accepts
 * a string, the store knows the closed set".
 *
 * ## What is deliberately not here
 *
 * `audit.diff` is `audit.list`'s sibling and is registered **elsewhere** — in `sync/syncCommands.ts`
 * behind `registerSyncCommands()` — because it reads the same git history the `history.*` commands do,
 * whereas `audit.list` queries the log. Different dependency, different module.
 *
 * That placement is the resolution of `plan/07` §5c's trap, and this docblock used to state the trap
 * against a now-superseded fact: it said `audit.diff` was registered at **module scope** in
 * `src/ipc/handlers/syncHandlers.ts`. A module-scope registration takes effect on *import*, so moving
 * it behind an explicit `register*Commands()` call would silently unregister it for the suites that
 * import the handler group without running `registerIpcHandlers()`. Step 3e slice 6 (2026-09-19)
 * moved it with the other 16 `sync.*` / `git.*` / `history.*` commands and made `registerSyncCommands()`
 * the explicit call, and both integration suites were re-pointed to make that call themselves — which
 * is exactly what the trap said was required. The shell file has since been deleted outright (P6
 * reduction, second group, 2026-09-21 — `plan/07` §5b-terdecies).
 */
import type { AuditListParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../../kernel/registry";
import { queryLog, type QueryLogOptions } from "../../store/gitStore";
import { loadConfig } from "../config";

export function registerAuditCommands(registry: CommandRegistry): void {
    registry.register("audit.list", async (params: AuditListParams) => {
        // `workspaceId` is optional in the protocol because the shell's channel defaulted it; the
        // store requires it. Resolving it here rather than in the shell is what makes the command
        // answerable by a client that has no idea which workspace is active.
        const workspaceId = params.workspaceId ?? loadConfig().activeWorkspaceId;
        return queryLog({ ...params, workspaceId } as QueryLogOptions);
    });
}
