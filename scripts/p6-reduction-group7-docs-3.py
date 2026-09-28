"""Group 7 doc ledger, part 3 — insert §5b-octodecies before §5c.

Part 1 reported "ok" for this pair and then aborted on the *second* pair, exiting before its
write() — so nothing was persisted. Re-applied here with the pair as the only edit, and with the
skip guard testing `new` (TRAPS.md) so a re-run cannot duplicate the section.
"""
import sys

P07 = "plan/07-phase-6-shell-seam.md"

ANCHOR = "### 5c. The mirror problem: five engine commands are registered *from the shell*"

SECTION = """#### 5b-octodecies. The reduction, seventh group — `applicationHandlers.ts` is deleted, and the delta is 0 for the second time (2026-09-21)

**Ten channels, all dead, all one-line delegates — and the file goes with them.** `applications:list` /
`:save` / `:delete` / `:start` / `:stop` / `:getState` / `:getAllStates` / `:getLogs` / `:checkPort` /
`:killPort` left `src/ipc/applicationHandlers.ts`, which is now **deleted** rather than reduced, along
with its import and call in `src/ipc/handlers.ts`. This is the first whole-file deletion since the
second group, and — unlike the fourth, fifth and sixth — it needed no rename, because the file
registered no kinds either.

**The criterion cleared all ten without a fix first, and the reason is on the record from §5c slice 5.**
Each body was `commandRegistry.invoke("<command>", {…}, ctx)` and nothing else: `ctx` is `{ bus }`, the
payload is the caller's arguments re-shaped, and no body touched `dialog`, `app.*`, a trust store or any
field the engine refuses to report. That is the `tls` / `entity.load` class **absent** rather than
missed — and the reason it is absent is worth stating, because it is not luck: §5c slice 5 is the slice
the ledger already calls *"the cleanest of the pass — nothing had to travel"*, since every dependency
(`workspaceFs`'s entity readers, `applications/processSpawner`, `commandGenerator`, `portUtils`) was
already inside `packages/engine`, and the shell file was a pure adapter from the day it was written.
The only shell-side thing in it was `electron`'s `ipcMain`.

**Delta 0, and it is the fourth group's cause rather than the `tls` group's** — which is why the number
alone is not the finding. Two independent reasons, both checked:

  - **No case was in `expectedChannels`**, so no generated `it` was lost. The array never listed an
    `applications:*` channel.
  - **No suite drove one by name.** `tests/integration/applications.integration.test.ts` was the only
    importer of `registerApplicationHandlers`, and it drove all ten — so under §5b-ter's rule this was a
    **re-point**, and a re-point preserves the count.

Both halves matter: §5b-quindecies' "delta 0 has two causes and the count cannot separate them" is
exactly this group's shape, and the separation came from reading the suite, not the number.

**The suite now drives the commands, and the mapping is the deleted bodies' mapping reproduced
verbatim.** `invoke("applications:save", appConfig)` became
`invokeCommand("application.save", { application: appConfig })`; `invoke("applications:start", ws, id,
"run")` became `invokeCommand("application.start", { workspaceId: ws, appId: id, mode: "run" })`; and
`applications:stop` / `:getState` / `:getLogs` / `:checkPort` / `:killPort` — which took their argument
**positionally** — became one-field payloads. That is §5b-quindecies' argument-shift trap in the
direction that avoids it: the mapping is written from the *call sites*, not inferred from the command's
parameter list, which is the sixth group's `workspace:add` lesson applied pre-emptively.

Three incidental consequences of driving commands instead of channels, all recorded rather than
discovered later:

  - **The `waitFor` helper became async.** The commands return promises where the channels were
    synchronous, so `waitFor(() => …status === "exited")` would have polled the value `undefined`
    forever and failed on the timeout rather than on the assertion. The one-line widening
    (`() => boolean | Promise<boolean>`) is load-bearing, not cosmetic.
  - **The `has(channel)` case became `isRegistered(command)`.** The case asserted *registration*, and
    under the reduction the thing that must be registered is the command — the same substitution
    `handlers.test.ts` made for `folder:*` in the third group. It asserts the **namespace** rather than
    the ratchet's whole set, because this suite's failure mode is a command that silently is not there.
  - **The `afterAll` cleanup re-points too** (`application.stop`), or the suite leaks a child process
    when a case fails mid-run.

**The spike's fixture did not grow — the fifth non-growth, and it confirms the corrected rule in the
other direction.** `tests/spike/protocolPoc.test.ts` drives ten channels and none is an
`applications:*` one, so the fixture stays **10 of 10**. §5b-duodecies' "grows with each slice" was
corrected by the second group to *"grows with each **driven** channel"*, and this group is a driven
channel that is simply not in its ten.

**Inventory, re-measured 2026-09-21 (`ipcMain.handle(` call sites):**

| | before | after |
| --- | --- | --- |
| literal channels in the handler files | 38 (5 files) | **28 (4 files)** |
| live | 18 | 18 |
| dead | 20 | **18** |

**18 dead channels remain in two files, and both are all-dead:** `runnerHandlers.ts` (6) and
`importExportHandlers.ts` (4). The other 12 channels are `clientHandlers.ts`'s 17-channel live set
minus nothing and `systemHandlers.ts`'s one client-local channel, plus `importExportHandlers.ts`'s four
live-by-contract ones — re-derived from the tree rather than carried, which is the rule §5b-sedecies
was corrected into.

**The dialog-owner risk subset is still 3, and this group owned none of them.** Re-derived with
`grep -n "dialog\\." src/ipc src/ipc/handlers`: `runner:exportReport`, `importExport:export`,
`importExport:preflight`. **Those are exactly the two groups still to go** — one dialog-owner in
`runnerHandlers.ts`, two in `importExportHandlers.ts` — which makes the remaining work the reduction's
highest-risk part rather than its tail.

"""

text = open(P07, newline="").read()
if text.count(SECTION) >= 1:
    print("skip (already applied)")
    sys.exit(0)
if text.count(ANCHOR) != 1:
    sys.exit(f"ABORT: anchor matched {text.count(ANCHOR)} times")
open(P07, "w", newline="").write(text.replace(ANCHOR, SECTION + ANCHOR, 1))
print("ok")
