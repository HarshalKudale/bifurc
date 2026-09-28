"""Group 7 doc ledger, part 2 — the supersede pointer on §5b-septendecies' inventory prose.

Written separately from part 1 because part 1 aborted on a CRLF mismatch: the patterns in these
scripts read with `newline=""`, so a multi-line `old` must use the file's own line ending (CRLF
here). A bare-`\\n` pattern finds 0 — a clean abort, which is the behaviour TRAPS.md documents.
"""
import sys

P07 = "plan/07-phase-6-shell-seam.md"

OLD = (
    "**20 dead channels remain, in three files, and all three are all-dead:** `applicationHandlers.ts` (10),\r\n"
    "`runnerHandlers.ts` (6), `importExportHandlers.ts` (4). `clientHandlers.ts` stays 17 live / 0 dead, and\r\n"
    "`systemHandlers.ts` holds the one live client-local channel."
)
NEW = (
    OLD
    + "\r\n"
    "\r\n"
    "> **Superseded 2026-09-21 by §5b-octodecies** (seventh group): `applicationHandlers.ts` is deleted,\r\n"
    "> leaving **18 dead in 2 files**, both all-dead. Read the inventory in the *latest* §5b-* section,\r\n"
    "> not the nearest one."
)

text = open(P07, newline="").read()
if text.count(NEW) == 1:
    print("skip (already applied)")
    sys.exit(0)
found = text.count(OLD)
if found != 1:
    sys.exit(f"ABORT: expected 1 match, found {found}")
open(P07, "w", newline="").write(text.replace(OLD, NEW))
print("ok")
