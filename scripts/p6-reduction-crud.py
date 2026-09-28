#!/usr/bin/env python3
"""P6 reduction, group 4: crudHandlers.ts -> crudKinds.ts, code and ledger.

Two edit sets, one script, because the group is one commit:

  A. the code — the file rename, its five call sites, and the comments that named it
     (`src/ipc/handlers.ts`, the three sibling `*Kinds.ts` files, and four engine comments).
  B. the ledger — `plan/07` §5b-octies' name-exception reversed, §5b-quaterdecies' "remain"
     line superseded, the new §5b-quindecies, `plan/13`'s step-3 progress, `plan/README.md`.

Count-asserting, CRLF-aware, idempotent.

  * Count-asserting: every pair must match exactly `n` times (or already be applied `n`
    times), else the script aborts without touching that file. `Edit`-tool races on this
    tree have silently lost writes before; a count assert turns that into a hard failure.
  * CRLF-aware: `src/**`, most of `tests/**` and `plan/07` / `plan/13` are checked out
    CRLF; `packages/**` and `plan/README.md` are LF. Patterns are written with plain LF and
    translated to the file's own ending — a bare-LF write into a CRLF file leaves one
    mixed-ending line, which is what happened on the first run of this group.
  * Idempotent: an edit whose `new` text is already present is skipped, so a partially
    applied run can be resumed rather than reverted. The guard tests `new` **only**, and
    that is not an optimisation: most of this group's ledger edits are append-shaped
    (`new` = `old` + extra), so `old` survives *inside* `new`, and a `found_old == 0`
    conjunct would never fire — silently appending a second copy on the re-run, which is
    exactly how §5b-quindecies and plan/13's progress paragraph were duplicated.

Kept as the record of how the group was made; not part of the build.
"""
import io
import sys

CODE_EDITS = [
    # ── src/ipc/handlers.ts ────────────────────────────────────────────────────
    ("src/ipc/handlers.ts", 1,
     'import { registerCrudHandlers } from "@/ipc/handlers/crudHandlers";',
     'import { registerCrudKinds } from "@/ipc/handlers/crudKinds";'),

    ("src/ipc/handlers.ts", 1,
     """  // The engine half honours it and falls back to `cfg.webhookPort ?? 9101`, so every existing caller
  // — all of which pass nothing — gets exactly the old behaviour.
  registerWebhookCommands(commandRegistry);""",
     """  // The engine half honours it and falls back to `cfg.webhookPort ?? 9101`, so every existing caller
  // — all of which pass nothing — gets exactly the old behaviour.
  //
  // **The five channels are now deleted** — the reduction's fourth group removed them on 2026-09-21
  // (`plan/07` §5b-quindecies), and `crudHandlers.ts` became `crudKinds.ts` with them. This call is
  // the five commands' only registration.
  registerWebhookCommands(commandRegistry);"""),

    ("src/ipc/handlers.ts", 1,
     """  registerSystemHandlers();
  registerCrudHandlers();
  registerRunnerHandlers();""",
     """  registerSystemHandlers();
  // Renamed from `registerCrudHandlers()` on 2026-09-21: the reduction's fourth group deleted this
  // file's last five channels (`webhook:*`, `webhookServer:*`), so it now registers kinds and nothing
  // else. See `crudKinds.ts`'s header and `plan/07` §5b-quindecies.
  registerCrudKinds();
  registerRunnerHandlers();"""),

    # ── the three sibling kinds files ──────────────────────────────────────────
    ("src/ipc/handlers/graphqlKinds.ts", 1,
     """// add one. `crudHandlers.ts` kept its name on purpose: it registers six kinds *and* five live
// `webhook*` channels, so "handlers" is still half-true there.""",
     """// add one. `crudHandlers.ts` was the one exception, on the grounds that it registered six kinds *and*
// five live `webhook*` channels; the reduction's fourth group deleted those five on 2026-09-21, so the
// exception expired and the file became `crudKinds.ts` / `registerCrudKinds()` like the rest."""),

    ("src/ipc/handlers/grpcKinds.ts", 1,
     """// add one. `crudHandlers.ts` kept its name on purpose: it registers six kinds *and* five live
// `webhook*` channels, so "handlers" is still half-true there.""",
     """// add one. `crudHandlers.ts` was the one exception, on the grounds that it registered six kinds *and*
// five live `webhook*` channels; the reduction's fourth group deleted those five on 2026-09-21, so the
// exception expired and the file became `crudKinds.ts` / `registerCrudKinds()` like the rest."""),

    ("src/ipc/handlers/soapKinds.ts", 1,
     """// add one. `crudHandlers.ts` kept its name on purpose: it registers six kinds *and* five live
// `webhook*` channels, so "handlers" is still half-true there.""",
     """// add one. `crudHandlers.ts` was the one exception, on the grounds that it registered six kinds *and*
// five live `webhook*` channels; the reduction's fourth group deleted those five on 2026-09-21, so the
// exception expired and the file became `crudKinds.ts` / `registerCrudKinds()` like the rest."""),

    # ── engine comments ────────────────────────────────────────────────────────
    ("packages/engine/src/proxy/webhookCommands.ts", 1,
     """ * They were served by `ipcMain.handle` bodies in `src/ipc/handlers/crudHandlers.ts`, which works only
 * while the shell and the engine are the same process. They are not shell concerns: every function""",
     """ * They were served by `ipcMain.handle` bodies in `src/ipc/handlers/crudHandlers.ts` — now
 * `crudKinds.ts`, and those bodies are gone (P6 reduction, fourth group, 2026-09-21) — which worked
 * only while the shell and the engine were the same process. They are not shell concerns: every function"""),

    ("packages/engine/src/store/entityCrudCommands.ts", 1,
     """ *    `soapHandlers`. (Every P6 doc before 2026-09-20 says *six*, including one that then lists exactly
 *    those five names; measured with `git log -S`, five is the number that has ever been true.) They did""",
     """ *    `soapHandlers`. Three of those five have since been renamed to `*Kinds.ts` once their channels
 *    went — `graphql`/`grpc`/`soap` in stage 3b, `crudHandlers` → `crudKinds.ts` in the reduction's
 *    fourth group. Every P6 doc before 2026-09-20 says *six*, including one that then lists exactly
 *    those five names; measured with `git log -S`, five is the number that has ever been true. They did"""),

    ("packages/engine/src/store/entityCrudCommands.ts", 1,
     """  // exactly the `rules` / `mocks` registrations and on no others (`crudHandlers.ts`), so the two""",
     """  // exactly the `rules` / `mocks` registrations and on no others (`crudHandlers.ts`, now
  // `crudKinds.ts`), so the two"""),

    ("packages/engine/src/transport/legacyCompanion.ts", 1,
     """ * guarantee. Verified against `src/ipc/handlers/crudHandlers.ts`, `entityCrudFactory.ts` and
 * `folderHandlers.ts` on 2026-09-18 — the last two are since deleted (`entityCrudFactory.ts` in P6
 * stage 3b, `folderHandlers.ts` in the reduction's third group), so the "registry equivalent" column is""",
     """ * guarantee. Verified against `src/ipc/handlers/crudHandlers.ts` (now `crudKinds.ts` — its channels
 * went in the reduction's fourth group, 2026-09-21), `entityCrudFactory.ts` and
 * `folderHandlers.ts` on 2026-09-18 — the last two are since deleted (`entityCrudFactory.ts` in P6
 * stage 3b, `folderHandlers.ts` in the reduction's third group), so the "registry equivalent" column is"""),

    # ── tests/integration/entityConflictScope.integration.test.ts ──────────────
    ("tests/integration/entityConflictScope.integration.test.ts", 1,
     'import { registerCrudHandlers } from "@/ipc/handlers/crudHandlers";',
     'import { registerCrudKinds } from "@/ipc/handlers/crudKinds";'),

    ("tests/integration/entityConflictScope.integration.test.ts", 1,
     """  registerCrudHandlers();
  // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
  // `registerCrudHandlers()` wires the *channels* and the per-kind options; the four commands those
  // channels delegate to are the engine half and have to be registered explicitly. Without this the
  // channels exist with nothing behind them — `No handler registered` at invocation time.""",
     """  registerCrudKinds();
  // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
  // `registerCrudKinds()` wires the per-kind options those four commands dispatch through — and now
  // nothing else, since the reduction's fourth group took the file's last five channels (2026-09-21).
  // Without this call the four commands throw `"<kind>" is not routed through the CommandRegistry yet`."""),

    # ── tests/integration/foldersAndWorkspaces.integration.test.ts ─────────────
    ("tests/integration/foldersAndWorkspaces.integration.test.ts", 1,
     '  const { registerCrudHandlers } = await import("@/ipc/handlers/crudHandlers");',
     '  const { registerCrudKinds } = await import("@/ipc/handlers/crudKinds");'),

    ("tests/integration/foldersAndWorkspaces.integration.test.ts", 1,
     """  // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
  // `registerCrudHandlers()` wires the *channels* and the per-kind options; the four commands those
  // channels delegate to are the engine half and have to be registered explicitly.""",
     """  // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
  // `registerCrudKinds()` wires the per-kind options those four commands dispatch through. It registers
  // no channels — the reduction's fourth group took the file's last five on 2026-09-21."""),

    ("tests/integration/foldersAndWorkspaces.integration.test.ts", 1,
     """  registerEntityCrudCommands(commandRegistry);
  registerCrudHandlers();
});""",
     """  registerEntityCrudCommands(commandRegistry);
  registerCrudKinds();
});"""),

    # ── tests/integration/settingsMutations.integration.test.ts ────────────────
    ("tests/integration/settingsMutations.integration.test.ts", 1,
     '  const { registerCrudHandlers } = await import("@/ipc/handlers/crudHandlers");',
     '  const { registerCrudKinds } = await import("@/ipc/handlers/crudKinds");'),

    ("tests/integration/settingsMutations.integration.test.ts", 1,
     """  registerClientHandlers();
  registerCrudHandlers();
});""",
     """  registerClientHandlers();
  registerCrudKinds();
});"""),

    ("tests/integration/settingsMutations.integration.test.ts", 1,
     """  // This goes in the `beforeAll` above, **not** next to `registerCrudHandlers()` in the `beforeEach` —""",
     """  // This goes in the `beforeAll` above, **not** next to `registerCrudKinds()` in the `beforeEach` —"""),

    # ── tests/spike/protocolPoc.test.ts ────────────────────────────────────────
    ("tests/spike/protocolPoc.test.ts", 1,
     '    const { registerCrudHandlers } = await import("../../src/ipc/handlers/crudHandlers");',
     '    const { registerCrudKinds } = await import("../../src/ipc/handlers/crudKinds");'),

    ("tests/spike/protocolPoc.test.ts", 1,
     """    // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
    // `registerCrudHandlers()` above wires the channels and the per-kind options; the four commands
    // those channels delegate to are the engine half.""",
     """    // P6 §5c slice 7: the four collapsed `entity.*` commands left `entityCrudFactory.ts`'s module scope.
    // `registerCrudKinds()` above wires the per-kind options the four commands dispatch through; it
    // registers no channels (the reduction's fourth group deleted the file's last five, 2026-09-21)."""),

    ("tests/spike/protocolPoc.test.ts", 1,
     """    registerCoreHandlers();
    registerCrudHandlers();""",
     """    registerCoreHandlers();
    registerCrudKinds();"""),
]


NEW_SECTION = """#### 5b-quindecies. The reduction, fourth group — `crudHandlers.ts` becomes `crudKinds.ts`, and the first group that cost nothing (2026-09-21)

**`src/ipc/handlers/crudHandlers.ts` is gone: 5 channels, all dead** (`webhook:registerActive`,
`webhook:unregisterActive`, `webhookServer:start|stop|status`). It is the first group whose channels are
**not** `commandRegistry.invoke()` delegates — every one of the five calls its engine function directly
(`registerActiveWebhook`, `unregisterActiveWebhook`, `startWebhookServer`, `stopWebhookServer`,
`isWebhookServerRunning` / `getWebhookPort` / `getWebhookServerError`), which is the shape §5a left them in
and the reason §5b-decies' classification had to be re-derived rather than inherited. The widened criterion
(*owns a step the engine does not perform*, §5b-undecies) found none: the engine half is exact, and in one
case strictly more capable — `webhookServer.start` accepts an optional `port`, which the shell body ignored
(`proxy/webhookCommands.ts`).

**The delta is 0, and that is the third distinct signature in four groups.** The five channels were never in
`handlers.test.ts`'s `expectedChannels`, so no generated `it` was lost; and no suite drives them **by name**,
so nothing had to be re-pointed. Their behaviour is asserted at the **command** level already —
`handlers.test.ts`'s "slice 3" block pins the port-resolution chain (explicit `port` → `cfg.webhookPort` →
`9101`) and the three-field status shape, and it does so against `commandRegistry.invoke()`. So a group can
cost zero either because its cases were *re-pointed* (the `tls` signature, §5b-duodecies) or because **it
never had any** — and the count alone cannot tell those apart, which is the same lesson §5b-terdecies drew
from the opposite direction.

**The file was renamed, and the exception that kept its name has expired.** §5b-octies recorded a deliberate
decision *not* to rename it, on the grounds that it registered six kinds **and** five live `webhook*`
channels, so "handlers" was still half-true. Deleting the five channels removes the premise, so the exception
goes with it: the file is now `crudKinds.ts` with `registerCrudKinds()`, matching `soapKinds.ts` /
`graphqlKinds.ts` / `grpcKinds.ts`. **The blast radius is five files** — `src/ipc/handlers.ts` (import +
call), `tests/integration/entityConflictScope.integration.test.ts`,
`tests/integration/foldersAndWorkspaces.integration.test.ts`,
`tests/integration/settingsMutations.integration.test.ts` and `tests/spike/protocolPoc.test.ts` — and each
test file also carried a comment saying `registerCrudHandlers()` "wires the *channels* and the per-kind
options". **That comment is now false in its first half**, which is why it was fixed in the same commit: the
call is what populates the per-kind registry the four collapsed `entity.*` commands dispatch through, and it
registers no channels at all. A stale comment on a *renamed* function is worse than a stale comment on a
deleted one — the deleted one fails loudly, this one reads as current.

**And the spike's fixture did not grow, which is the rule cutting the same way as the second group.**
`tests/spike/protocolPoc.test.ts` drives **ten** channels and none of them is `webhook:*` or
`webhookServer:*` (measured: the file contains no occurrence of "webhook"), so the retired-channel fixture
stays at **7 of 10**. The rule is "grows with each **driven** channel": third group, two driven, fixture grew;
second and fourth, none driven, fixture unchanged.

**Inventory, re-measured 2026-09-21 (`ipcMain.handle(` call sites):**

| | before | after |
| --- | --- | --- |
| literal channels in the handler files | 64 (7 files) | **59 (6 files)** |
| live | 18 | 18 |
| dead | 46 | **41** |

(`src/ipc/rpcBridge.ts` still holds 2 `ipcMain.handle(` call sites and is excluded as before: it registers
`RPC_CHANNEL` and the step-3a push channel — the bridge, i.e. the reduction's *subject* rather than its
target.)

**41 dead channels remain in 5 files**, one handler group per commit: `coreHandlers.ts` (17),
`applicationHandlers.ts` (10), `runnerHandlers.ts` (6), `systemHandlers.ts` (4 dead + `app:checkUpdate`) and
`importExportHandlers.ts` (4). `clientHandlers.ts` is still 17 live / 0 dead. The dialog-owner risk subset
stays at 6, since this group owned no dialog.

**One thing the next group must check, and it is the shape this group just established.** `systemHandlers.ts`
holds four dead channels of two different kinds, and the difference decides the order:

  - its three `server:*` bodies (`server:start` / `server:stop` / `server:restart`) are **self-contained** in
    exactly the way these five were — they call `startServer` / `stopServer` directly rather than through the
    registry — so they need the same confirmation that the `server.*` commands are exact equivalents. They
    are, and §5a's trap is why they were left that way; the check is cheap and it is not optional.
  - `capture:shareJson` owns a `dialog.showSaveDialog` and is one of §5b-decies' remaining six, so it is
    **not** cleared by the criterion until the client's dialog-first ordering is confirmed the way
    `audit:export`'s was (§5b-terdecies).

That is the "SUPERSEDED SHELL HALF" class, and it is the last class that can still hide a lost step.

"""

LEDGER_EDITS = [
    # ── plan/07 §5b-octies: the name exception, reversed ──────────────────────
    ("plan/07-phase-6-shell-seam.md", 1,
     """**`crudHandlers.ts` deliberately keeps its name.** It registers six kinds **and** five live `webhook*`
channels (`webhook:registerActive`, `webhook:unregisterActive`, `webhookServer:start|stop|status`), so
"handlers" is still half-true there. Renaming it would have been the same mistake in reverse.""",
     """**`crudHandlers.ts` deliberately keeps its name — *for now*.** It registers six kinds **and** five live
`webhook*` channels (`webhook:registerActive`, `webhook:unregisterActive`,
`webhookServer:start|stop|status`), so "handlers" is still half-true there. Renaming it would have been the
same mistake in reverse.

> **Superseded 2026-09-21 by §5b-quindecies.** The reduction's fourth group deleted the five channels, so the
> premise above expired and the file became `crudKinds.ts` / `registerCrudKinds()`. The decision was right
> while it lasted, and it is recorded rather than edited away because the *reason* is the useful part: a
> file name is a claim, and this one was true until the channels went."""),

    # ── plan/07 §5b-quaterdecies: the live "remain" line, superseded ──────────
    ("plan/07-phase-6-shell-seam.md", 1,
     """**46 dead channels remain in 6 files**, one handler group per commit: `coreHandlers.ts` (17),
`applicationHandlers.ts` (10), `runnerHandlers.ts` (6), `crudHandlers.ts` (5), `systemHandlers.ts` (4 dead +
`app:checkUpdate`) and `importExportHandlers.ts` (4). `clientHandlers.ts` is still 17 live / 0 dead. The
dialog-owner risk subset is unchanged at 6, since this group owned no dialog.""",
     """**46 dead channels remain in 6 files**, one handler group per commit: `coreHandlers.ts` (17),
`applicationHandlers.ts` (10), `runnerHandlers.ts` (6), `crudHandlers.ts` (5), `systemHandlers.ts` (4 dead +
`app:checkUpdate`) and `importExportHandlers.ts` (4). `clientHandlers.ts` is still 17 live / 0 dead. The
dialog-owner risk subset is unchanged at 6, since this group owned no dialog.

→ **Superseded the same day by §5b-quindecies** (fourth group): `crudHandlers.ts` is gone, leaving **41 dead
in 5 files**. Read the inventory in the *latest* §5b-* section, not the nearest one."""),

    # ── plan/07: insert the new section before "### 5c." ─────────────────────
    ("plan/07-phase-6-shell-seam.md", 1,
     "### 5c. The mirror problem: five engine commands are registered *from the shell*",
     NEW_SECTION + "### 5c. The mirror problem: five engine commands are registered *from the shell*"),

    # ── plan/13-checklist.md: the fourth-group progress paragraph ────────────
    ("plan/13-checklist.md", 1,
     """      which cannot drift. The spike's retired-channel fixture **grew 5 → 7 of 10** (`folder:add` /
      `folder:move` are driven), the counterpart to the second group's non-growth. **Inventory: 64 literal
      channels in 7 files = 18 live + 46 dead**, so **46 dead across 6 files** remain.""",
     """      which cannot drift. The spike's retired-channel fixture **grew 5 → 7 of 10** (`folder:add` /
      `folder:move` are driven), the counterpart to the second group's non-growth. **Inventory: 64 literal
      channels in 7 files = 18 live + 46 dead**, so **46 dead across 6 files** remain.
      **Progress 2026-09-21, fourth group — `crudHandlers.ts` becomes `crudKinds.ts`, and the first group
      that cost nothing** (`plan/07` §5b-quindecies). Five channels, all dead
      (`webhook:registerActive` / `unregisterActive`, `webhookServer:start|stop|status`) — and the **first
      group whose bodies are not `invoke()` delegates**: each calls its engine function directly, the shape
      §5a left them in, so the criterion had to be re-applied rather than inherited. It cleared them: the
      engine half is exact, and `webhookServer.start` is strictly *more* capable (it honours an optional
      `port` the shell body ignored). **Delta 0 — the third distinct signature in four groups**, and not the
      `tls` one: the five were never in `expectedChannels` *and* no suite drives them by name, so nothing was
      lost and nothing needed re-pointing. Their behaviour is already asserted at the **command** level
      (`handlers.test.ts` "slice 3": the port chain `port` → `cfg.webhookPort` → `9101`, and the three-field
      status shape). **Suite: 101 / 2,492 / 2,451 / 40 / 1** — unchanged, as predicted. The **rename** is the
      other half of the group: §5b-octies' decision to keep the `*Handlers` name rested on "six kinds **and**
      five live `webhook*` channels", and deleting the five expired the premise — so it is `crudKinds.ts` /
      `registerCrudKinds()` now, matching its three siblings. Five call sites, each of which also carried a
      comment saying the call "wires the *channels* and the per-kind options" — **false in its first half now,
      and a stale comment on a *renamed* function is worse than on a deleted one, because it reads as
      current**. The spike's fixture **did not grow** (it drives none of the five; the file contains no
      occurrence of "webhook"), the same non-growth as the second group. **Inventory: 59 literal channels in
      6 files = 18 live + 41 dead**, so **41 dead across 5 files** remain."""),

    # ── plan/README.md: the P6 status row's tail ─────────────────────────────
    ("plan/README.md", 1,
     """`crudHandlers.ts` keeps its name (six kinds *and* five live `webhook*` channels). **"Six caller files" was corrected to five** throughout — it was never six, and one sentence claiming it listed exactly five names. **Still open:** `registerIpcHandlers()`'s own reduction. |""",
     """`crudHandlers.ts` kept its name at the time (six kinds *and* five live `webhook*` channels) — **that exception expired 2026-09-21**, when the reduction's fourth group deleted the five and the file became `crudKinds.ts`. **"Six caller files" was corrected to five** throughout — it was never six, and one sentence claiming it listed exactly five names. **The reduction is under way, one handler group per commit** (`plan/07` §5b-duodecies … §5b-quindecies): `tlsHandlers.ts` (6 channels, Δ0), `syncHandlers.ts` (19, Δ−5), `folderHandlers.ts` (4, Δ−3) and `crudHandlers.ts` (5, Δ0) are deleted, leaving **41 dead channels in 5 files** out of 59 literal = 18 live + 41 dead. `clientHandlers.ts` (17 live / 0 dead) is untouchable. **Still open:** `registerIpcHandlers()`'s own reduction. |"""),

    # ── plan/README.md: a stale fixture count in the same row ────────────────
    ("plan/README.md", 1,
     "its retired-channel fixture now covers 4 of the 10 channels it drives",
     "its retired-channel fixture now covers 7 of the 10 channels it drives"),
]


def apply(edits, label):
    failures, skipped = [], 0
    for path, n, old, new in edits:
        with io.open(path, "r", encoding="utf-8", newline="") as fh:
            text = fh.read()
        if "\r\n" in text:
            old_f, new_f = old.replace("\n", "\r\n"), new.replace("\n", "\r\n")
        else:
            old_f, new_f = old, new
        found_old, found_new = text.count(old_f), text.count(new_f)

        # Idempotency, and the rule is subtler than it looks.
        #
        # Most of this group's ledger edits are **append-shaped**: `new` is `old` plus an extra
        # paragraph (the supersede pointer, the progress note) or `old` with the new section
        # prepended. For those, applying the edit leaves `old` **still present** — as a substring of
        # `new`. So a guard of the form `found_old == 0 and found_new == n` never fires for them, and a
        # re-run appends a second copy. That is exactly what happened on this group's first re-run:
        # §5b-quindecies and plan/13's progress paragraph were duplicated.
        #
        # The correct guard asks only whether `new` is already there. If it is present exactly `n`
        # times the edit is applied; whether `old` survives inside it is irrelevant.
        if found_new == n:
            skipped += 1
            print("skip %-62s x%d" % (path, n))
            continue
        if found_old != n:
            failures.append("%s: expected %d, found %d\n    ---\n%s\n    ---"
                            % (path, n, found_old, old[:300]))
            continue
        with io.open(path, "w", encoding="utf-8", newline="") as fh:
            fh.write(text.replace(old_f, new_f))
        print("ok   %-62s x%d" % (path, n))
    if failures:
        print("\nABORTED (%s) - pairs below did not match:" % label)
        for f in failures:
            print(f)
    return failures, skipped


def main() -> int:
    fa, sa = apply(CODE_EDITS, "code")
    fb, sb = apply(LEDGER_EDITS, "ledger")
    if fa or fb:
        return 1
    import subprocess
    out = subprocess.run(
        ["git", "grep", "-n", "-e", "crudHandlers", "-e", "registerCrudHandlers",
         "--", "*.ts", "*.tsx"],
        capture_output=True, text=True,
    ).stdout
    print("\n--- residual .ts/.tsx references (all deliberate past-tense) ---")
    print(out or "(none)")
    print("\nskipped: %d code, %d ledger (idempotent re-run)" % (sa, sb))
    return 0


if __name__ == "__main__":
    sys.exit(main())
