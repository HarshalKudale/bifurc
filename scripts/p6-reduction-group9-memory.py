"""Group 9 / reduction closure: update .workbuddy-ai/memory/MEMORY.md.

Three independent replacements, each asserted to match exactly once. The file is LF.

  A. baseline  -> group 9's 101 / 2,458 / 2,417 / 40 / 1, with the delta's mechanism named
  B. eight groups -> nine, the reduction closed, 0 dead remain
  C. the "Remaining: 12 dead in 1 file" bullet -> a closure bullet
"""

from pathlib import Path

P = Path(".workbuddy-ai/memory/MEMORY.md")


PAIRS = [
    # ---- A. baseline ----------------------------------------------------------------
    (
        "- **Baseline (2026-09-21, after group 8): 101 files / 2,462 tests \u2014 2,421 passed, 40 skipped, 1 failed**\n",
        "- **Baseline (2026-09-21, after group 9 \u2014 the reduction is CLOSED): 101 files / 2,458 tests \u2014\n"
        "  2,417 passed, 40 skipped, 1 failed**\n",
    ),
    (
        "  test over group 7's 2,461 is group 8's **new corrupt-run case**, written into the suite during the re-point,\n"
        "  so it sits on both sides of the measurement and the delta *is* 0.\n",
        "  test over group 7's 2,461 is group 8's **new corrupt-run case**, written into the suite during the re-point,\n"
        "  so it sits on both sides of the measurement and the delta *is* 0. **Group 9's delta is \u22124, the reduction's\n"
        "  only genuine ratchet shrink**: its four names *were* in `expectedChannels`, so four generated\n"
        "  `it(\"registers a handler for \u2026\")` cases went with them (110 \u2192 106) \u2014 every other delta came from\n"
        "  somewhere else and had to be explained.\n",
    ),
    # ---- B. group count / closure ---------------------------------------------------
    (
        "### The reduction \u2014 `plan/07` \u00a75b-decies \u2026 \u00a75b-novendecies, one file per commit\n",
        "### The reduction \u2014 `plan/07` \u00a75b-decies \u2026 \u00a75b-vicendecies, one file per commit \u2014 **COMPLETE**\n",
    ),
    (
        "- **Eight groups deleted:** `tlsHandlers.ts` (6, \u03940 \u2014 *re-point*) \u00b7 `syncHandlers.ts` (19, \u0394\u22125 \u2014 *generated*) \u00b7\n"
        "  `folderHandlers.ts` (4, \u0394\u22123) \u00b7 `crudHandlers.ts` \u2192 **`crudKinds.ts`** (5, \u03940 \u2014 **never had cases**) \u00b7\n"
        "  `systemHandlers.ts` (4, \u0394\u22128) \u00b7 `coreHandlers.ts` \u2192 **`coreKinds.ts`** (17, **\u0394\u221225**) \u00b7\n"
        "  `applicationHandlers.ts` (**deleted**, 10, \u03940 \u2014 *both causes: no cases AND a re-point*) \u00b7\n"
        "  `runnerHandlers.ts` (**deleted**, 6, **\u03940 \u2014 third cause: both at once *plus* a duplicated implementation**).\n"
        "  **\u03940 has three causes now and the count separates none of them** \u2014 read per-file. `src/` 20 \u2192 **18** files\n"
        "  (a delete drops the count; a rename does not).\n",
        "- **Nine groups; the reduction is CLOSED.** `tlsHandlers.ts` (6, \u03940 \u2014 *re-point*) \u00b7 `syncHandlers.ts` (19,\n"
        "  \u0394\u22125 \u2014 *generated*) \u00b7 `folderHandlers.ts` (4, \u0394\u22123) \u00b7 `crudHandlers.ts` \u2192 **`crudKinds.ts`** (5, \u03940 \u2014\n"
        "  **never had cases**) \u00b7 `systemHandlers.ts` (4, \u0394\u22128) \u00b7 `coreHandlers.ts` \u2192 **`coreKinds.ts`** (17,\n"
        "  **\u0394\u221225**) \u00b7 `applicationHandlers.ts` (**deleted**, 10, \u03940 \u2014 *both causes: no cases AND a re-point*) \u00b7\n"
        "  `runnerHandlers.ts` (**deleted**, 6, **\u03940 \u2014 third cause: both at once *plus* a duplicated implementation**) \u00b7\n"
        "  `importExportHandlers.ts` (**deleted**, 4, **\u0394\u22124** \u2014 the only genuine ratchet shrink).\n"
        "  **\u03940 has three causes now and the count separates none of them** \u2014 read per-file. `src/` 20 \u2192 **17** files\n"
        "  (a delete drops the count; a rename does not). **Zero dead channels remain; every remaining one is a keeper,\n"
        "  and the dialog-owner subset is now empty.**\n",
    ),
    # ---- C. the remaining bullet -> closure -----------------------------------------
    (
        "- **Remaining: 12 dead in 1 file** \u2014 `importExportHandlers.ts` (4: `:formats`, `:export`, `:preflight`,\n"
        "  `:import`), the reduction's **last group**. Confirmed from the tree 2026-09-21: **21 literal channels in\n"
        "  4 files = 10 live + 11 dead** by a raw `ipcMain.handle(\"` count, and **22 = 11 + 11** treating `:formats`\n"
        "  as dead too (it returns a static table and routes nowhere, so it is the one channel whose live/dead\n"
        "  label is a judgement rather than a count \u2014 say which you are quoting). Dialog-owner subset **2**\n"
        "  (`importExport:export`, `importExport:preflight` \u2014 re-derived, `dialog.showSaveDialog` at `:90` and\n"
        "  `dialog.showOpenDialog` at `:123`) \u2014 never carry it forward (TRAPS.md, \"A count written into prose\").\n"
        "  **Re-measure from the tree, not from prose.**\n",
        "- **Nothing remains.** Re-derived from the tree 2026-09-21, post-group-9: **18 `ipcMain.handle(` call sites,\n"
        "  all keepers** \u2014 17 in `clientHandlers.ts` + 1 client-local in `systemHandlers.ts` (`app:checkUpdate`);\n"
        "  down from 21. Counting `rpcBridge.ts`'s constant-bridged channel gives the other reading. **§5b-vicendecies'\n"
        "  judgement: `:formats` IS dead** (its body is `call(\"export.formats\", {})` and\n"
        "  `packages/engine/src/importExport/commands.ts:59` registers that command) \u2014 it looked ambiguous only because\n"
        "  it is the one channel whose engine command returns data rather than an artifact, so a cheap read has the\n"
        "  shape of a client-local query and of a delegate at once. **Decided by whether a protocol command exists,\n"
        "  not by the cost of the call.** `registerIpcHandlers()` is reduced, **not** deleted, and that is now a\n"
        "  *finished state* \u2014 which makes `plan/07` work item 6's rollback flag weaker than it reads (no legacy path\n"
        "  left to revert to). Dialog-owner subset was tracked 3 \u2192 2 \u2192 **0**; re-derive with `grep -n \"dialog\\.\"`,\n"
        "  never carry it forward (TRAPS.md, \"A count written into prose\"). **Re-measure from the tree, not prose.**\n"
        "- **The reduction's transferable lesson (group 9) \u2014 a twin must carry the STATE, not just the calls.**\n"
        "  `pendingUploads` was cross-channel shell state (`:preflight` writes, `:import` reads), so deleting either\n"
        "  alone breaks the pair with no single-channel test seeing it. The safe check is that the client has **its\n"
        "  own copy** (`packages/client/src/index.ts:493`), not that the channel is unreachable. And the ordering rule\n"
        "  the nine groups kept proving: **write the twin first, document its divergence, retire the shell file last**\n"
        "  \u2014 every group that went wrong (`tlsGenerate`, `entity:load`) deleted before the twin was complete.\n",
    ),
]


def main() -> None:
    text = P.read_text(encoding="utf-8", newline="")
    for i, (old, new) in enumerate(PAIRS, 1):
        n = text.count(old)
        assert n == 1, f"pair {i}: old matched {n} times, expected exactly 1"
        assert new not in text, f"pair {i}: replacement already present"
        text = text.replace(old, new, 1)
    P.write_text(text, encoding="utf-8", newline="")

    raw = P.read_bytes()
    assert raw.count(b"\r\n") == 0, "MEMORY.md is supposed to be LF"
    assert b"Eight groups deleted" not in raw
    assert b"Remaining: 12 dead in 1 file" not in raw
    print(f"ok  MEMORY.md rewritten ({len(raw)} bytes, {raw.count(chr(10).encode())} lines)")


if __name__ == "__main__":
    main()
