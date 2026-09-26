/**
 * P4 work item 4 — authentication and authorisation, tested without a socket.
 *
 * This is the payoff of building auth as a **decorator**: every case below runs over `in-process`, so
 * the handshake and the scope gate are both exercised before `ws` exists. `ws` stays
 * behind its hard gate — "an unauthenticated remote RPC surface is a remote code execution hole" —
 * and this file is what makes that gate satisfiable rather than merely blocking.
 *
 * **The token check was the third thing in that list until 2026-09-25**, and its removal is recorded
 * where the token's own cases used to be — see "the token, and why there is nothing here" below. The
 * short version: what is left is the half of work item 4 that was never about a credential, and the
 * boundary that replaced it lives in `transport/ws.ts` (the bind) and `lib/net.ts` (the origin).
 *
 * ## What is deliberately asserted as a *property* rather than a spot check
 *
 * The scope table is 93 entries and the interesting failures are not "is `tls.generate` admin?" but
 * "can a session holding only `read` reach anything that writes?", and "can the companion extension
 * reach anything destructive?". Those are sweeps over the whole table, so a future re-scoping cannot
 * quietly open a hole that a spot check would not notice.
 */
import { describe, expect, it } from "vitest";
import { COMMAND_FIXTURES, EngineError, PROTOCOL_VERSION, type RpcError } from "@bifurc/protocol";
import {
  CommandRegistry,
  type CommandContext,
  type SessionIdentity,
} from "../../src/commands/registry";
import { EngineEventBus } from "../../src/eventBus";
import { EventLog, type EventLogOptions } from "../../src/transport/eventLog";
import { createInProcessTransport } from "../../src/transport/inProcess";
import {
  assertScopesAreTotal,
  commandIsAllowed,
  COMPANION_SCOPES,
  createAuthenticatedTransport,
  HELLO_ACTION,
  READ_ONLY_SCOPES,
  requiredScopeFor,
  SCOPE_BY_COMMAND,
  SCOPES,
  SHELL_SCOPES,
  type Scope,
} from "../../src/transport/auth";
import type { EngineEvent, Transport } from "../../src/transport/types";

const READ_COMMAND = "config.get";
const WRITE_COMMAND = "folder.delete";
const EXECUTE_COMMAND = "script.execute";
const ADMIN_COMMAND = "tls.generate";

const VALID_HELLO = {
  protocolVersion: PROTOCOL_VERSION,
  clientName: "cli" as const,
  clientVersion: "0.0.0-test",
};

/**
 * The protocol's own valid payload for a command.
 *
 * Needed because `CommandRegistry.invoke()` validates against the frozen Zod schema *before* running
 * the handler, so `{}` for `folder.delete` is a `BAD_REQUEST` rather than a call. Reusing
 * `COMMAND_FIXTURES` rather than hand-writing payloads keeps these tests from drifting away from the
 * protocol — the same table the transport matrix is built from.
 */
function validPayload(cmd: string): unknown {
  const fixture = (COMMAND_FIXTURES as Record<string, { valid: unknown } | undefined>)[cmd];
  if (fixture === undefined) throw new Error(`no COMMAND_FIXTURES entry for "${cmd}"`);
  return fixture.valid;
}

// `tempDirs`, an `afterEach` that removed them, and `tempDir()` stood here until 2026-09-25. All three
// existed for one client: the `engine.token` file, which the deleted token block above was the only thing
// to create, read and stat. With that gone there is nothing on this path that touches the filesystem, so
// `node:fs/promises`, `node:os` and `node:path` are no longer imported at all — this file is now pure.

// ── the scope table ──────────────────────────────────────────────────────────

describe("scopes", () => {
  it("keys @bifurc/protocol's COMMANDS exactly", () => {
    // `assertScopesAreTotal()` runs at import time; this asserts it did not throw and that the table
    // is non-trivial, so a refactor that emptied it cannot pass by vacuously covering nothing.
    expect(() => assertScopesAreTotal()).not.toThrow();
    expect(Object.keys(SCOPE_BY_COMMAND).length).toBeGreaterThan(50);
  });

  it("throws naming the commands that would be denied, if the table is short", () => {
    // Teeth: the guard's whole value is that an unscoped command cannot be served, so a guard that
    // cannot fail is decoration. The scenario is the real one — the protocol gains a command and the
    // table is not updated — simulated by handing it a command list with one extra entry.
    const real = Object.keys(COMMAND_FIXTURES);
    expect(() => assertScopesAreTotal(real)).not.toThrow();

    const err = (() => {
      try {
        assertScopesAreTotal([...real, "brand.new.command"]);
      } catch (e) {
        return e as Error;
      }
      return undefined;
    })();
    expect(err?.message).toMatch(/Unscoped/);
    expect(err?.message).toMatch(/brand\.new\.command/);
  });

  it("throws for a scope assigned to something that is not a command", () => {
    // The other direction, and the other real scenario: a command is removed from the protocol while
    // the table keeps its entry, which reads like coverage while covering nothing.
    const withoutOne = Object.keys(COMMAND_FIXTURES).filter((c) => c !== READ_COMMAND);
    expect(() => assertScopesAreTotal(withoutOne)).toThrow(/Scoped but not a command/);
    expect(() => assertScopesAreTotal(withoutOne)).toThrow(/config\.get/);
  });

  it("assigns only scopes that exist", () => {
    const known = new Set<Scope>(SCOPES);
    for (const [cmd, scope] of Object.entries(SCOPE_BY_COMMAND)) {
      expect(known.has(scope), `${cmd} has unknown scope ${scope}`).toBe(true);
    }
  });

  it("fails closed for a command that is not in the table", () => {
    // The security-relevant default. `undefined` is not a "no scope needed" sentinel.
    expect(requiredScopeFor("not.a.command")).toBeUndefined();
    expect(commandIsAllowed(SHELL_SCOPES, "not.a.command")).toBe(false);
  });

  it("spots the plan's intent on the commands it names explicitly", () => {
    expect(requiredScopeFor(READ_COMMAND)).toBe("read");
    expect(requiredScopeFor(WRITE_COMMAND)).toBe("write");
    expect(requiredScopeFor(EXECUTE_COMMAND)).toBe("execute");
    expect(requiredScopeFor(ADMIN_COMMAND)).toBe("admin");
  });

  it("lets the shell reach every command", () => {
    // The shell's session *is* the user. A command the shell cannot run is a command nobody can.
    const unreachable = Object.keys(SCOPE_BY_COMMAND).filter((c) => !commandIsAllowed(SHELL_SCOPES, c));
    expect(unreachable).toEqual([]);
  });

  it("lets a read-only session reach reads and nothing else", () => {
    const reachable = Object.keys(SCOPE_BY_COMMAND).filter((c) => commandIsAllowed(READ_ONLY_SCOPES, c));
    expect(reachable.length).toBeGreaterThan(0);
    for (const cmd of reachable) {
      expect(requiredScopeFor(cmd), `${cmd} must be read-only`).toBe("read");
    }
  });

  it("cannot reach anything destructive from the companion extension's scopes", () => {
    // `plan/05`: "The companion extension keeps a `write`-only, non-destructive scope — which is what
    // ALLOWED_ACTIONS approximates today." Swept rather than spot-checked: "non-destructive" is a
    // claim about the whole table, and the interesting regression is the one nobody thought to check.
    const reachable = Object.keys(SCOPE_BY_COMMAND).filter((c) => commandIsAllowed(COMPANION_SCOPES, c));
    const privileged = reachable.filter((c) => {
      const scope = requiredScopeFor(c);
      return scope === "execute" || scope === "admin";
    });
    expect(privileged).toEqual([]);
    expect(reachable).toContain(READ_COMMAND);
    expect(reachable).not.toContain(ADMIN_COMMAND);
    expect(reachable).not.toContain(EXECUTE_COMMAND);
  });

  it("does not treat admin as implying read", () => {
    // Documented as a decision, so it is asserted as one: scopes are an explicit set and a hierarchy
    // would make "what can this session reach?" a derivation instead of a lookup.
    const adminOnly = new Set<Scope>(["admin"]);
    expect(commandIsAllowed(adminOnly, ADMIN_COMMAND)).toBe(true);
    expect(commandIsAllowed(adminOnly, READ_COMMAND)).toBe(false);
  });
});

// ── the token, and why there is nothing here ─────────────────────────────────
//
// **Nine cases stood here until 2026-09-25**, and they are not skipped or weakened — they are gone,
// because the thing they tested is gone. `generateToken`, `verifyToken`, `ensureEngineToken`,
// `readEngineToken` and `ENGINE_TOKEN_ENV_VAR` were all deleted with the token itself, so every one of
// them would now be a test of a symbol that does not exist.
//
// The list is worth keeping, because "the token tests were deleted" and "the token tests were lost" are
// different events and this record is what tells them apart:
//
//   - "generates a fresh 256-bit base64url token each time" — `generateToken`
//   - "accepts the matching token and rejects a different one" — `verifyToken`
//   - "rejects a token that is only a correct prefix" — the `timingSafeEqual` guard
//   - "rejects non-strings without throwing" — the same guard, from the other side
//   - "writes the token where a client can read it" — `ensureEngineToken`
//   - "rotates on every call, so a token does not survive a restart" — the session-secret property
//   - "writes the token file 0600 where the platform has permission bits" — the file's mode
//   - "takes the token from the environment and writes no file" — the container path
//   - "ignores an empty environment variable rather than trusting it", and the two file-shape cases
//
// **What replaced them is not a weaker version of the same thing, and that is the point of deleting
// rather than porting.** A credential test asserts that a *secret* is compared correctly. The engine no
// longer has a secret to compare, so the boundary moved to two places that are not secrets at all: the
// loopback bind (`transport/ws.ts`, pinned in `ws.test.ts`) and the `Origin` guard on the upgrade
// (`lib/net.ts`'s `isAllowedOrigin`, pinned in `lib/net.test.ts` and driven end-to-end in `ws.test.ts`).
// Neither can be "rotated", and neither has a file mode.
//
// The **one** case here that outlived its subject is the credential half of the handshake, and it now
// asserts the opposite: `protocol.conformance.ts` has "accepts a `hello` that still carries the removed
// `token` field", which is the migration story rather than a leftover.

// ── the decorator ────────────────────────────────────────────────────────────

interface Harness {
  transport: Transport;
  bus: EngineEventBus;
  inner: Transport;
  failures: RpcError[];
  /**
   * The log the decorator was given, when the harness was asked for one.
   *
   * `undefined` by default, so the existing cases keep exercising the "no log" path — which is not a
   * degenerate case but the honest answer for a transport wrapped without one: a client that presents
   * a `lastSeq` is told `resyncRequired` rather than being promised a replay nobody can perform.
   */
  log?: EventLog;
}

function harness(
  opts: Partial<Parameters<typeof createAuthenticatedTransport>[1]> = {},
  harnessOpts: { log?: Partial<Omit<EventLogOptions, "bus">> } = {},
): Harness {
  const registry = new CommandRegistry();
  const bus = new EngineEventBus();
  registry.register(READ_COMMAND, () => ({ reached: READ_COMMAND }));
  registry.register(WRITE_COMMAND, () => ({ reached: WRITE_COMMAND }));
  registry.register(EXECUTE_COMMAND, () => ({ reached: EXECUTE_COMMAND }));
  registry.register(ADMIN_COMMAND, () => ({ reached: ADMIN_COMMAND }));

  // `{ log: {} }` means "a log with the real defaults"; `{ log: { maxEvents: 2 } }` means a tight one,
  // which is how a case can force the retention window to move without emitting 1,000 events.
  const log = harnessOpts.log === undefined ? undefined : new EventLog({ bus, ...harnessOpts.log });

  const inner = createInProcessTransport(registry, { bus, ctx: { bus }, log });
  const failures: RpcError[] = [];
  const transport = createAuthenticatedTransport(inner, {
    // **No `token`.** It was the first field here, and the harness threaded it through `opts` so a case
    // could present the *wrong* one — see `authenticated()` below, which did exactly that. There is no
    // credential now, so "a wrong token" is not a state this harness can be put into; the one identity
    // failure the decorator still has (a request that is not `hello`) is driven directly instead.
    engineVersion: "0.3.3",
    onAuthFailure: (error) => failures.push(error),
    ...(log === undefined ? {} : { log }),
    ...opts,
  });
  return { transport, bus, inner, failures, log };
}

/** A harness already past the handshake. */
async function authenticated(
  opts: Partial<Parameters<typeof createAuthenticatedTransport>[1]> = {},
  harnessOpts: { log?: Partial<Omit<EventLogOptions, "bus">> } = {},
): Promise<Harness> {
  const h = harness(opts, harnessOpts);
  // The payload no longer carries a token; sending it **first** is the rule this call satisfies.
  await h.transport.request(HELLO_ACTION, { ...VALID_HELLO });
  return h;
}

describe("the hello handshake", () => {
  it("resolves with the protocol's response shape", async () => {
    const { transport } = harness();
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
    })) as Record<string, unknown>;

    expect(response.protocolVersion).toBe(PROTOCOL_VERSION);
    expect(response.engineVersion).toBe("0.3.3");
    expect(response.capabilities).toEqual([]);
    expect(typeof response.sessionId).toBe("string");
    expect(response.sessionId).not.toBe("");
  });

  it("advertises the capabilities it was given", async () => {
    const { transport } = harness({ capabilities: ["blob", "events.replay"] });
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
    })) as { capabilities: string[] };
    expect(response.capabilities).toEqual(["blob", "events.replay"]);
  });

  it("reports the host's own domain when it was told one — the transport option, not `--domain`", async () => {
    // The only route a domain has to a client: the host that holds one passes it down as an auth option, and this
    // decorator is what turns it into a wire field. Pinned here because this is where the *shape* is decided;
    // `tests/integration/workspaceHost.integration.test.ts` is where the host-to-wire wiring is pinned, and the
    // two are different questions.
    //
    // **The value is arbitrary on purpose.** `plan/15` work item 9's `--domain` flag was removed on 2026-09-24,
    // so the only host that exists passes `localhost` — but the *option* is still a free string, and this case is
    // about the decorator's shape rather than about which host supplies what. A case that only ever saw
    // `localhost` could not tell "passed through" from "defaulted".
    const { transport } = harness({ hostDomain: "engine.example.com" });
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
    })) as { hostDomain?: string };
    expect(response.hostDomain).toBe("engine.example.com");
  });

  it("OMITS hostDomain when it was not told one, rather than inventing localhost", async () => {
    // **The absence is the assertion, and `in` is the only way to make it.** A spread that produced
    // `hostDomain: undefined` would satisfy `toBeUndefined()` while putting a *present* field on the wire — and a
    // client reading it the way this codebase reads `replayedFrom`/`resyncRequired` (`"key" in response`, which
    // is what the schema's own docblock records) would then see a domain the engine never had.
    //
    // Inventing `localhost` would be worse still, and it is the tempting shortcut: the desktop app's engine
    // really is `localhost`, so the substitution would be *right* there and a lie everywhere else — and a client
    // could not tell the two apart.
    const { transport } = harness();
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
    })) as Record<string, unknown>;
    expect("hostDomain" in response).toBe(false);
  });

  it("uses the injected session-id factory", async () => {
    const { transport } = harness({ sessionId: () => "session-fixed" });
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
    })) as { sessionId: string };
    expect(response.sessionId).toBe("session-fixed");
  });

  it("refuses a request before the handshake, and closes the session", async () => {
    // This replaced "refuses a missing or wrong token and closes the session". The rule it asserts is the
    // same one — work item 5's *"Auth failure returns UNAUTHORIZED and closes. Never fall back to
    // unauthenticated"* — but the **trigger** is now the rule itself rather than a credential: the first
    // request must be `hello`, and a session that skipped the handshake has claimed nothing to revoke.
    //
    // Driven across two namespaces, because an implementation could exempt one and the exemption would be
    // exactly the hole this case exists to close. The payload is `{}` on purpose: the decorator refuses
    // *before* the registry ever sees the command, so a valid payload would prove nothing extra.
    for (const cmd of [READ_COMMAND, WRITE_COMMAND]) {
      const { transport } = harness();
      await expect(
        transport.request(cmd, {}),
        `${cmd} before the handshake must be refused`,
      ).rejects.toMatchObject({ code: "UNAUTHORIZED" });
      // Closed, per work item 5.
      await expect(transport.request(READ_COMMAND, {})).rejects.toBeInstanceOf(EngineError);
    }
  });

  it("ACCEPTS a hello carrying a `token`, because the field is no longer read", async () => {
    // This replaced "treats a non-string token as a malformed message, not a wrong credential", and it
    // asserts the **opposite** of that case. `token: 42` used to be a `BAD_REQUEST`, because the field was
    // part of `HelloRequestSchema` and `42` is not a string. It is not part of the schema any more, so a
    // `z.object` strips it whatever its type and the handshake succeeds.
    //
    // Both values are worth driving: `42` is the shape that used to fail schema validation, and a plausible
    // string is what a real client built before 2026-09-25 still sends.
    //
    // **This is not a hole, and the reason is the shape of the change.** The old case's security claim was
    // that a bad credential is never *accepted*; the claim now is that there is no credential to be bad —
    // the boundary moved to the loopback bind and the `Origin` guard. `protocol.conformance.ts` pins the
    // same property from the conformance side, so it holds for every transport rather than for this one.
    for (const token of [42, "a-stale-token-from-an-older-client"]) {
      const { transport } = harness();
      await expect(
        transport.request(HELLO_ACTION, { ...VALID_HELLO, token }),
        `token ${JSON.stringify(token)} must be ignored, not refused`,
      ).resolves.toBeDefined();
    }
  });

  it("refuses a malformed hello without closing, so the client can correct it", async () => {
    // A serialisation bug is not an identity failure — nothing was claimed, so there is nothing to
    // revoke. Closing here would turn a fixable client bug into a reconnect loop.
    const { transport } = harness();
    await expect(transport.request(HELLO_ACTION, { protocolVersion: PROTOCOL_VERSION })).rejects.toMatchObject(
      { code: "BAD_REQUEST" },
    );
    // Still usable:
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO }),
    ).resolves.toBeDefined();
  });

  it("refuses a major protocol mismatch with UNSUPPORTED", async () => {
    const { transport } = harness();
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, protocolVersion: "2.0.0" }),
    ).rejects.toMatchObject({ code: "UNSUPPORTED" });
  });

  it("ACCEPTS a minor protocol mismatch, because the companion extension depends on it", async () => {
    // `checkVersionCompatibility`: "A client with an older protocolVersion than the engine is the
    // *expected* state for weeks after every release (the companion extension case) — never refuse on
    // minor alone." A naive `!==` here would lock out every released extension build, which is the
    // external consumer `plan/README.md:150` freezes four commands for.
    const { transport } = harness({ protocolVersion: "1.4.0" });
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, protocolVersion: "1.2.0" }),
    ).resolves.toBeDefined();
  });

  it("refuses a second hello on the same session", async () => {
    // Re-running the handshake would let a peer swap its scopes mid-session, which turns the boundary
    // into a suggestion.
    const { transport } = await authenticated();
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO }),
    ).rejects.toMatchObject({ code: "CONFLICT" });
  });

  it("reports the failure to onAuthFailure", async () => {
    // The trigger is a request that is not `hello`. The "wrong credential" version of this went with the
    // token; this is the one identity failure the decorator has left to report.
    const { transport, failures } = harness();
    await expect(transport.request(READ_COMMAND, {})).rejects.toThrow();
    expect(failures).toHaveLength(1);
    expect(failures[0]!.code).toBe("UNAUTHORIZED");
  });
});

/**
 * Work item 4's audit trail, and the `ctx.session` half of it.
 *
 * The decorator is the only thing that knows **who** connected — the version check and the `hello`
 * payload both live in it — so it is the only thing that can report the identity. *Where*
 * that identity is recorded is deliberately not its business, which is why these cases assert the
 * observer and never reach for a command context: the decorator observes, the caller records.
 *
 * The negative cases are the ones that matter. An identity reported on a **rejected** handshake would
 * put a client that never authenticated into the audit trail, which is worse than no trail at all.
 */
describe("the session identity", () => {
  function collect(): {
    seen: SessionIdentity[];
    opts: Partial<Parameters<typeof createAuthenticatedTransport>[1]>;
  } {
    const seen: SessionIdentity[] = [];
    return { seen, opts: { onSession: (identity) => seen.push(identity) } };
  }

  it("reports the session id, client name and client version once the handshake succeeds", async () => {
    const { seen, opts } = collect();
    const { transport } = harness(opts);

    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
    })) as { sessionId: string };

    expect(seen).toHaveLength(1);
    // The id must be the one the client was actually told. Reporting a different one — or a fresh
    // UUID — would produce an audit trail that cannot be joined to anything.
    expect(seen[0]).toMatchObject({
      sessionId: response.sessionId,
      clientName: VALID_HELLO.clientName,
      clientVersion: VALID_HELLO.clientVersion,
    });
    // The **authority** rides along with the identity, defaulting to the shell's full set when the
    // caller supplies no `scopesFor` — deliberately the same default the scope gate itself uses, so a
    // handler can never be shown a narrower authority than the one its commands were checked against.
    expect(seen[0].scopes).toEqual(SHELL_SCOPES);
  });

  it("reports the scopes the gate will actually enforce when scopesFor narrows them", async () => {
    // The reporter and the gate read the same value, so this is what makes a handler's
    // `callerHasScope()` answer match the check that let the command through in the first place.
    const { seen, opts } = collect();
    const { transport } = harness({ ...opts, scopesFor: () => COMPANION_SCOPES });

    await transport.request(HELLO_ACTION, { ...VALID_HELLO });

    expect(seen).toHaveLength(1);
    expect(seen[0].scopes).toEqual(COMPANION_SCOPES);
  });

  it("reports nothing when the session is refused", async () => {
    // Replaced "reports nothing when the token is wrong". The property is the one that matters for an
    // audit trail — a client that never established a session must not appear in it — but the way to
    // produce a refusal is now a request that is not `hello`.
    const { seen, opts } = collect();
    const { transport } = harness(opts);
    await expect(transport.request(READ_COMMAND, {})).rejects.toMatchObject({ code: "UNAUTHORIZED" });
    expect(seen).toEqual([]);
  });

  it("reports nothing on a major protocol mismatch", async () => {
    const { seen, opts } = collect();
    const { transport } = harness(opts);
    const [major, minor, patch] = PROTOCOL_VERSION.split(".");
    await expect(
      transport.request(HELLO_ACTION, {
        ...VALID_HELLO,
        protocolVersion: `${Number(major) + 1}.${minor}.${patch}`,
      }),
    ).rejects.toMatchObject({ code: "UNSUPPORTED" });
    expect(seen).toEqual([]);
  });

  it("reports nothing for a malformed hello", async () => {
    const { seen, opts } = collect();
    const { transport } = harness(opts);
    // Malformed, not unauthorised: nothing was claimed, so there is nothing to attribute. It is also
    // the one refusal that does *not* close the session, which makes it the easiest to fire by
    // accident — hence a case rather than a comment.
    await expect(
      transport.request(HELLO_ACTION, { protocolVersion: PROTOCOL_VERSION }),
    ).rejects.toMatchObject({ code: "BAD_REQUEST" });
    expect(seen).toEqual([]);
  });

  it("reports nothing for a command refused before the handshake", async () => {
    const { seen, opts } = collect();
    const { transport } = harness(opts);
    await expect(transport.request(READ_COMMAND, {})).rejects.toMatchObject({ code: "UNAUTHORIZED" });
    expect(seen).toEqual([]);
  });

  it("reports at most once, because a second hello is refused", async () => {
    const { seen, opts } = collect();
    const { transport } = harness(opts);
    await transport.request(HELLO_ACTION, { ...VALID_HELLO });
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO }),
    ).rejects.toMatchObject({ code: "CONFLICT" });
    expect(seen).toHaveLength(1);
  });

  it("does not write the identity into the command context — that is the caller's job", async () => {
    // The other half of the split, asserted end to end: the decorator reports, so a caller that never
    // wires `onSession` to its context leaves handlers with no identity — and that must be *visible*
    // rather than silently defaulted to something that looks like a session.
    const bus = new EngineEventBus();
    const ctx: CommandContext = { bus };
    const registry = new CommandRegistry();
    registry.register(READ_COMMAND, (_params, inner) => ({ session: inner.session ?? null }));
    const transport = createAuthenticatedTransport(
      createInProcessTransport(registry, { bus, ctx }),
      { engineVersion: "0.3.3" },
    );

    await transport.request(HELLO_ACTION, { ...VALID_HELLO });

    expect(ctx.session).toBeUndefined();
    await expect(transport.request(READ_COMMAND, {})).resolves.toEqual({ session: null });
  });

  it("does fill the context when the caller wires the observer to it", async () => {
    // The wiring a boundary transport must do — the same two lines `ws.ts` and `socket.ts` contain.
    const bus = new EngineEventBus();
    const ctx: CommandContext = { bus };
    const registry = new CommandRegistry();
    registry.register(READ_COMMAND, (_params, inner) => ({ session: inner.session ?? null }));
    const transport = createAuthenticatedTransport(
      createInProcessTransport(registry, { bus, ctx }),
      {
        engineVersion: "0.3.3",
        onSession: (identity) => {
          ctx.session = identity;
        },
      },
    );

    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
    })) as { sessionId: string };

    await expect(transport.request(READ_COMMAND, {})).resolves.toMatchObject({
      session: {
        sessionId: response.sessionId,
        clientName: VALID_HELLO.clientName,
        clientVersion: VALID_HELLO.clientVersion,
      },
    });
  });
});

describe("no unauthenticated fallback path", () => {
  it("refuses every command before the handshake, whatever its scope", async () => {
    // The claim `plan/05` work item 5 makes is a *universal* one — "Never fall back to
    // unauthenticated" — so it is swept across the protocol rather than spot-checked. A per-command
    // exception would be exactly the hole this work item exists to close.
    //
    // A fresh session per command, because the first refusal **closes** the session: reusing one
    // would make every iteration after the first assert the closed-session path instead, and the
    // sweep would silently stop testing what it claims to.
    const commands = Object.keys(COMMAND_FIXTURES);
    expect(commands.length).toBeGreaterThan(50);

    for (const cmd of commands) {
      const { transport } = harness();
      await expect(transport.request(cmd, {}), `${cmd} must be refused pre-auth`).rejects.toMatchObject({
        code: "UNAUTHORIZED",
      });
    }
  });

  it("closes the session on the first non-handshake request", async () => {
    const { transport } = harness();
    await expect(transport.request(READ_COMMAND, {})).rejects.toMatchObject({ code: "UNAUTHORIZED" });
    // Once closed, the request is delegated, so the inner transport's "closed" error applies rather
    // than a second UNAUTHORIZED — one state, one error code.
    await expect(transport.request(READ_COMMAND, {})).rejects.toBeInstanceOf(EngineError);
  });

  it("throws rather than silently no-ops on subscribe before the handshake", async () => {
    // A subscription that succeeds and never fires is the failure mode `plan/05` singles out.
    const { transport } = harness();
    expect(() => transport.subscribe(["event.server.error"], () => {})).toThrow(/hello/i);
  });
});

describe("scope enforcement", () => {
  it("allows what the session's scopes cover", async () => {
    const { transport } = await authenticated();
    await expect(transport.request(READ_COMMAND, validPayload(READ_COMMAND))).resolves.toEqual({
      reached: READ_COMMAND,
    });
  });

  it("refuses with FORBIDDEN and names the required scope", async () => {
    const { transport } = await authenticated({ scopesFor: () => READ_ONLY_SCOPES });
    await expect(transport.request(WRITE_COMMAND, {})).rejects.toMatchObject({
      code: "FORBIDDEN",
      details: { required: "write" },
    });
  });

  it("decides on scope before the payload is validated", async () => {
    // The gate runs ahead of `registry.invoke()`, so a forbidden command is refused identically
    // whether or not its payload is well-formed. That matters: if the order were reversed, the
    // *shape* of the error would tell an unauthorised caller whether a command exists and what its
    // schema wants — a free oracle for mapping the protocol from a session that should see nothing.
    const { transport } = await authenticated({ scopesFor: () => READ_ONLY_SCOPES });
    await expect(transport.request(WRITE_COMMAND, { garbage: true })).rejects.toMatchObject({
      code: "FORBIDDEN",
    });
    await expect(transport.request(WRITE_COMMAND, validPayload(WRITE_COMMAND))).rejects.toMatchObject({
      code: "FORBIDDEN",
    });
  });

  it("does NOT close the session on FORBIDDEN", async () => {
    // A scope failure is not an identity failure: the peer proved who it is and asked for something
    // out of scope. Closing would make a mis-scoped client look like a network fault.
    const { transport } = await authenticated({ scopesFor: () => READ_ONLY_SCOPES });
    await expect(transport.request(ADMIN_COMMAND, {})).rejects.toMatchObject({ code: "FORBIDDEN" });
    await expect(transport.request(READ_COMMAND, validPayload(READ_COMMAND))).resolves.toEqual({
      reached: READ_COMMAND,
    });
  });

  it("honours scopesFor per session", async () => {
    const readOnly = await authenticated({ scopesFor: () => READ_ONLY_SCOPES });
    await expect(readOnly.transport.request(EXECUTE_COMMAND, {})).rejects.toMatchObject({
      code: "FORBIDDEN",
    });

    const companion = await authenticated({ scopesFor: () => COMPANION_SCOPES });
    await expect(
      companion.transport.request(WRITE_COMMAND, validPayload(WRITE_COMMAND)),
    ).resolves.toEqual({ reached: WRITE_COMMAND });
    await expect(companion.transport.request(EXECUTE_COMMAND, {})).rejects.toMatchObject({
      code: "FORBIDDEN",
    });
  });

  it("passes an unknown command through so the registry can answer UNKNOWN_COMMAND", async () => {
    // Better than FORBIDDEN, which would imply the command exists. Not fail-open: an unknown command
    // executes nothing, because the registry has no handler for it either.
    const { transport } = await authenticated();
    await expect(transport.request("not.a.command", {})).rejects.toMatchObject({
      code: "UNKNOWN_COMMAND",
    });
  });
});

describe("the decorator behaves like a transport", () => {
  it("forwards kind", async () => {
    const { transport, inner } = await authenticated();
    expect(transport.kind).toBe(inner.kind);
  });

  it("delivers events after the handshake", async () => {
    const { transport, bus } = await authenticated();
    const seen: EngineEvent[] = [];
    transport.subscribe(["event.server.error"], (e) => seen.push(e));
    bus.emitTyped("server.error", "after auth");
    // `in-process` delivers synchronously; this asserts the decorator does not break that.
    expect(seen).toHaveLength(1);
    expect(seen[0]!.payload).toBe("after auth");
  });

  it("closes idempotently and delegates afterwards", async () => {
    const { transport } = await authenticated();
    await transport.close();
    await expect(transport.close()).resolves.toBeUndefined();
    await expect(transport.request(READ_COMMAND, {})).rejects.toBeInstanceOf(EngineError);
    expect(() => transport.subscribe(["event.server.error"], () => {})).toThrow(/closed/i);
  });
});

// ── replay from lastSeq (work item 3) ────────────────────────────────────────

/**
 * The decorator's half of replay: it owns the *decision* (`replayedFrom` / `resyncRequired`) and the
 * *delivery* of the backlog, because `hello` is the only place a `lastSeq` arrives.
 *
 * The behaviour through a transport is covered by the conformance suite's `replay` cases. What is
 * here instead is the edges the suite cannot reach: an engine with no log at all, a window that moves
 * between the handshake and the subscribe, and what a callback that throws mid-backlog does to the
 * live subscription.
 */
describe("replay from lastSeq", () => {
  const ERROR = "event.server.error";
  const CHUNK = "event.log.chunk";

  /** Attach retention for a name and leave — the blip retention exists for. */
  const openTheWindow = (log: EventLog, names: string[] = [ERROR]): void => {
    log.subscribe(names, () => {})();
  };

  it("answers resyncRequired when it has no log to replay from", async () => {
    // Not a degenerate case: a decorator wrapped without a log has no history, and the honest answer
    // is "re-fetch state". Answering `replayedFrom` here would be the silent-staleness bug — the
    // client waits for events that were never buffered.
    const { transport } = harness();
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
      lastSeq: 7,
    })) as Record<string, unknown>;

    expect(response.resyncRequired).toBe(true);
    expect(response).not.toHaveProperty("replayedFrom");
  });

  it("advertises events.replay only when it can actually replay", async () => {
    // Derived from the wiring rather than left to the caller's list. A client that sees no
    // `events.replay` will not send `lastSeq`, so an engine with a log and without the capability
    // would silently never replay — a failure that looks like "replay is broken".
    const without = harness();
    const with_ = harness({}, { log: {} });

    const ask = async (t: Transport) =>
      ((await t.request(HELLO_ACTION, { ...VALID_HELLO })) as {
        capabilities: string[];
      }).capabilities;

    expect(await ask(without.transport)).not.toContain("events.replay");
    expect(await ask(with_.transport)).toContain("events.replay");
  });

  it("does not add events.replay twice when the caller already listed it", async () => {
    const { transport } = harness({ capabilities: ["events.replay"] }, { log: {} });
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
    })) as { capabilities: string[] };
    expect(response.capabilities).toEqual(["events.replay"]);
  });

  it("replays only the names the client subscribed to", async () => {
    const { transport, bus, log } = harness({}, { log: {} });
    openTheWindow(log!, [ERROR, CHUNK]);
    bus.emitTyped("server.error", "an-error");
    bus.emitTyped("log.chunk", { logId: "l1", chunk: "a-chunk", done: false });

    await transport.request(HELLO_ACTION, { ...VALID_HELLO, lastSeq: 0 });

    const errors: EngineEvent[] = [];
    transport.subscribe([ERROR], (e) => errors.push(e));

    // Only the error. Replaying the chunk too would push a client events it never asked for, and for
    // a chatty `log.chunk` stream that is a burst of unrelated output on every reconnect.
    expect(errors.map((e) => e.payload)).toEqual(["an-error"]);
    // The engine's seqs, not a client-side recount: seq 2 belongs to the chunk the client did not
    // subscribe to. A gap here is correct and is the documented cost of an engine-scoped counter.
    expect(errors.map((e) => e.seq)).toEqual([1]);
  });

  it("throws CONFLICT when the window moves past the resume point before the client subscribes", async () => {
    // The handshake answers a question about the buffer *as it is then*. This is what happens when the
    // answer stops being true: `maxEvents: 2` so the window can be moved by hand instead of by 1,000
    // events.
    const { transport, bus, log } = harness({}, { log: { maxEvents: 2 } });
    openTheWindow(log!);
    bus.emitTyped("server.error", "one");

    // The promise is made while seq 1 is still retained.
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
      lastSeq: 0,
    })) as Record<string, unknown>;
    expect(response.replayedFrom).toBe(0);

    // …and then the window moves past it.
    bus.emitTyped("server.error", "two");
    bus.emitTyped("server.error", "three");
    expect(log!.oldestSeq, "the precondition: seq 1 must have been evicted").toBe(2);

    let thrown: unknown;
    try {
      transport.subscribe([ERROR], () => {});
    } catch (err) {
      thrown = err;
    }

    // Thrown rather than answered with a partial replay. A partial replay leaves the client believing
    // it is current, which is the permanently-wrong-UI failure `plan/05` calls the high risk; and
    // `CONFLICT` rather than `ENGINE_ERROR` because the client's recovery is specific and already
    // specified — reconnect without `lastSeq`, then re-fetch state.
    expect(thrown).toBeInstanceOf(EngineError);
    expect((thrown as EngineError).code).toBe("CONFLICT");
    expect((thrown as EngineError).message).toMatch(/can no longer be served/);
  });

  it("leaves no subscription behind when that CONFLICT is thrown", async () => {
    // Why the backlog is computed *before* anything is attached. A caller cannot clean up what it was
    // never given, so a throw after `inner.subscribe()` would leak a live subscription on every
    // refusal — invisible until the same session retried and got every event twice.
    const { transport, bus, log } = harness({}, { log: { maxEvents: 2 } });
    openTheWindow(log!);
    bus.emitTyped("server.error", "one");
    await transport.request(HELLO_ACTION, { ...VALID_HELLO, lastSeq: 0 });
    bus.emitTyped("server.error", "two");
    bus.emitTyped("server.error", "three");

    expect(() => transport.subscribe([ERROR], () => {})).toThrow();
    expect(log!.subscriberCount(ERROR)).toBe(0);
  });

  it("detaches the live subscription when the callback throws mid-backlog", async () => {
    // The callback is arbitrary caller code. If it throws while the backlog is being delivered, the
    // live subscription attached moments earlier must not survive — the caller never received the
    // unsubscribe function, so it has no way to detach it.
    const { transport, bus, log } = harness({}, { log: {} });
    openTheWindow(log!);
    bus.emitTyped("server.error", "in-the-backlog");
    await transport.request(HELLO_ACTION, { ...VALID_HELLO, lastSeq: 0 });

    const seen: EngineEvent[] = [];
    let first = true;
    expect(() =>
      transport.subscribe([ERROR], (e) => {
        seen.push(e);
        if (first) {
          first = false;
          throw new Error("the caller's callback blew up");
        }
      }),
    ).toThrow(/blew up/);

    expect(seen).toHaveLength(1);
    expect(log!.subscriberCount(ERROR)).toBe(0);

    bus.emitTyped("server.error", "live");
    expect(seen, "the live subscription outlived the call that threw").toHaveLength(1);
  });

  it("subscribes live even when there is nothing to replay", async () => {
    // `replayFrom` can legitimately return an empty backlog — the client is already current. That must
    // not be confused with "no subscription", which is the shape of the bug where a reconnecting client
    // is caught up and then never hears anything again.
    const { transport, bus, log } = harness({}, { log: {} });
    openTheWindow(log!);
    bus.emitTyped("server.error", "one");
    await transport.request(HELLO_ACTION, { ...VALID_HELLO, lastSeq: 1 });

    const seen: EngineEvent[] = [];
    transport.subscribe([ERROR], (e) => seen.push(e));
    expect(seen).toHaveLength(0);

    bus.emitTyped("server.error", "two");
    expect(seen.map((e) => e.payload)).toEqual(["two"]);
  });
});
