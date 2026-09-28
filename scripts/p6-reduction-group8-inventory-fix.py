"""Group 8 doc edit, pair 4 — correct the group-8 inventory to the verified count and label.

The draft said "28 (4 files) -> 22 (3 files) / 10 live + 12 dead". The re-derived count is 21 where
`ipcMain.handle(` appears literally (`rpcBridge.ts`'s own channel is bridged through
`RPC_CHANNEL`/`EVENT_CHANNEL` constants, not a literal, and `importExportHandlers.ts` holds 4), and the
live/dead split is 10 + 11 there or 11 + 11 if `:formats` counts as dead. Record both readings rather
than one wrong one — the whole point of "re-measure from the tree" is that a bare number is a claim.

One pair per script, guarded on the new text only (append-shaped edits are not idempotent otherwise).
"""
import sys

PATH = "plan/07-phase-6-shell-seam.md"

OLD = (
    "| | before | after |\r\n"
    "| --- | --- | --- |\r\n"
    "| literal channels in the handler files | 28 (4 files) | **22 (3 files)** |\r\n"
    "| live | 10 | 10 |\r\n"
    "| dead | 18 | **12** |\r\n"
    "\r\n"
    "**12 dead channels remain in one file.** `importExportHandlers.ts`'s four (`importExport:formats`,\r\n"
)

NEW = (
    "| | before | after |\r\n"
    "| --- | --- | --- |\r\n"
    "| literal channels in the handler files | 28 (4 files) | **21 (3 files)** |\r\n"
    "| live | 10 | 10 |\r\n"
    "| dead | 18 | **11** |\r\n"
    "\r\n"
    "**The count is measured with `grep -c 'ipcMain.handle('`, and that is `src/ipc/rpcBridge.ts`'s own\r\n"
    "channel excluded** — it is bridged through the `RPC_CHANNEL` / `EVENT_CHANNEL` constants rather than a\r\n"
    "literal string, so it does not appear in a name-level grep. Counting it gives 22. The 21 is quoted\r\n"
    "because it is the reading that decomposes: `clientHandlers.ts` 17 + `systemHandlers.ts` 1 +\r\n"
    "`importExportHandlers.ts` 4 = 22, less the one `systemHandlers.ts` channel that is **client-local**\r\n"
    "(`app:checkUpdate`, which stays) = 21 in the dead/live accounting.\r\n"
    "\r\n"
    "**11 dead channels remain in one file, with one labelling judgement.** `importExportHandlers.ts`'s four\r\n"
    "are `importExport:formats`, `:export`, `:preflight` and `:import`. `:export` and `:preflight` are the\r\n"
    "dead ones under the criterion beyond argument — both own a `dialog.*` step (`showSaveDialog` at `:90`,\r\n"
    "`showOpenDialog` at `:123`). `:import` is dead by the same routing test as every other deleted channel\r\n"
    "(`packages/client/src` names no channel). **`:formats` is the one channel whose label is a judgement\r\n"
    "rather than a count**: it returns a static table and routes nowhere, but it may be genuinely\r\n"
    "client-local in the `app:checkUpdate` sense — so the reduction's last group must *decide* it rather\r\n"
    "than assume it, and a bare \"12 dead\" would have hidden that. `importExport:formats`,\r\n"
)

with open(PATH, "r", encoding="utf-8", newline="") as f:
    text = f.read()

if "11 dead channels remain in one file" in text:
    print("skip: already corrected")
    sys.exit(0)

found = text.count(OLD)
if found != 1:
    print(f"FAIL: pattern appears {found} times, expected 1")
    sys.exit(1)

with open(PATH, "w", encoding="utf-8", newline="") as f:
    f.write(text.replace(OLD, NEW, 1))

print("ok")
