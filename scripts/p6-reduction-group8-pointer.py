"""Group 8 doc edit, pair 2 — the supersede pointer on the seventh group's closing inventory.

One pair per script. See `p6-reduction-group8-ledger.py` for why.
"""
import sys

PATH = "plan/07-phase-6-shell-seam.md"
OLD = (
    "**18 dead channels remain in two files, and both are all-dead:** `runnerHandlers.ts` (6) and\r\n"
    "`importExportHandlers.ts` (4). The other 12 channels are `clientHandlers.ts`'s 17-channel live set\r\n"
)
NEW = (
    "> **Superseded 2026-09-21 by §5b-novendecies.** `runnerHandlers.ts`'s six channels are now deleted,\r\n"
    "> so the remaining surface is **12 dead channels in one file** (`importExportHandlers.ts`). The\r\n"
    "> paragraph below is kept as the seventh group's measurement, which is what the eighth group's\r\n"
    "> \"before\" column compares against.\r\n"
    "\r\n"
    "**18 dead channels remain in two files, and both are all-dead:** `runnerHandlers.ts` (6) and\r\n"
    "`importExportHandlers.ts` (4). The other 12 channels are `clientHandlers.ts`'s 17-channel live set\r\n"
)

with open(PATH, "r", encoding="utf-8", newline="") as f:
    text = f.read()

if "Superseded 2026-09-21 by §5b-novendecies." in text:
    print("skip: already present")
    sys.exit(0)

found = text.count(OLD)
if found != 1:
    print(f"FAIL: pattern appears {found} times, expected 1")
    sys.exit(1)

with open(PATH, "w", encoding="utf-8", newline="") as f:
    f.write(text.replace(OLD, NEW, 1))

print("ok")
