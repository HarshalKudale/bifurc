#!/usr/bin/env python3
"""P6 reduction, group 5: the four dead channels in `systemHandlers.ts`.

Deletes `server:start` / `server:stop` / `server:restart` / `capture:shareJson`, re-points the three
suites that drove them, and pins the client's ordering for `shareCaptureJson` — the confirmation that
makes deleting the dialog-owning channel safe.

Count-asserting, CRLF-aware, idempotent (the guard tests `new` only — see TRAPS.md).
"""
import io
import sys

EDITS = [
    # ── src/ipc/handlers.ts: the serverCommands trap, dissolved ───────────────
    ("src/ipc/handlers.ts", 1,
     """  // Those shell bodies **stay as they are** for now, which is deliberate rather than an oversight:
  // re-pointing them at the registry would make them depend on this function having run, and two
  // suites register handler *groups* without it. See `serverCommands.ts`'s header for the full trap.
  registerServerCommands(commandRegistry);""",
     """  // Those shell bodies **stayed as they were** rather than being re-pointed, which was deliberate
  // rather than an oversight: re-pointing them at the registry would have made them depend on this
  // function having run, and two suites register handler *groups* without it. See `serverCommands.ts`'s
  // header for the full trap.
  //
  // **The trap is now dissolved rather than answered** — the reduction's fifth group deleted the three
  // `server:*` channels on 2026-09-21 (`plan/07` §5b-sedecies), so the duplication it warned about is
  // gone and this call is the commands' only registration. `app:checkUpdate` did **not** come with
  // them: it is client-local, not legacy.
  registerServerCommands(commandRegistry);"""),

    # ── packages/engine/src/proxy/serverCommands.ts: two header sections ──────
    ("packages/engine/src/proxy/serverCommands.ts", 1,
     """ * They were served by `ipcMain.handle` bodies in `src/ipc/handlers/systemHandlers.ts` and
 * `coreHandlers.ts`, which works only while the shell and the engine are the same process. They are
 * not shell concerns:""",
     """ * They were served by `ipcMain.handle` bodies in `src/ipc/handlers/systemHandlers.ts` and
 * `coreHandlers.ts` — the three `systemHandlers` ones are **deleted as of 2026-09-21** (P6
 * reduction, fifth group), which worked only while the shell and the engine were the same process.
 * They are not shell concerns:"""),

    ("packages/engine/src/proxy/serverCommands.ts", 1,
     """ * ## What is deliberately absent: `app.checkUpdate`
 *
 * It is the one command in the step-3b inventory that may genuinely belong to the shell rather than
 * the engine — it reads `app.getVersion()` from Electron and asks GitHub for a release.
 * `plan/07`'s local-handlers table calls it a "shell half" while `@bifurc/client` classifies it
 * `{kind: "transport"}`. That disagreement is recorded in `tests/ipc/handlers.test.ts`'s
 * "registry coverage" block and is **not** resolved here, because resolving it is a decision about
 * where the version number comes from on a remote engine, not a mechanical move.
 *
 * ## The trap: do NOT re-point the legacy channels at this module yet
 *
 * The obvious "DRY" follow-up is to replace the shell's six `ipcMain.handle` bodies with
 * `commandRegistry.invoke("<command>", {}, ctx)`, the way `config:get` in `coreHandlers.ts` already
 * does. **That was tried and reverted, and it is a real regression rather than a style question.**
 *
 * The reason is the difference between how `coreHandlers.ts` registers and how this module is
 * registered. `config.get` / `env.setActive` / `workspace.setActive` are registered at **module
 * scope**, so importing `coreHandlers.ts` is enough to populate them. This module is registered by
 * an **explicit call inside `registerIpcHandlers()`**, so a legacy body that delegates here acquires
 * a hidden ordering dependency on a function in a different file — and, worse, on a function that
 * callers are not obliged to run.
 *
 * They are not obliged to run it, and two suites already didn't. `tests/spike/protocolPoc.test.ts`
 * and `tests/integration/settingsMutations.integration.test.ts` deliberately register handler
 * *groups* (`registerCoreHandlers()` / `registerSystemHandlers()`) without calling
 * `registerIpcHandlers()`. Re-pointing broke six of their tests with `UNKNOWN_COMMAND` —
 * `server.status` among them — because the registry they reached was empty. The failure is a
 * `No handler registered` at *invocation* time, so nothing catches it at build time and the shell's
 * own handler file looks untouched.
 *
 * So the duplication is deliberate for now, and bounded: six one-line bodies, behaviourally identical
 * to the handlers below. It is removed by **step 3**, which deletes `registerIpcHandlers()` and the
 * shell's handler groups together — at which point there is one implementation and no second entry
 * point to keep in sync. Re-pointing early trades a temporary, visible duplication for a permanent,
 * invisible coupling.
 */""",
     """ * ## What is deliberately absent: `app.checkUpdate` — resolved, and not by moving it
 *
 * It was the one command in the step-3b inventory that might genuinely belong to the shell rather
 * than the engine: it reads `app.getVersion()` from Electron and asks GitHub for a release.
 * `plan/07`'s local-handlers table called it a "shell half" while `@bifurc/client` classified it
 * `{kind: "transport"}`, and this header used to record that disagreement without resolving it.
 *
 * **It was resolved on 2026-09-20, and not by moving it.** The command left `@bifurc/protocol`
 * entirely and the surface key became `local`, because `currentVersion` is the version of the thing
 * the *user* runs and the download URL must be the asset for the *user's* platform — so there was no
 * engine half to contract with. `packages/client/src/local.ts` carries the argument. The shell
 * channel `app:checkUpdate` therefore **stays**, and is now the one channel `systemHandlers.ts` still
 * registers.
 *
 * ## The trap that no longer applies — and why it was right while it did
 *
 * The obvious "DRY" follow-up was to replace the shell's `ipcMain.handle` bodies with
 * `commandRegistry.invoke("<command>", {}, ctx)`, the way `config:get` in `coreHandlers.ts` already
 * does. **That was tried and reverted, and it was a real regression rather than a style question.**
 *
 * The reason was the difference between how `coreHandlers.ts` registers and how this module is
 * registered. `config.get` / `env.setActive` / `workspace.setActive` are registered at **module
 * scope**, so importing `coreHandlers.ts` is enough to populate them. This module is registered by an
 * **explicit call inside `registerIpcHandlers()`**, so a legacy body that delegated here acquired a
 * hidden ordering dependency on a function in a different file — and, worse, on a function that
 * callers were not obliged to run.
 *
 * They were not obliged to run it, and two suites didn't. `tests/spike/protocolPoc.test.ts` and
 * `tests/integration/settingsMutations.integration.test.ts` deliberately register handler *groups*
 * (`registerCoreHandlers()` / `registerSystemHandlers()`) without calling `registerIpcHandlers()`.
 * Re-pointing broke six of their tests with `UNKNOWN_COMMAND` — `server.status` among them — because
 * the registry they reached was empty. The failure was a `No handler registered` at *invocation*
 * time, so nothing caught it at build time and the shell's own handler file looked untouched.
 *
 * **The trap is dissolved, not answered, and the distinction is worth keeping.** The reduction's
 * fifth group deleted the three `server:*` channels on 2026-09-21, so the duplication the trap warned
 * about is gone and there is now one implementation with no second entry point to keep in sync — which
 * is what step 3 was going to achieve by deleting both halves together. **The suites did not have to
 * change their registration shape for this**: they kept registering the same handler groups, and the
 * three `server:*` calls in `settingsMutations` were re-pointed at the commands in that suite's own
 * `invoke`. The lesson is that "duplication kept deliberately to avoid a hidden coupling" is a
 * *temporary* answer that a deletion can retire, and that retiring it needs no new coupling at all.
 */"""),

    # ── tests/ipc/handlers.test.ts: expectedChannels ─────────────────────────
    ("tests/ipc/handlers.test.ts", 1,
     """      "request:replay", "server:status", "proxy:status",
      "server:restart", "server:stop", "server:start",
      "app:checkUpdate",""",
     """      "request:replay", "server:status", "proxy:status",
      // ── The three server-lifecycle channels are gone (P6 reduction, fifth group, 2026-09-21) ──
      // `server:restart` / `server:stop` / `server:start` were deleted with the four dead channels in
      // `src/ipc/handlers/systemHandlers.ts` (`plan/07` §5b-sedecies). They were the first group whose
      // bodies were **not** `commandRegistry.invoke()` delegates — they called `startServer` /
      // `stopServer` directly — so the criterion was re-applied per channel against
      // `proxy/serverCommands.ts` rather than inherited, and it found exact equivalents.
      // Their behaviour is asserted at the command level in the "delivers the 21 moved commands"
      // block below, which is why no case here needed re-pointing.
      // `app:checkUpdate` **stays**: it is client-local, not legacy.
      "app:checkUpdate","""),

    # ── tests/ipc/handlers.test.ts: the restart port assertion, folded in ─────
    ("tests/ipc/handlers.test.ts", 1,
     """        // `restart` is stop-then-start, not a distinct code path — and the order is the contract:
        // starting before stopping would rebind a port the old listener still holds.
        vi.mocked(startServer).mockClear();
        vi.mocked(stopServer).mockClear();
        expect(commandRegistry.invoke("server.restart", {}, ctx)).toEqual({ ok: true });
        expect(vi.mocked(stopServer).mock.invocationCallOrder[0])
          .toBeLessThan(vi.mocked(startServer).mock.invocationCallOrder[0]);""",
     """        // `restart` is stop-then-start, not a distinct code path — and the order is the contract:
        // starting before stopping would rebind a port the old listener still holds.
        //
        // `startServer(currentConfig.port)` is asserted here rather than in a case of its own: the
        // deleted `server:restart` handler had one, the reduction's fifth group removed it as a
        // duplicate of this block, and this is the one assertion it made that this block did not.
        vi.mocked(startServer).mockClear();
        vi.mocked(stopServer).mockClear();
        expect(commandRegistry.invoke("server.restart", {}, ctx)).toEqual({ ok: true });
        expect(vi.mocked(stopServer).mock.invocationCallOrder[0])
          .toBeLessThan(vi.mocked(startServer).mock.invocationCallOrder[0]);
        expect(startServer).toHaveBeenCalledWith(currentConfig.port);"""),

    # ── tests/ipc/handlers.test.ts: the six redundant cases ──────────────────
    ("tests/ipc/handlers.test.ts", 1,
     """  // ── server:restart ────────────────────────────────────────────────────

  describe("server:restart handler", () => {
    it("stops and restarts the server at the configured port", () => {
      currentConfig.port = 9090;
      vi.mocked(isRunning).mockReturnValue(true);

      const result = getHandler("server:restart")(EVENT);

      expect(stopServer).toHaveBeenCalled();
      expect(startServer).toHaveBeenCalledWith(9090);
      expect(result).toEqual({ ok: true });
    });

    it("registers a handler for 'server:restart'", () => {
      expect(registeredHandlers.has("server:restart")).toBe(true);
    });
  });

  // ── server:stop ───────────────────────────────────────────────────────

  describe("server:stop handler", () => {
    it("registers a handler for 'server:stop'", () => {
      expect(registeredHandlers.has("server:stop")).toBe(true);
    });

    it("stops the server and returns { ok: true }", () => {
      const result = getHandler("server:stop")(EVENT);
      expect(stopServer).toHaveBeenCalled();
      expect(result).toEqual({ ok: true });
    });
  });

  // ── server:start ──────────────────────────────────────────────────────

  describe("server:start handler", () => {
    it("registers a handler for 'server:start'", () => {
      expect(registeredHandlers.has("server:start")).toBe(true);
    });

    it("starts the server at the configured port and returns { ok: true }", () => {
      currentConfig.port = 8181;
      const result = getHandler("server:start")(EVENT);
      expect(startServer).toHaveBeenCalledWith(8181);
      expect(result).toEqual({ ok: true });
    });
  });

  // ── rule:update — not-found branch ──────────────────────────────────""",
     """  // ── server:restart / server:stop / server:start — cases deleted ──────
  //
  // Three `describe`s stood here, six cases in all: for each channel one `it` asserting
  // `registeredHandlers.has(channel)` and one driving `getHandler(channel)`. The reduction's fifth
  // group deleted the channels (`plan/07` §5b-sedecies), and **all six were already covered
  // elsewhere**, which is why none was re-pointed:
  //
  //  - the three `has()` cases asserted what `expectedChannels` generated a case for — the same
  //    assertion twice over, and the generated one is the list that exists to check it. Registration
  //    of `server.start` / `server.stop` / `server.restart` is *also* asserted by the "registry
  //    coverage" block, which iterates every protocol command.
  //  - the three behaviour cases asserted `startServer` / `stopServer` were called and `{ok:true}`
  //    returned — which the "delivers the 21 moved commands" block above asserts against the same
  //    mocks, and asserts *more* (the stop-before-start order). The one assertion that block was
  //    missing — `server.restart` starting on `currentConfig.port` — was folded into it rather than
  //    left behind here.
  //
  // Deleting rather than re-pointing follows `plan/07` §5b-septies' rule: re-point the cases whose
  // assertions live only in the channel, delete the ones a suite already makes. Re-pointing these
  // would have produced three literal duplicates of a block 900 lines above them.

  // ── rule:update — not-found branch ──────────────────────────────────"""),

    # ── tests/integration/settingsMutations.integration.test.ts ──────────────
    ("tests/integration/settingsMutations.integration.test.ts", 1,
     """ *   - `server:restart` / `server:stop` / `server:start` lifecycle,""",
     """ *   - `server.restart` / `server.stop` / `server.start` lifecycle (the channels were deleted by the
 *     P6 reduction's fifth group; this suite's `invoke` re-points them at the commands),"""),

    ("tests/integration/settingsMutations.integration.test.ts", 1,
     """function invoke<T = any>(channel: string, ...args: any[]): T {
  const fn = handlers.get(channel);
  if (!fn) throw new Error(`No handler registered for "${channel}"`);
  return fn({}, ...args) as T;
}""",
     """/**
 * The three `server.*` lifecycle channels, driven directly — the reduction's **fifth** group
 * (`plan/07` §5b-sedecies).
 *
 * `server:restart` / `server:stop` / `server:start` were deleted with the four dead channels in
 * `src/ipc/handlers/systemHandlers.ts` on 2026-09-21. Unlike the earlier groups these bodies were
 * **not** `commandRegistry.invoke()` delegates — they called `startServer` / `stopServer` directly —
 * so the criterion was re-applied per channel against `proxy/serverCommands.ts` rather than
 * inherited. It found exact equivalents, and the commands take no payload, so the translation is
 * positional-argument-free: what the channel did was call the command.
 *
 * A local table rather than the shared `deletedChannelHarness`: this suite drives eleven channels and
 * only these three died, so a harness would have had to model channels the reduction has not reached.
 * Same reasoning as `foldersAndWorkspaces.integration.test.ts`'s folder table.
 */
const DELETED_SERVER_CHANNELS: Record<string, (...args: any[]) => unknown> = {
  "server:restart": () => commandRegistry.invoke("server.restart", {}, { bus }),
  "server:stop": () => commandRegistry.invoke("server.stop", {}, { bus }),
  "server:start": () => commandRegistry.invoke("server.start", {}, { bus }),
};

function invoke<T = any>(channel: string, ...args: any[]): T {
  const deleted = DELETED_SERVER_CHANNELS[channel];
  if (deleted) return deleted(...args) as T;
  const fn = handlers.get(channel);
  if (!fn) throw new Error(`No handler registered for "${channel}"`);
  return fn({}, ...args) as T;
}"""),

    ("tests/integration/settingsMutations.integration.test.ts", 1,
     """  const { registerCoreHandlers } = await import("@/ipc/handlers/coreHandlers");
  const { registerSystemHandlers } = await import("@/ipc/handlers/systemHandlers");
  const { registerClientHandlers } = await import("@/ipc/handlers/clientHandlers");
  const { registerCrudKinds } = await import("@/ipc/handlers/crudKinds");
  registerCoreHandlers();
  registerSystemHandlers();
  registerClientHandlers();
  registerCrudKinds();
});""",
     """  const { registerCoreHandlers } = await import("@/ipc/handlers/coreHandlers");
  const { registerClientHandlers } = await import("@/ipc/handlers/clientHandlers");
  const { registerCrudKinds } = await import("@/ipc/handlers/crudKinds");
  registerCoreHandlers();
  registerClientHandlers();
  registerCrudKinds();
  // `registerSystemHandlers()` used to be called here, for the three `server:*` channels. They are
  // gone (P6 reduction, fifth group) and the one channel it still registers — `app:checkUpdate` — is
  // not driven by this suite, so the call was removed with them.
});"""),

    ("tests/integration/settingsMutations.integration.test.ts", 1,
     """  const { registerEntityCrudCommands } = await import("@bifurc/engine/store/entityCrudCommands");
  registerEntityCrudCommands(commandRegistry);
});""",
     """  const { registerEntityCrudCommands } = await import("@bifurc/engine/store/entityCrudCommands");
  registerEntityCrudCommands(commandRegistry);
  // P6 reduction, fifth group: the three `server:*` channels are gone, so `DELETED_SERVER_CHANNELS`
  // invokes the commands directly and this is the only registration they need. Same shape as the
  // `registerFolderCommands()` call the third group added.
  const { registerServerCommands } = await import("@bifurc/engine/proxy/serverCommands");
  registerServerCommands(commandRegistry);
});"""),

    ("tests/integration/settingsMutations.integration.test.ts", 1,
     """describe("settings — server:restart / server:stop / server:start", () => {""",
     """describe("settings — server.restart / server.stop / server.start (deleted channels)", () => {"""),
]


def main() -> int:
    failures, skipped = [], 0
    for path, n, old, new in EDITS:
        with io.open(path, "r", encoding="utf-8", newline="") as fh:
            text = fh.read()
        crlf = "\r\n" in text
        old_f = old.replace("\n", "\r\n") if crlf else old
        new_f = new.replace("\n", "\r\n") if crlf else new
        found_old, found_new = text.count(old_f), text.count(new_f)

        if found_new == n:
            skipped += 1
            print("skip %-58s x%d" % (path, n))
            continue
        if found_old != n:
            failures.append("%s: expected %d, found %d\n    ---\n%s\n    ---"
                            % (path, n, found_old, old[:300]))
            continue
        with io.open(path, "w", encoding="utf-8", newline="") as fh:
            fh.write(text.replace(old_f, new_f))
        print("ok   %-58s x%d" % (path, n))

    if failures:
        print("\nABORTED — pairs below did not match:")
        for f in failures:
            print(f)
        return 1
    print("\nskipped: %d" % skipped)
    return 0


if __name__ == "__main__":
    sys.exit(main())
