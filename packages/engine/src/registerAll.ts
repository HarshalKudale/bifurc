/**
 * P14 WI 3 — the engine's **whole** surface, in one call: the commands, *and* the CRUD kinds they read.
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
 * ## The same bug, one layer *down* — found 2026-09-23 (`plan/15` work item 10)
 *
 * Moving the eighteen commands engine-side fixed the **names**. It did not fix what the four `entity.*`
 * commands **read**: a per-kind options container that five `register*Kinds()` files populated — and those
 * files were still in the shell. So `registerAllEngineCommands()` registered `entity.create`/`update`/
 * `delete` and left every container empty, and a headless host threw
 * `kind "<kind>" is not routed through the CommandRegistry yet` for every kind while
 * `isRegistered("entity.delete")` was **true**. Measured, with a control; the table is in
 * `registerAllEngineKinds()`'s docblock below.
 *
 * **The lesson generalises, and it is why the kinds are registered from *this* file rather than a second
 * function nobody calls:** *a command's registration is not the whole of its dependency.* A list that
 * enumerates commands and stops can be complete and still leave the surface half-dead.
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
 *   `tests/integration/workspaceHost.integration.test.ts`, which now guards **both** axes: the command names
 *   and the kinds.
 *
 * ## Not a side effect, still
 *
 * This function must be **called**, and that is the §5c rule preserved rather than undone. What changed is
 * that there is now a non-shell way to call it.
 */
import type { CommandRegistry } from "./kernel/registry";
import { registerBlobCommands } from "./blob/commands";
import { registerImportExportCommands } from "./importexport/commands";
import { registerFileOpsCommands } from "./fileops/commands";
import { registerCertCommands } from "./certificates/certCommands";
import { registerServerCommands } from "./proxy/serverCommands";
import { registerWebhookCommands } from "./webhooks/webhookCommands";
import { registerConfigCommands } from "./workspace/entities/configCommands";
import { registerRestCommands } from "./rest/commands";
import { registerMockCommands } from "./mocks/commands";
import { registerHealthbarCommands } from "./workspace/healthbarCommands";
import { registerRunnerCommands } from "./runner/runnerCommands";
import { registerAuditCommands } from "./workspace/entities/auditCommands";
import { registerCoreCommands } from "./workspace/entities/coreCommands";
import { registerFolderCommands } from "./workspace/entities/folderCommands";
import { registerGraphqlCommands } from "./graphql/commands";
import { registerSoapCommands } from "./soap/commands";
import { registerGrpcCommands } from "./grpc/commands";
import { registerApplicationCommands } from "./applications/applicationCommands";
import { registerSyncCommands } from "./gitops/syncCommands";
import { registerEntityCrudCommands } from "./workspace/entities/entityCrudCommands";
import { registerSoapKinds } from "./soap/kinds";
import { registerGraphqlKinds } from "./graphql/kinds";
import { registerGrpcKinds } from "./grpc/kinds";
import { registerCrudKinds } from "./workspace/entities/kinds/crudKinds";
import { registerCoreKinds } from "./workspace/entities/kinds/coreKinds";

/**
 * Register every CRUD **kind** the four `entity.*` commands dispatch through.
 *
 * ## A kind is not a command, and the drift guard could not see the difference
 *
 * `entity.create` is **one** command serving sixteen kinds; it learns which kind it was handed by looking
 * the name up in a container that `registerCrudKind()` / `registerSimpleKind()` populate. So the two axes
 * are not comparable by one list, and `registerAllEngineCommands()`'s drift guard — which compares
 * **command names** — was structurally blind to this one.
 *
 * ## The defect this closes (2026-09-23, `plan/15` work item 10)
 *
 * The five `register*Kinds()` files lived in the **Electron shell's** `src/ipc/handlers/`, so
 * `registerAllEngineCommands()` — a headless host's only registration path — registered
 * `entity.create`/`update`/`delete` and left every container empty. Measured, with a control:
 *
 * | after | `entity.delete({kind:"mocks"})` |
 * |---|---|
 * | `registerAllEngineCommands(reg)` | `kind "mocks" is not routed through the CommandRegistry yet` |
 * | …then `registerCrudKind({kind:"mocks",…})` | **`{ok:true}`** |
 *
 * Every CRUD kind and every simple kind answered the same way, and `isRegistered("entity.delete")` was
 * **`true`** throughout — so `@bifurc/client` advertised a command that could not run, and the error
 * blamed the *kind* rather than the host. That is this file's own bug class one layer down: the commands
 * moved engine-side, the containers they consult did not. The files now live in
 * `packages/engine/src/workspace/entities/kinds/` (phase 5 moved them out of
 * `packages/engine/src/store/kinds/`).
 *
 * ## The shell keeps its own five explicit calls
 *
 * `src/ipc/handlers.ts` still calls all five itself rather than calling this, and that is the same
 * deliberate non-redundancy as the eighteen command calls: each carries a comment naming the P6 reduction
 * that made it the only registration, and collapsing them would delete that record. A test keeps the two
 * lists honest — see `tests/integration/workspaceHost.integration.test.ts`'s kinds guard.
 */
export function registerAllEngineKinds(): void {
  registerSoapKinds();
  registerGraphqlKinds();
  registerGrpcKinds();
  registerCrudKinds();
  registerCoreKinds();
}

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
 *
 * **It registers the kinds too**, and that is not scope creep: a consumer that registers the four
 * `entity.*` commands without their containers has four commands that throw. "One call registers the
 * engine's whole surface" has to include the half the commands read, or the sentence is false — which is
 * exactly how this defect survived (2026-09-23, `plan/15` work item 10).
 */
export function registerAllEngineCommands(registry: CommandRegistry): void {
  registerAllEngineKinds();
  registerBlobCommands(registry);
  registerImportExportCommands(registry);
  registerFileOpsCommands(registry);
  registerCertCommands(registry);
  registerServerCommands(registry);
  registerConfigCommands(registry);
  registerWebhookCommands(registry);
  registerRestCommands(registry);
  registerMockCommands(registry);
  registerHealthbarCommands(registry);
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
