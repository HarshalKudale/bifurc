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
 * the *method name*; `renderer/panels/AuditLogPanel.tsx:61–62` reaches the same command through
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
 * `audit.diff` is `audit.list`'s sibling and is *also* registered from the shell — at **module scope**
 * in `src/ipc/handlers/syncHandlers.ts`. It is **not** moved here, and the reason is `plan/07` §5c's
 * trap rather than tidiness: a module-scope registration takes effect on *import*, so moving it behind
 * an explicit `registerAuditCommands()` call would silently unregister it for the suites that import
 * the handler group without running `registerIpcHandlers()`. Moving already-registered commands is a
 * separate pass with its own blast radius.
 */
import type { AuditListParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";
import { queryLog, type QueryLogOptions } from "./gitStore";
import { loadConfig } from "./config";

export function registerAuditCommands(registry: CommandRegistry): void {
    registry.register("audit.list", async (params: AuditListParams) => {
        // `workspaceId` is optional in the protocol because the shell's channel defaulted it; the
        // store requires it. Resolving it here rather than in the shell is what makes the command
        // answerable by a client that has no idea which workspace is active.
        const workspaceId = params.workspaceId ?? loadConfig().activeWorkspaceId;
        return queryLog({ ...params, workspaceId } as QueryLogOptions);
    });
}
