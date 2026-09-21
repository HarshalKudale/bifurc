/**
 * P14 WI 3 — the engine's **whole** command surface, in one call.
 *
 * ## Why this exists, and the gap it closes
 *
 * `createEngine()` registers **no** commands, by design: P3's rule is that *"the consumer declares what it
 * is willing to serve."* For the Electron shell that consumer is `src/ipc/handlers.ts#registerIpcHandlers()`,
 * which calls all eighteen `register*Commands()` functions. P6 §5c moved every one of them from a module
 * side effect to an explicit call site for a good reason (an explicit call is checkable; an import side
 * effect is not).
 *
 * The consequence, which **nothing had exercised until a host existed**: those eighteen calls live in the
 * **shell**, so a headless consumer gets an empty registry. The first `createWorkspaceHost()` call answered
 * `UNKNOWN_COMMAND` for `config.get` — a command the protocol declares and `@bifurc/client` advertises. That
 * is the *exact* class of bug §5c was written about, one layer up: the shell was registering them, so the
 * ratchet could not see that a non-shell consumer had nothing behind them.
 *
 * ## Why the list is here and not copied into `host.ts`
 *
 * A host that listed eighteen imports itself would be the *second* copy of this list, and the two would
 * drift — with the failure mode being silent `UNKNOWN_COMMAND` for whatever was added to one and not the
 * other. So the list is engine-side, and both consumers call it:
 *
 * - the host calls it (a deployable unit has no shell at all);
 * - `registerIpcHandlers()` **keeps its own explicit calls** rather than being rewritten to call this.
 *   That is deliberate and not redundancy-for-its-own-sake: the shell's ordering comments are load-bearing
 *   documentation of P6's findings (which channel's deletion made which call the only registration), and
 *   collapsing them into one opaque call would delete that record. The two lists are kept honest by a test
 *   that asserts this function registers a superset of the shell's set — see
 *   `tests/integration/workspaceHost.integration.test.ts`.
 *
 * ## Not a side effect, still
 *
 * This function must be **called**, and that is the §5c rule preserved rather than undone. What changed is
 * that there is now a non-shell way to call it.
 */
import type { CommandRegistry } from "./commands/registry";
import { registerBlobCommands } from "./blob/commands";
import { registerImportExportCommands } from "./importExport/commands";
import { registerFileOpsCommands } from "./fileOps/commands";
import { registerCertCommands } from "./proxy/certCommands";
import { registerServerCommands } from "./proxy/serverCommands";
import { registerWebhookCommands } from "./proxy/webhookCommands";
import { registerConfigCommands } from "./store/configCommands";
import { registerMiscCommands } from "./miscCommands";
import { registerRunnerCommands } from "./runner/runnerCommands";
import { registerAuditCommands } from "./store/auditCommands";
import { registerCoreCommands } from "./store/coreCommands";
import { registerFolderCommands } from "./store/folderCommands";
import { registerGraphqlCommands } from "./protocols/graphqlCommands";
import { registerSoapCommands } from "./protocols/soapCommands";
import { registerGrpcCommands } from "./protocols/grpcCommands";
import { registerApplicationCommands } from "./applications/applicationCommands";
import { registerSyncCommands } from "./sync/syncCommands";
import { registerEntityCrudCommands } from "./store/entityCrudCommands";

/**
 * Register every engine command on `registry`.
 *
 * **Not idempotent, and must not be called twice on the same registry** — `CommandRegistry.register()`
 * throws on a duplicate *on purpose* (TRAPS.md, "Registering a command twice throws"). A caller that needs
 * both a host and the shell in one process has one registry too many; that is a design error, not something
 * this function should paper over by swallowing the throw.
 *
 * Order is preserved from `registerIpcHandlers()` so the two lists diff cleanly. It is not otherwise
 * significant: no `register*Commands()` here depends on another having run — the P6 ledger records the two
 * that once did (`server.*`, `runner.*`) as **dissolved** when their legacy channels were deleted.
 */
export function registerAllEngineCommands(registry: CommandRegistry): void {
  registerBlobCommands(registry);
  registerImportExportCommands(registry);
  registerFileOpsCommands(registry);
  registerCertCommands(registry);
  registerServerCommands(registry);
  registerConfigCommands(registry);
  registerWebhookCommands(registry);
  registerMiscCommands(registry);
  registerRunnerCommands(registry);
  registerAuditCommands(registry);
  registerCoreCommands(registry);
  registerFolderCommands(registry);
  registerGraphqlCommands(registry);
  registerSoapCommands(registry);
  registerGrpcCommands(registry);
  registerApplicationCommands(registry);
  registerSyncCommands(registry);
  registerEntityCrudCommands(registry);
}
