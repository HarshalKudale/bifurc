/**
 * P2 work item 7 — the CommandRegistry.
 *
 * Deliberately built last within P2 (per the plan's own ordering note): it is the mechanical
 * payoff of items 1–6, not a prerequisite for them. Once every handler is registered here by its
 * `@bifurc/protocol` command name, `registerIpcHandlers()` becomes a thin adapter that maps the
 * legacy `namespace:verb` IPC channel onto `registry.invoke(action, payload, ctx)` — and P4's
 * transport can call `registry.invoke()` directly, with no `ipcMain` involved at all.
 *
 * This module validates every payload against the frozen Zod schema in
 * `packages/protocol/src/commands/index.ts` before the handler ever runs — the same schema P4's
 * transport and P1's own smoke test already exercise. An engine command is unreachable from any
 * transport unless the protocol package knows about it first.
 *
 * Scope of this pass (see `plan/03-phase-2-engine-extraction.md` work item 7 for the full
 * status note): the registry itself is complete and unit-tested. Only a handful of the simplest
 * `coreHandlers.ts` commands (`config.get`, `env.setActive`, `workspace.setActive`) have been
 * wired through it so far, proving the pattern end-to-end without changing the wire behaviour
 * `tests/ipc/handlers.test.ts` already pins. Converting the remaining ~100 handlers is *not*
 * purely mechanical: most of the CRUD channels (`mock:add`, `rule:update`, `ws:delete`, …) are
 * generated per-kind by `entityCrudFactory.ts` today, while the protocol collapses all of them
 * into six generic `entity.*` commands (P1 item 2). Registering those requires first collapsing
 * the factory's per-kind channels onto the generic shape — a data-modelling change, not a
 * mechanical find-and-replace — so it is left for a dedicated follow-up pass rather than rushed
 * here.
 */
import { type CommandAction, getCommandParamsSchema, isKnownCommand } from "@bifurc/protocol";
import type { bus } from "@/eventBus";

/** Carried into every handler — the bus today, session/auth once P4 exists (per the plan's own
 * description of `ctx`: "carries the session ... and the bus"). */
export interface CommandContext {
  bus: typeof bus;
}

export type CommandHandler<Params = unknown, Result = unknown> = (
  params: Params,
  ctx: CommandContext,
) => Result;

// `register()`'s handler params type is deliberately `unknown`, not narrowed per command: the
// frozen `packages/protocol` `COMMANDS` map widens every schema to `z.ZodType` via its internal
// `spec()` helper, so there is no stronger static type to recover without editing the frozen
// protocol package. Correctness comes from the runtime `schema.safeParse()` in `invoke()` below,
// not from the type system — handlers should narrow with the same schema type import if they
// want compile-time help (see `coreHandlers.ts` for the pattern).

/**
 * One handler per command name, validated against the frozen protocol schema. Deliberately
 * synchronous end-to-end (no `async invoke`): several existing handlers
 * (`config:get`, `env:setActive`, `workspace:setActive`) are synchronous today and
 * `tests/ipc/handlers.test.ts` asserts on their return value without `await`. Wrapping every
 * call in a `Promise` would silently change that contract. Handlers that are themselves async
 * still work — `invoke()` just returns whatever the handler returns, sync value or `Promise`,
 * unchanged.
 */
export class CommandRegistry {
  private readonly handlers = new Map<CommandAction, CommandHandler<any, any>>();

  register<A extends CommandAction>(
    action: A,
    // `any`, not `unknown` — TS's contravariant parameter checking would otherwise reject any
    // handler typed against its own command's narrower params shape (e.g.
    // `(params: EnvSetActiveParams) => ...`), which is exactly the pattern `coreHandlers.ts`
    // uses for compile-time help. Runtime safety still comes from `invoke()`'s
    // `schema.safeParse()`, not from this signature.
    handler: CommandHandler<any, unknown>,
  ): void {
    if (this.handlers.has(action)) {
      throw new Error(`Command "${action}" is already registered.`);
    }
    this.handlers.set(action, handler);
  }

  isRegistered(action: string): action is CommandAction {
    return isKnownCommand(action) && this.handlers.has(action);
  }

  /** Registered command names, for introspection and tests. */
  list(): CommandAction[] {
    return [...this.handlers.keys()];
  }

  /**
   * Validates `payload` against the command's frozen Zod schema, then calls its handler with
   * the parsed (typed, defaulted) params. Throws on an unknown command or a payload that fails
   * validation — callers (today: the `ipcMain.handle` adapter; later: P4's transport) are
   * expected to translate that into their own error shape.
   */
  invoke(action: CommandAction, payload: unknown, ctx: CommandContext): unknown {
    const handler = this.handlers.get(action);
    if (!handler) {
      throw new Error(`No handler registered for command "${action}".`);
    }
    const schema = getCommandParamsSchema(action);
    const parsed = schema.safeParse(payload);
    if (!parsed.success) {
      throw new Error(`Invalid payload for command "${action}": ${parsed.error.message}`);
    }
    return handler(parsed.data, ctx);
  }
}

/** Singleton — one registry per process, matching `bus`'s existing pattern. */
export const commandRegistry = new CommandRegistry();
