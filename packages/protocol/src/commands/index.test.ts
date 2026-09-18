/**
 * P1 acceptance criterion: "Every command has a Zod schema; a smoke test validates a valid and
 * an invalid payload per command."
 *
 * The fixture table itself lives in `./fixtures.ts` rather than here — P4 work item 3 shares it with
 * the transport conformance suite's every-command matrix, and two copies of a 93-entry table is a
 * guarantee that one of them will be wrong. What stays here is the **coverage invariant** over it,
 * which is the reason the shared table can be trusted at all:
 *
 *   `Object.keys(COMMAND_FIXTURES)` must equal `Object.keys(COMMANDS)`.
 *
 * Adding a command without adding a fixture is therefore a test failure — and because the transport
 * matrix is driven off the same table, that one failure also widens the matrix to the new command.
 * The two cannot drift apart.
 */
import { describe, it, expect } from "vitest";
import { COMMANDS, COMMAND_FIXTURES, type CommandAction } from "./index";

describe("@bifurc/protocol command schemas", () => {
  const actions = Object.keys(COMMANDS) as CommandAction[];

  it("every command in COMMANDS has a fixture pair (no command silently unverified)", () => {
    const fixtureKeys = Object.keys(COMMAND_FIXTURES).sort();
    expect(fixtureKeys).toEqual([...actions].sort());
  });

  for (const action of actions) {
    const { params } = COMMANDS[action];
    const fixture = COMMAND_FIXTURES[action];

    it(`${action} — accepts a valid payload`, () => {
      const result = params.safeParse(fixture.valid);
      expect(result.success, result.success ? "" : JSON.stringify(result.error?.issues)).toBe(true);
    });

    it(`${action} — rejects an invalid payload`, () => {
      const result = params.safeParse(fixture.invalid);
      expect(result.success).toBe(false);
    });
  }
});
