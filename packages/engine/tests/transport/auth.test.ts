/**
 * P4 work item 4 — authentication and authorisation, tested without a socket.
 *
 * This is the payoff of building auth as a **decorator**: every case below runs over `in-process`, so
 * the handshake, the token check and the scope gate are all exercised before `ws` exists. `ws` stays
 * behind its hard gate — "an unauthenticated remote RPC surface is a remote code execution hole" —
 * and this file is what makes that gate satisfiable rather than merely blocking.
 *
 * ## What is deliberately asserted as a *property* rather than a spot check
 *
 * The scope table is 93 entries and the interesting failures are not "is `tls.generate` admin?" but
 * "can a session holding only `read` reach anything that writes?", and "can the companion extension
 * reach anything destructive?". Those are sweeps over the whole table, so a future re-scoping cannot
 * quietly open a hole that a spot check would not notice.
 */
import { afterEach, describe, expect, it } from "vitest";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
  ENGINE_TOKEN_ENV_VAR,
  ensureEngineToken,
  generateToken,
  HELLO_ACTION,
  READ_ONLY_SCOPES,
  readEngineToken,
  requiredScopeFor,
  SCOPE_BY_COMMAND,
  SCOPES,
  SHELL_SCOPES,
  verifyToken,
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

const tempDirs: string[] = [];
afterEach(async () => {
  for (const dir of tempDirs.splice(0)) await rm(dir, { recursive: true, force: true });
});

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "bifurc-auth-"));
  tempDirs.push(dir);
  return dir;
}

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

// ── the token ────────────────────────────────────────────────────────────────

describe("token", () => {
  it("generates a fresh 256-bit base64url token each time", () => {
    const a = generateToken();
    const b = generateToken();
    expect(a).not.toBe(b);
    // 32 bytes base64url = 43 chars, no padding — safe in a URL, a JSON string, an env var and a file.
    expect(a).toHaveLength(43);
    expect(a).toMatch(/^[A-Za-z0-9_-]+$/);
  });

  it("accepts the matching token and rejects a different one", () => {
    const token = generateToken();
    expect(verifyToken(token, token)).toBe(true);
    expect(verifyToken(token, generateToken())).toBe(false);
  });

  it("rejects a token that is only a correct prefix", () => {
    // Guards the comparison against a `startsWith`/`slice` rewrite, which would accept any extension
    // of the real token — a far weaker secret than it looks.
    const token = generateToken();
    expect(verifyToken(token, token.slice(0, 20))).toBe(false);
    expect(verifyToken(token, token + "x")).toBe(false);
  });

  it("rejects non-strings without throwing", () => {
    // `token` arrives from a client. A TypeError escaping here would crash the session instead of
    // refusing it, and `timingSafeEqual` throws on a length mismatch, so both paths matter.
    const token = generateToken();
    for (const bad of [undefined, null, 42, {}, [], true, Buffer.from(token)]) {
      expect(verifyToken(token, bad), `${String(bad)} must be refused`).toBe(false);
    }
    expect(verifyToken(token, "")).toBe(false);
  });

  it("writes the token where a client can read it", async () => {
    const dir = await tempDir();
    const { token, path, source } = await ensureEngineToken(dir, {});
    expect(source).toBe("generated");
    expect(path).toBeDefined();
    expect(await readEngineToken(dir)).toBe(token);
  });

  it("rotates on every call, so a token does not survive a restart", async () => {
    // The property that makes this a *session* secret. A reused token stays valid against a
    // freshly-started engine, so anything that ever read it keeps its access forever.
    const dir = await tempDir();
    const first = await ensureEngineToken(dir, {});
    const second = await ensureEngineToken(dir, {});
    expect(second.token).not.toBe(first.token);
    expect(await readEngineToken(dir)).toBe(second.token);
    expect(verifyToken(second.token, first.token)).toBe(false);
  });

  it("writes the token file 0600 where the platform has permission bits", async () => {
    const dir = await tempDir();
    const { path } = await ensureEngineToken(dir, {});
    if (process.platform === "win32") {
      // Stated rather than asserted: Windows has no POSIX permission bits and Node maps `mode` to the
      // read-only attribute only, so the token is protected by the data directory's ACL and nothing
      // else. Same class of gap as the named-pipe transport; belongs in P9/P12.
      expect(path).toBeDefined();
      return;
    }
    const mode = (await stat(path!)).mode & 0o777;
    expect(mode.toString(8)).toBe("600");
  });

  it("takes the token from the environment and writes no file", async () => {
    // The Docker / mounted-secret path. A container that mounts its secret should not also have it
    // written into a volume that may be more widely readable.
    const dir = await tempDir();
    const { token, path, source } = await ensureEngineToken(dir, {
      [ENGINE_TOKEN_ENV_VAR]: "from-the-environment",
    });
    expect(source).toBe("environment");
    expect(token).toBe("from-the-environment");
    expect(path).toBeUndefined();
    expect(await readEngineToken(dir)).toBeUndefined();
  });

  it("ignores an empty environment variable rather than trusting it", async () => {
    // An empty token is a trivially guessable credential. Falling through to generation is the safe
    // reading of "set but empty".
    const dir = await tempDir();
    const { source, token } = await ensureEngineToken(dir, { [ENGINE_TOKEN_ENV_VAR]: "" });
    expect(source).toBe("generated");
    expect(token).not.toBe("");
  });

  it("treats a missing or blank file as no token", async () => {
    const dir = await tempDir();
    expect(await readEngineToken(dir)).toBeUndefined();
    await writeFile(join(dir, "engine.token"), "   \n", "utf8");
    // A truncated write must not become a credential.
    expect(await readEngineToken(dir)).toBeUndefined();
  });

  it("writes the token as-is, with no trailing newline", async () => {
    // A client reads this file and puts the bytes in `hello`. A newline would make the value differ
    // from the one the engine compares against, and the failure would look like a wrong token.
    const dir = await tempDir();
    const { token } = await ensureEngineToken(dir, {});
    expect(await readFile(join(dir, "engine.token"), "utf8")).toBe(token);
  });
});

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
    token: "the-engine-token",
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
  await h.transport.request(HELLO_ACTION, { ...VALID_HELLO, token: opts.token ?? "the-engine-token" });
  return h;
}

describe("the hello handshake", () => {
  it("resolves with the protocol's response shape", async () => {
    const { transport } = harness();
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
      token: "the-engine-token",
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
      token: "the-engine-token",
    })) as { capabilities: string[] };
    expect(response.capabilities).toEqual(["blob", "events.replay"]);
  });

  it("uses the injected session-id factory", async () => {
    const { transport } = harness({ sessionId: () => "session-fixed" });
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
      token: "the-engine-token",
    })) as { sessionId: string };
    expect(response.sessionId).toBe("session-fixed");
  });

  it("refuses a missing or wrong token and closes the session", async () => {
    for (const token of [undefined, "wrong"]) {
      const { transport } = harness();
      await expect(
        transport.request(HELLO_ACTION, { ...VALID_HELLO, token }),
      ).rejects.toMatchObject({ code: "UNAUTHORIZED" });
      // Closed, per work item 5: "Auth failure returns UNAUTHORIZED and closes."
      await expect(transport.request(READ_COMMAND, {})).rejects.toBeInstanceOf(EngineError);
    }
  });

  it("treats a non-string token as a malformed message, not a wrong credential", async () => {
    // `42` fails `HelloRequestSchema`, so it is `BAD_REQUEST` and the session survives — a
    // well-formed message carrying the wrong secret is an *identity* failure and closes, but a
    // message that is not shaped like a handshake at all claimed nothing to revoke.
    //
    // The security-relevant half is that it is never *accepted*: the schema refuses it here, and
    // `verifyToken` refuses a non-string independently — belt and braces, because a future change
    // could loosen the schema without anyone re-checking the comparison.
    const { transport } = harness();
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, token: 42 }),
    ).rejects.toMatchObject({ code: "BAD_REQUEST" });
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token" }),
    ).resolves.toBeDefined();
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
      transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token" }),
    ).resolves.toBeDefined();
  });

  it("refuses a major protocol mismatch with UNSUPPORTED", async () => {
    const { transport } = harness();
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, protocolVersion: "2.0.0", token: "the-engine-token" }),
    ).rejects.toMatchObject({ code: "UNSUPPORTED" });
  });

  it("ACCEPTS a minor protocol mismatch, because the companion extension depends on it", async () => {
    // `checkVersionCompatibility`: "A client with an older protocolVersion than the engine is the
    // *expected* state for weeks after every release (the companion extension case) — never refuse on
    // minor alone." A naive `!==` here would lock out every released extension build, which is the
    // external consumer `plan/README.md:150` freezes four commands for.
    const { transport } = harness({ protocolVersion: "1.4.0" });
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, protocolVersion: "1.2.0", token: "the-engine-token" }),
    ).resolves.toBeDefined();
  });

  it("refuses a second hello on the same session", async () => {
    // Re-running the handshake would let a peer swap its scopes mid-session, which turns the boundary
    // into a suggestion.
    const { transport } = await authenticated();
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token" }),
    ).rejects.toMatchObject({ code: "CONFLICT" });
  });

  it("reports the failure to onAuthFailure", async () => {
    const { transport, failures } = harness();
    await expect(transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "wrong" })).rejects.toThrow();
    expect(failures).toHaveLength(1);
    expect(failures[0]!.code).toBe("UNAUTHORIZED");
  });
});

/**
 * Work item 4's audit trail, and the `ctx.session` half of it.
 *
 * The decorator is the only thing that knows **who** connected — the token check, the version check and
 * the `hello` payload all live in it — so it is the only thing that can report the identity. *Where*
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
      token: "the-engine-token",
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

    await transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token" });

    expect(seen).toHaveLength(1);
    expect(seen[0].scopes).toEqual(COMPANION_SCOPES);
  });

  it("reports nothing when the token is wrong", async () => {
    const { seen, opts } = collect();
    const { transport } = harness(opts);
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "definitely-not-the-token" }),
    ).rejects.toMatchObject({ code: "UNAUTHORIZED" });
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
        token: "the-engine-token",
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
    await transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token" });
    await expect(
      transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token" }),
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
      { token: "the-engine-token", engineVersion: "0.3.3" },
    );

    await transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token" });

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
        token: "the-engine-token",
        engineVersion: "0.3.3",
        onSession: (identity) => {
          ctx.session = identity;
        },
      },
    );

    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
      token: "the-engine-token",
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
      token: "the-engine-token",
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
      ((await t.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token" })) as {
        capabilities: string[];
      }).capabilities;

    expect(await ask(without.transport)).not.toContain("events.replay");
    expect(await ask(with_.transport)).toContain("events.replay");
  });

  it("does not add events.replay twice when the caller already listed it", async () => {
    const { transport } = harness({ capabilities: ["events.replay"] }, { log: {} });
    const response = (await transport.request(HELLO_ACTION, {
      ...VALID_HELLO,
      token: "the-engine-token",
    })) as { capabilities: string[] };
    expect(response.capabilities).toEqual(["events.replay"]);
  });

  it("replays only the names the client subscribed to", async () => {
    const { transport, bus, log } = harness({}, { log: {} });
    openTheWindow(log!, [ERROR, CHUNK]);
    bus.emitTyped("server.error", "an-error");
    bus.emitTyped("log.chunk", { logId: "l1", chunk: "a-chunk", done: false });

    await transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token", lastSeq: 0 });

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
      token: "the-engine-token",
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
    await transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token", lastSeq: 0 });
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
    await transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token", lastSeq: 0 });

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
    await transport.request(HELLO_ACTION, { ...VALID_HELLO, token: "the-engine-token", lastSeq: 1 });

    const seen: EngineEvent[] = [];
    transport.subscribe([ERROR], (e) => seen.push(e));
    expect(seen).toHaveLength(0);

    bus.emitTyped("server.error", "two");
    expect(seen.map((e) => e.payload)).toEqual(["two"]);
  });
});
