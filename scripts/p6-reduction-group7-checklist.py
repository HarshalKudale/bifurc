"""Group 7 — append the progress record to plan/13-checklist.md, after the sixth group's record.

CRLF file. The anchor is the last line of the sixth group's progress paragraph, so the new record
lands between it and the `step 3d` checkbox that follows.
"""
import sys

P13 = "plan/13-checklist.md"

ANCHOR = (
    '      where the table mapped `(id, name)` — discovered as `Invalid payload for command "workspace.add":\r\n'
    '      expected string, received undefined` **on `name`**.\r\n'
)

ADD = (
    "      **Progress 2026-09-21, seventh group — `applicationHandlers.ts` is deleted, ten channels, and the\r\n"
    "      delta is 0 for the second time** (`plan/07` §5b-octodecies). All ten `applications:*` channels were\r\n"
    "      one-line `commandRegistry.invoke()` delegates with **no step of their own** — no `dialog`, no\r\n"
    "      `app.*`, no field the engine refuses to report — so the criterion cleared the whole file and nothing\r\n"
    "      had to travel. That is not luck: §5c slice 5 is the slice the ledger already calls *\"the cleanest of\r\n"
    "      the pass\"*, because every dependency was already inside `packages/engine` and the shell file was a\r\n"
    "      pure adapter from the day it was written. The file is **deleted**, not reduced (it registered no\r\n"
    "      kinds either), and `registerApplicationHandlers()` left `src/ipc/handlers.ts` with it — the first\r\n"
    "      whole-file deletion since the second group. `src/` goes 20 → 19 files, which is the integrity check\r\n"
    "      (a delete drops the count; a rename does not). **Delta 0 = the fourth group's cause, not the `tls`\r\n"
    "      group's**, and both halves were checked rather than assumed: no `applications:*` entry was ever in\r\n"
    "      `expectedChannels` (no generated `it` lost), and no suite drove one **by name** —\r\n"
    "      `tests/integration/applications.integration.test.ts` drove all ten, so this was a **re-point**, and\r\n"
    "      a re-point preserves the count. **Suite: 101 / 2,461 / 2,420 / 40 / 1** — unchanged, the 1 being the\r\n"
    "      standing environmental `soap.execute`. (One extra failure appeared and was dismissed as a flake\r\n"
    "      *with evidence, not by assuming*: `tests/store/gitStore.test.ts`'s \"creates a git commit with the\r\n"
    "      correct subject format\" failed at **10017 ms** in the full run, where that file spends 108 s and\r\n"
    "      each git test ~5 s — and the same suite runs **24 passed (24)** in isolation.) The suite now drives\r\n"
    "      the commands with the deleted bodies' positional → payload mapping reproduced **verbatim**, which is\r\n"
    "      §5b-quindecies' argument-shift trap avoided by reading the call sites rather than the command's\r\n"
    "      parameter list. Three consequences worth naming: `waitFor` had to become async (the commands return\r\n"
    "      promises where the channels were synchronous, so a sync predicate would have polled `undefined`\r\n"
    "      forever and failed on the timeout), the `has(channel)` case became `isRegistered(command)` over the\r\n"
    "      **namespace**, and the `afterAll` cleanup re-points too or the suite leaks a child process.\r\n"
    "      **Inventory: 28 literal channels in 4 files = 10 live + 18 dead**, so **18 dead in 2 files** remain\r\n"
    "      — `runnerHandlers.ts` (6) and `importExportHandlers.ts` (4) — both all-dead. The spike's fixture did\r\n"
    "      **not** grow (its fifth non-growth; it drives no `applications:*` channel), staying at **10 of 10**,\r\n"
    "      which confirms §5b-duodecies' corrected rule — *\"grows with each **driven** channel\"* — in the other\r\n"
    "      direction. Dialog-owner subset **still 3** (`runner:exportReport`, `importExport:export`,\r\n"
    "      `importExport:preflight`), re-derived with `grep -n \"dialog\\.\"` rather than carried: **exactly the\r\n"
    "      two groups still to go**, which makes the remainder the reduction's highest-risk part rather than\r\n"
    "      its tail.\r\n"
)

text = open(P13, newline="").read()
if text.count(ADD) >= 1:
    print("skip (already applied)")
    sys.exit(0)
if text.count(ANCHOR) != 1:
    sys.exit(f"ABORT: anchor matched {text.count(ANCHOR)} times")
open(P13, "w", newline="").write(text.replace(ANCHOR, ANCHOR + ADD, 1))
print("ok")
