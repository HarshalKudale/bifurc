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
 * status note): the registry itself is complete and unit-tested, and roughly 112 commands
 * across `coreHandlers.ts`, the CRUD-factory channels (`entityCrudFactory.ts` — `mock:add`,
 * `rule:update`, `ws:delete`, `env:add`, and every other kind collapse onto `entity.create`/
 * `entity.update`/`entity.delete`), `entity:load`/`entity:setEnabled`, the no-`AppConfig`-array
 * kinds (`graphqlSchemas`/`protoFiles`/`wsdls`, via `entity.create`/`entity.delete`/`entity.list`
 * and `registerSimpleEntityHandlers()`), and most of `syncHandlers.ts`/`folderHandlers.ts`/
 * `tlsHandlers.ts`/`runnerHandlers.ts`/`graphqlHandlers.ts`/`soapHandlers.ts`/`grpcHandlers.ts`/
 * `applicationHandlers.ts` now go through it. `environments` — the one kind with a create-gate
 * check (`CrudFactoryOpts.gateKind`) — is now routed too. Still outside its scope: the P3-bound
 * `importExport:*` SPLIT channels.
 */
import { type CommandAction, getCommandParamsSchema, isKnownCommand, EngineError } from "@bifurc/protocol";
import type { bus } from "../eventBus";

/**
 * The authorisation vocabulary — what a session is allowed to do.
 *
 * Declared **here**, next to `CommandContext`, rather than in `transport/auth/scopes.ts` which owns the
 * command → scope table: `transport/` imports `commands/` and never the reverse (all four transports
 * import `CommandContext` from this file), so the table re-exports this type instead of the other way
 * round. One vocabulary, no inverted dependency.
 *
 * A flat set with **no hierarchy** — `admin` does not imply `read` — so "what may this session reach?"
 * is a lookup rather than a derivation. The per-command assignment and the `SHELL_SCOPES` /
 * `COMPANION_SCOPES` / `READ_ONLY_SCOPES` presets live in `transport/auth/scopes.ts`.
 */
export type Scope = "read" | "write" | "execute" | "admin";

/**
 * Who is invoking a command, when the caller is a session rather than the engine's own code.
 *
 * `clientName`/`clientVersion` come from the `hello` handshake. `HelloRequestSchema` has declared both
 * since P1 with the comment "recorded per session so the engine can log and diagnose which build is
 * connected" — and nothing recorded them until now. They live here rather than being re-derived at
 * each use site because the handshake is the only thing that knows them.
 */
export interface SessionIdentity {
  sessionId: string;
  clientName?: string;
  clientVersion?: string;
  /**
   * The peer address, when the transport has one.
   *
   * `ws` has a real address. A unix socket or a named pipe does **not** — its path names the engine's
   * own endpoint, not the client — so `socket` omits this rather than recording something a reader
   * would take for a client identifier. `in-process` and `stdio` have no peer at all.
   *
   * So absent is the normal case, and it must stay distinguishable from "unknown": that is why this is
   * optional rather than defaulted to a placeholder string.
   */
  peer?: string;
  /**
   * The scopes granted at the handshake — the session's authority, as distinct from its identity.
   *
   * Optional, but **absent is not "none"**: it means an identity was reported without an authority, and
   * `callerHasScope()` below refuses anything that needs a scope rather than assuming one. A transport
   * that establishes sessions should always report these; the field is optional only so a caller wiring
   * `onSession` by hand (the conformance runners do) is not forced to invent a set.
   */
  scopes?: ReadonlySet<Scope>;
}

/**
 * Carried into every handler — the bus today, and the session now that P4 exists (per the plan's own
 * description of `ctx`: "carries the session ... and the bus").
 *
 * `session` is **absent** for the engine's own callers and for `in-process`/`stdio`. Neither of those
 * performs a handshake, so there is no identity to record and inventing one would put a fiction in the
 * audit log; "the engine called itself" is the honest answer for a direct `registry.invoke()`.
 *
 * It is filled in **after** construction, which is why the field is mutable: a transport builds one
 * context per session, and the auth decorator sets this once the handshake succeeds — the session id
 * does not exist before then. The consequence to respect is that a transport must hand the *same
 * object* to its inner transport and to the decorator, or the identity lands on a context nobody reads.
 */
export interface CommandContext {
  bus: typeof bus;
  session?: SessionIdentity;
}

/**
 * Whether the caller's authority covers `scope`.
 *
 * **A caller with no session is the engine's own** — `in-process`, `stdio` and the legacy
 * `ipcMain.handle` adapters all invoke with a session-less context, because none of them crosses a
 * boundary. Such a caller holds every scope: there is nobody to defend against, and refusing would
 * break the shell's own behaviour for no gain.
 *
 * **A caller with a session but no reported scopes is refused.** Unknown authority fails closed; the
 * alternative is that forgetting to report scopes silently grants them.
 */
export function callerHasScope(ctx: CommandContext, scope: Scope): boolean {
  if (!ctx.session) return true;
  return ctx.session.scopes?.has(scope) ?? false;
}

/**
 * May this caller affect entities it did **not** name?
 *
 * This is the rule that keeps a lower-trust client additive. Creating a mock normally resolves
 * conflicts by *disabling* an existing enabled mock with the same signature, and enabling one does the
 * same — so a plain `write` would let a caller reach entities it never mentioned, discovered by
 * scanning config. That is a different power from `entity.update`, which is `write` because it mutates
 * an entity the caller **identified**.
 *
 * So it requires `admin` — which is exactly what the companion's `{read, write}` set lacks. That is how
 * the browser extension's additive-only guarantee survives the move onto the registry: it falls out of
 * the existing total scope table rather than needing a rule written specially for it.
 *
 * Deliberately a **named** predicate rather than a bare `callerHasScope(ctx, "admin")` at each site:
 * the policy is the thing being expressed, and the two conflict sites (`entity.create`'s
 * `onAddConflict` and `entity.setEnabled`'s conflict loop) must agree about it.
 */
export function mayAffectUnnamedEntities(ctx: CommandContext): boolean {
  return callerHasScope(ctx, "admin");
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
   * the parsed (typed, defaulted) params.
   *
   * **Throws `EngineError`** (`@bifurc/protocol`) for the two conditions the caller must be able to
   * report as distinct wire errors — `UNKNOWN_COMMAND` and `BAD_REQUEST` — so a transport can
   * translate them without matching on message text. The messages are unchanged from the plain
   * `Error`s these used to be, so message-matching assertions elsewhere still hold; what is new is
   * that the code travels with the message.
   *
   * Callers (today: the `ipcMain.handle` adapter; later: P4's transport) still own the translation
   * into their own error shape — but they now have something to translate *from*. Before P4 this
   * method threw a bare `Error`, which left a transport with no choice but to pattern-match
   * `"Invalid payload for command"` to distinguish a client error from an engine bug.
   *
   * A handler that throws is passed through untouched: its own error is the most specific
   * information available, and re-wrapping it would lose an `EngineError`'s code.
   */
  invoke(action: CommandAction, payload: unknown, ctx: CommandContext): unknown {
    const handler = this.handlers.get(action);
    if (!handler) {
      throw new EngineError("UNKNOWN_COMMAND", `No handler registered for command "${action}".`);
    }
    const schema = getCommandParamsSchema(action);
    const parsed = schema.safeParse(payload);
    if (!parsed.success) {
      throw new EngineError(
        "BAD_REQUEST",
        `Invalid payload for command "${action}": ${parsed.error.message}`,
      );
    }
    return handler(parsed.data, ctx);
  }
}

/** Singleton — one registry per process, matching `bus`'s existing pattern. */
export const commandRegistry = new CommandRegistry();
