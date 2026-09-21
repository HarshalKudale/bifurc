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

  /**
   * P14 WI 1 (D13) — the workspace-scoped servers accept a `workspaceId`, and the two properties that make
   * it a compatibility story rather than a break.
   *
   * The generic per-command pair above cannot assert this: it checks that *the fixture* parses, and a fixture
   * is one payload. What matters here is a **relationship** — present is accepted, absent is still accepted —
   * which is what "omitted means the active workspace" actually means on the wire. Asserting both directions
   * is also what would catch someone making the field required, the single most likely way this change goes
   * wrong for the desktop.
   */
  describe("workspaceId scoping (P14 D13 / D17)", () => {
    const SCOPED = [
      // D13 — the servers, addressed to a workspace behind one host-wide RPC endpoint.
      "server.status",
      "server.start",
      "server.stop",
      "server.restart",
      "proxy.status",
      "webhook.registerActive",
      "webhook.unregisterActive",
      "webhookServer.start",
      "webhookServer.stop",
      "webhookServer.status",
      // D17 — the blob layer, so a blobId is namespaced to the workspace that staged it.
      "blob.put",
      "blob.stat",
      "blob.read",
      "blob.release",
      // WI 8 — the egress set's blob half: above `BLOB_INLINE_THRESHOLD_BYTES` these stage an artifact
      // that the client reads scoped to its own workspace.
      "runner.exportReport",
      "capture.shareJson",
      "audit.export",
      // `tls.exportCert` is deliberately absent — the CA is one per *machine*, and a `ca-cert.pem` is
      // ~1.2 KB against a 1 MiB threshold, so it cannot reach the blob branch. Recorded in
      // `fileOps/commands.ts`; absent here so a later reader cannot read it as an oversight.
    ] as const;

    it.each(SCOPED)("%s accepts a workspaceId", (action) => {
      const { params } = COMMANDS[action];
      const fixture = COMMAND_FIXTURES[action];
      const withWs = { ...(fixture.valid as object), workspaceId: "ws1" };
      const result = params.safeParse(withWs);
      expect(result.success, result.success ? "" : JSON.stringify(result.error?.issues)).toBe(true);
    });

    it.each(SCOPED)("%s still accepts the payload WITHOUT a workspaceId (the desktop's shape)", (action) => {
      const { params } = COMMANDS[action];
      const fixture = COMMAND_FIXTURES[action];
      const withoutWs = { ...(fixture.valid as Record<string, unknown>) };
      delete withoutWs.workspaceId;
      const result = params.safeParse(withoutWs);
      expect(result.success, result.success ? "" : JSON.stringify(result.error?.issues)).toBe(true);
    });

    it.each(SCOPED)("%s rejects a non-string workspaceId", (action) => {
      const { params } = COMMANDS[action];
      const fixture = COMMAND_FIXTURES[action];
      /**
       * **`{...valid, workspaceId: 42}`, not `{workspaceId: 42}` alone.** The bare form was the original
       * and it is a test that cannot fail for the reason it names: a command with any required field
       * (e.g. `blob.read`'s `blobId`, `audit.export`'s `format`) rejects `{workspaceId: 42}` because
       * those fields are *missing*, so `expect(success).toBe(false)` passed whatever happened to the
       * workspaceId's type. Spreading the valid fixture leaves the type as the only possible error —
       * which is what makes this an assertion about `workspaceId` rather than about required fields.
       */
      const result = params.safeParse({ ...(fixture.valid as object), workspaceId: 42 });
      expect(result.success).toBe(false);
      // And it fails on the *type*, not on something else the spread happened to disturb.
      if (!result.success) {
        expect(result.error.issues.some((i) => i.path.includes("workspaceId"))).toBe(true);
      }
    });

    it("services.discover is deliberately NOT scoped — it reports host-wide services", () => {
      // Recorded as an assertion so the omission is a decision rather than an oversight: the command lists
      // ports this host is listening on, which is a property of the host and not of any workspace.
      expect(COMMANDS["services.discover"].params.safeParse({ workspaceId: "ws1" }).success).toBe(false);
    });
  });
});
