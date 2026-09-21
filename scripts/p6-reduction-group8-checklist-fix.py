"""Group 8 doc edit, pair 5 — align plan/13's eighth-group record with the verified inventory.

One pair per script, guarded on the new text only.
"""
import sys

PATH = "plan/13-checklist.md"

OLD = (
    "      **Inventory: 22 literal channels in 3 files = 10 live + 12 dead**, so **12 dead in one file**\r\n"
    "      remain: `importExportHandlers.ts`'s four, the one file §5b-decies classified as **not** delegates\r\n"
)

NEW = (
    "      **Inventory: 21 literal channels in 3 files = 10 live + 11 dead** (the reading measured with\r\n"
    "      `grep -c 'ipcMain.handle('`; counting `rpcBridge.ts`'s constant-bridged channel gives 22), so\r\n"
    "      **11 dead in one file** remain: `importExportHandlers.ts`'s four, the one file §5b-decies classified as **not** delegates\r\n"
)

with open(PATH, "r", encoding="utf-8", newline="") as f:
    text = f.read()

if "10 live + 11 dead" in text:
    print("skip: already aligned")
    sys.exit(0)

found = text.count(OLD)
if found != 1:
    print(f"FAIL: pattern appears {found} times, expected 1")
    sys.exit(1)

with open(PATH, "w", encoding="utf-8", newline="") as f:
    f.write(text.replace(OLD, NEW, 1))

print("ok")
