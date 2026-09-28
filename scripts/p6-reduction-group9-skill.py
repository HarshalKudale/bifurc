"""Group 9 / reduction closure: refresh the bifurc-dead-channel-reduction skill.

Four edits, each asserted to match exactly once:
  A. the frontmatter description (last group + the closure)
  B. "eight groups are done" -> nine, reduction CLOSED
  C. the remaining-inventory paragraph -> the closure state
  D. the §5b-* reference range
Plus a new section recording the group-9 lesson (state, not just calls).
"""

from pathlib import Path

P = Path(r"C:\Users\wrath\.workbuddy-ai\skills\bifurc-dead-channel-reduction\SKILL.md")

PAIRS = [
    # ---- D. the §5b-* range in the intro ------------------------------------------
    (
        "criterion, and \u00a75b-duodecies \u2026 \u00a75b-novendecies are the eight completed groups \u2014 this file is the checklist.\n",
        "criterion, and \u00a75b-duodecies \u2026 \u00a75b-vicendecies are the **nine completed groups \u2014 the reduction is\n"
        "closed** \u2014 this file is the checklist.\n",
    ),
    # ---- B. the Progress line ------------------------------------------------------
    (
        "**Progress: eight groups are done** \u2014 `tlsHandlers.ts` (6, delta 0), `syncHandlers.ts` (19, delta \u22125),\n"
        "`folderHandlers.ts` (4, delta \u22123), `crudHandlers.ts` \u2192 **`crudKinds.ts`** (5, delta 0), **`systemHandlers.ts`\n"
        "reduced, not deleted** (4, delta \u22128), **`coreHandlers.ts` \u2192 `coreKinds.ts`** (17, **delta \u221225** \u2014 the\n"
        "largest group, and the one that found two live bugs), **`applicationHandlers.ts` deleted** (10, delta 0),\n"
        "and **`runnerHandlers.ts` deleted** (6, **delta 0 \u2014 the first group with a genuine step-owner**).\n"
        "Re-measured 2026-09-21 after group 8: **22 literal channels remain in 3 files = 10 live + 12 dead**, so\n"
        "**12 dead in 1 file** remains \u2014 `importExportHandlers.ts` (4), the reduction's **last group**, and the one\n"
        "file whose bodies are **not** delegates (the P3 client half: dialog + bytes + the `pendingUploads` map,\n"
        "filed `transport`/`shim` in `surface.ts`). Dialog-owner subset is **2** (`importExport:export`,\n"
        "`importExport:preflight`), so the reduction ends on its **highest-risk** file rather than its easiest.\n"
        "`clientHandlers.ts` (17) left the reduction's scope: 17 live / 0 dead. Re-measure before trusting any of\n"
        "this, and read the warning under *Traps* about the two partitions before quoting a live/dead split.\n",
        "**Progress: nine groups \u2014 the reduction is CLOSED.** `tlsHandlers.ts` (6, delta 0), `syncHandlers.ts`\n"
        "(19, delta \u22125), `folderHandlers.ts` (4, delta \u22123), `crudHandlers.ts` \u2192 **`crudKinds.ts`** (5, delta 0),\n"
        "**`systemHandlers.ts` reduced, not deleted** (4, delta \u22128), **`coreHandlers.ts` \u2192 `coreKinds.ts`** (17,\n"
        "**delta \u221225** \u2014 the largest group, and the one that found two live bugs), **`applicationHandlers.ts`\n"
        "deleted** (10, delta 0), **`runnerHandlers.ts` deleted** (6, **delta 0 \u2014 the first genuine step-owner**),\n"
        "and **`importExportHandlers.ts` deleted** (4, **delta \u22124 \u2014 the only genuine ratchet shrink**, because its\n"
        "four names *were* in `expectedChannels`).\n"
        "\n"
        "**Post-group-9 state, re-derived 2026-09-21: `grep -c 'ipcMain.handle('` gives 18 call sites, ALL keepers**\n"
        "\u2014 17 in `clientHandlers.ts` + 1 client-local in `systemHandlers.ts` (`app:checkUpdate`); down from 21 at\n"
        "the start. Counting `rpcBridge.ts`'s **constant-bridged** channel gives the other reading (21 vs 22 as the\n"
        "pre-state), so always say which you are quoting. **Zero dead channels remain; every remaining one is a\n"
        "keeper, and the dialog-owner subset is empty** (tracked 3 \u2192 2 \u2192 0; re-derive with `grep -n \"dialog\\.\"`,\n"
        "never carry it forward). `clientHandlers.ts` (17) was never in scope: 17 live / 0 dead. **If a future task\n"
        "says a channel looks dead, the reduction's answer is that none are** \u2014 treat the claim as a new claim and\n"
        "re-derive it, but do not look for the twelve. Re-measure before trusting any of this, and read the warning\n"
        "under *Traps* about the two partitions before quoting a live/dead split.\n",
    ),
    # ---- C. the frontmatter description --------------------------------------------
    (
        'description: Delete \u2014 or reduce \u2014 a group of dead legacy ipcMain.handle channels from the Bifurc shell (I:/workspace/Bifurc-Worskspace/Bifurc) \u2014 the P6 reduction of `registerIpcHandlers()` that follows stage 3b. Use when a task mentions the channel reduction, deleting a handler group, `registerIpcHandlers()` shrinking, "12 dead channels", the risk subset / superseded shell halves, `tlsHandlers.ts` / `syncHandlers.ts` / `folderHandlers.ts` / `crudHandlers.ts` (now `crudKinds.ts`) / `systemHandlers.ts` / `coreHandlers.ts` (now `coreKinds.ts`) / `applicationHandlers.ts` / `runnerHandlers.ts` / `importExportHandlers.ts` or the other handler files, or plan/07 \u00a75b-decies through \u00a75b-novendecies. Also use before deleting ANY channel that looks dead, because reachability does not decide deletability.\n',
        'description: Delete \u2014 or reduce \u2014 a group of dead legacy ipcMain.handle channels from the Bifurc shell (I:/workspace/Bifurc-Worskspace/Bifurc) \u2014 the P6 reduction of `registerIpcHandlers()` that follows stage 3b. **COMPLETE 2026-09-21 in nine groups; zero dead channels remain and `registerIpcHandlers()` is reduced to the local-handler entry point.** Use when a task mentions the channel reduction, deleting a handler group, `registerIpcHandlers()` shrinking, the risk subset / superseded shell halves, `tlsHandlers.ts` / `syncHandlers.ts` / `folderHandlers.ts` / `crudHandlers.ts` (now `crudKinds.ts`) / `systemHandlers.ts` / `coreHandlers.ts` (now `coreKinds.ts`) / `applicationHandlers.ts` / `runnerHandlers.ts` / `importExportHandlers.ts` or the other handler files, or plan/07 \u00a75b-decies through \u00a75b-vicendecies. Also use before deleting ANY channel that looks dead, because reachability does not decide deletability \u2014 and after this reduction, the correct answer to "is this one dead too" is normally **no**.\n',
    ),
]


NEW_SECTION = """
## The twin must carry the STATE, not just the calls (group 9's contribution)

The criterion is about *behaviour* moving to the client, and behaviour includes **state that outlives one
call**. Group 9 was the only group where a channel pair shared a piece of shell state: `:preflight` wrote
`pendingUploads.set(localPath, blobId)` so that `:import` could skip a re-upload, and `:import`'s `finally`
forgot it. That map was cross-channel, so **deleting either channel alone would have broken the pair in a way
no single-channel test could see** \u2014 and no count would have shown it.

The safe check is therefore **not** "is the channel unreachable" (reachability never decides anything here) but
**"does the client carry the state, not merely the calls"**. Verified, not assumed:
`packages/client/src/index.ts:493` declares its **own** `pendingUploads` map with `rememberUpload` /
`forgetUpload` / `uploadOnce`, same key, same replace-and-release-the-displaced-entry behaviour, same re-upload
fallback for an app restart between the two steps. **When a group's bodies touch a module-scope `Map`, `Set`,
or counter, grep the twin for its own declaration before deleting anything.**

**And the ordering rule the nine groups kept re-proving.** The client file's own comment named the deletion
before it happened (`index.ts:354`) and said the copy existed *"so moving it would not mean editing a file that
is about to be deleted"*. That is the correct order: **write the twin, document the divergence, retire the
shell file last.** Every group that went wrong (`tls`'s `tlsGenerate`, group 6's `entity:load`) went wrong by
deleting before the twin was complete \u2014 a latent contract and a live bug are the same shape at deletion time,
and only the evidence distinguishes them.
"""


def main() -> None:
    text = P.read_text(encoding="utf-8", newline="")
    for i, (old, new) in enumerate(PAIRS, 1):
        n = text.count(old)
        assert n == 1, f"pair {i}: old matched {n} times, expected exactly 1"
        assert new not in text, f"pair {i}: replacement already present"
        text = text.replace(old, new, 1)

    anchor = "## Three shapes, only one of which is a delegate (group 8's contribution)\n"
    assert text.count(anchor) == 1, "the group-8 section header did not match exactly once"
    assert "group 9's contribution" not in text, "the new section is already present"
    text = text.replace(anchor, NEW_SECTION.lstrip("\n") + "\n" + anchor, 1)

    P.write_text(text, encoding="utf-8", newline="")
    raw = P.read_bytes()
    assert raw.count(b"\r\n") == 0, "the skill is LF"
    nine = raw.count(b"nine groups")
    state = raw.count(b"carry the STATE")
    print(f"ok  SKILL.md: {len(raw)} bytes; nine-groups={nine}; state-section={state}")


if __name__ == "__main__":
    main()
