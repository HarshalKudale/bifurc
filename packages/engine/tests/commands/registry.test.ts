/**
 * The authority predicate — `callerHasScope()` and `mayAffectUnnamedEntities()`.
 *
 * These two decide whether a command may reach **beyond the entities its caller named**, which is the
 * mechanism that keeps a lower-trust client additive (P4 item 1). The cases worth pinning are the
 * *asymmetries*, not the happy path: a caller with **no session** must be treated as the engine itself
 * and granted everything, while a caller **with** a session that reported no scopes must be refused.
 * Those two look almost identical in a diff and behave oppositely, so they sit side by side here.
 *
 * `mayAffectUnnamedEntities` is asserted to be exactly `admin` rather than "some write-ish scope" — a
 * test that only checked the companion's `{read, write}` case would still pass if the predicate were
 * relaxed to `write`, which is precisely the regression that would hand a browser page the ability to
 * disable the user's mocks.
 */

import { describe, expect, it } from "vitest";
import {
  callerHasScope,
  mayAffectUnnamedEntities,
  type CommandContext,
  type Scope,
  type SessionIdentity,
} from "../../src/commands/registry";
import {
  COMPANION_SCOPES,
  READ_ONLY_SCOPES,
  SCOPES,
  SHELL_SCOPES,
} from "../../src/transport/auth/scopes";

/** What `in-process`, `stdio` and every legacy `ipcMain.handle` adapter pass: a bus, no session. */
const engineCtx = (): CommandContext => ({ bus: {} as CommandContext["bus"] });

const sessionCtx = (scopes?: ReadonlySet<Scope>): CommandContext => {
  const session: SessionIdentity = {
    sessionId: "s-1",
    clientName: "client",
    clientVersion: "0.0.0",
    ...(scopes ? { scopes } : {}),
  };
  return { bus: {} as CommandContext["bus"], session };
};

describe("callerHasScope()", () => {
  it("grants every scope to a caller with no session, because that caller is the engine itself", () => {
    for (const scope of SCOPES) {
      expect(callerHasScope(engineCtx(), scope)).toBe(true);
    }
  });

  it("answers from the session's own scopes when there is one", () => {
    expect(callerHasScope(sessionCtx(SHELL_SCOPES), "admin")).toBe(true);
    expect(callerHasScope(sessionCtx(COMPANION_SCOPES), "write")).toBe(true);
    expect(callerHasScope(sessionCtx(COMPANION_SCOPES), "admin")).toBe(false);
    expect(callerHasScope(sessionCtx(READ_ONLY_SCOPES), "read")).toBe(true);
    expect(callerHasScope(sessionCtx(READ_ONLY_SCOPES), "write")).toBe(false);
  });

  it("does not read a hierarchy into the set — admin alone does not imply read", () => {
    const adminOnly = new Set<Scope>(["admin"]);
    expect(callerHasScope(sessionCtx(adminOnly), "admin")).toBe(true);
    expect(callerHasScope(sessionCtx(adminOnly), "read")).toBe(false);
  });

  it("refuses a session that reported no scopes at all, rather than assuming one", () => {
    for (const scope of SCOPES) {
      expect(callerHasScope(sessionCtx(), scope)).toBe(false);
    }
  });
});

describe("mayAffectUnnamedEntities()", () => {
  it("allows the engine's own callers, which is what keeps the shell's behaviour unchanged", () => {
    expect(mayAffectUnnamedEntities(engineCtx())).toBe(true);
  });

  it("allows a shell session, which holds admin", () => {
    expect(mayAffectUnnamedEntities(sessionCtx(SHELL_SCOPES))).toBe(true);
  });

  it("refuses the companion's {read, write} session — the additive-only guarantee", () => {
    expect(mayAffectUnnamedEntities(sessionCtx(COMPANION_SCOPES))).toBe(false);
  });

  it("refuses a session holding read, write and execute but not admin", () => {
    expect(
      mayAffectUnnamedEntities(sessionCtx(new Set<Scope>(["read", "write", "execute"]))),
    ).toBe(false);
  });

  it("refuses a read-only session", () => {
    expect(mayAffectUnnamedEntities(sessionCtx(READ_ONLY_SCOPES))).toBe(false);
  });

  it("refuses a session with no reported scopes", () => {
    expect(mayAffectUnnamedEntities(sessionCtx())).toBe(false);
  });
});
