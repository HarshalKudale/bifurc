"""Group 9 doc edit — insert plan/07 §5b-vicendecies before the `### 5c.` anchor.

This is the reduction's LAST group, so the section doubles as its closure record. One pair per script;
CRLF throughout; verified afterwards with `grep -c` on the heading and a bare-LF count.
"""
import sys

PATH = "plan/07-phase-6-shell-seam.md"
ANCHOR = "### 5c. The mirror problem: five engine commands are registered *from the shell*"

with open(PATH, "r", encoding="utf-8", newline="") as f:
    text = f.read()

if "#### 5b-vicendecies" in text:
    print("skip: already present")
    sys.exit(0)

found = text.count(ANCHOR)
if found != 1:
    print(f"FAIL: anchor appears {found} times, expected 1")
    sys.exit(1)

section = """#### 5b-vicendecies. The reduction, ninth and last group — `importExportHandlers.ts` is deleted, the strongest delta of the whole reduction, and the surface is closed (2026-09-21)

**Four channels, the file deleted, and the only group that cost the ratchet anything.**
`importExport:formats`, `:export`, `:preflight` and `:import` left
`src/ipc/importExportHandlers.ts`, which is now **deleted** rather than reduced, along with its import
and call in `src/ipc/handlers.ts`. `src/` goes 18 → 17 `.ts` files.

**Delta −4, and it is the reduction's only genuine ratchet shrink — every earlier group's delta came
from somewhere else.** The four names *were* in `tests/ipc/handlers.test.ts`'s `expectedChannels`, so
four generated `it("registers a handler for …")` cases went with them: **110 → 106 tests**. §5b-decies
… §5b-novendecies recorded Δ0 four times and Δ−5, Δ−3, −8, −25 for the others, and every one of those
had to be *explained* (a re-point, a generated-list shrink from a different mechanism, a suite that
was re-pointed wholesale). This one needs no explanation beyond the list itself, which is what makes it
the cleanest measurement the reduction produced — and it is worth noting that it arrives *last*,
because the file that was easiest to see was the one that was hardest to delete safely.

**This is the group the whole criterion was built for, and the reason it went last.** §5b-decies
classified `importExportHandlers.ts` as the **only** file whose bodies are not delegates, and
§5b-novendecies left it as the last work with the note that *"the reduction ends on its highest-risk
file rather than its easiest"*. The file opened with its own one-line statement of the shape: it is
**the P3 client half** — dialog, bytes, and the `pendingUploads` map — not an adapter. Applying the
criterion per channel:

  - **`importExport:formats` — the judgement §5b-novendecies flagged, and it resolves to dead.** It was
    the one channel whose label was a judgement rather than a count: it returns a static table, routes
    nowhere, and could have been client-local in the `app:checkUpdate` sense. It is not. Its body is
    `call("export.formats", {})` — a one-line delegate to an engine command — and
    `packages/engine/src/importExport/commands.ts:59` registers that command. That file's header even
    records the history: *"this module does register `export.formats` — the shell used to register that
    one itself, and it moved here so all four `export.*`/`import.*` commands have a single owner and a
    single call site."* So the body owns nothing, and the **reason it looked ambiguous is that it is the
    only one of the four whose engine command returns data rather than an artifact** — a cheap read has
    the shape of a client-local query and the shape of a delegate at the same time. The difference is
    decided by whether a protocol command exists, not by the cost of the call.
  - **`importExport:export` — a genuine step-owner, and the reduction's second dialog-first egress.**
    Dialog → `export.create` (filename only crosses, never the path) → `writeArtifact`. Twin:
    `packages/client/src/index.ts:585`'s `exportData`, which its own comment ties to
    `File_Ops_Protocol.md` §3.2's ordering rule and to `pickSavePath`'s existence.
  - **`importExport:preflight` — a step-owner with *four* steps.** Dialog → read + size-cap →
    `blob.put` → `import.preflight` → and then it **remembers the upload** (`rememberUpload`). The last
    step is the one a reader is most likely to miss, because it is not a call:
    `pendingUploads.set(localPath, blobId)` is what lets the *next* channel avoid re-uploading the file.
    Twin: `packages/client/src/index.ts:626`, which performs all four.
  - **`importExport:import` — a step-owner that *reads* what preflight wrote.** `uploadFor(filePath)` is
    the map lookup with a re-upload fallback (an app restart between the two steps), then
    `import.commit`, then a `finally` that both forgets the upload and releases the blob. Twin:
    `index.ts:653`, same `finally`, same order.

**The `pendingUploads` map was the specific thing that could have been lost silently — and the check is
that the client has its own copy, not that the channel is unreachable.** The map is the only piece of
state P3 added to the shell, and it is cross-channel: `:preflight` writes it, `:import` reads it, so
deleting either alone would have broken the pair in a way no single-channel test could see. Verified
rather than assumed: `packages/client/src/index.ts:493` declares **its own** `pendingUploads =
new Map<string, string>()`, with `rememberUpload` / `forgetUpload` / `uploadOnce` as closures at `:499`
and `:512` — same key (the local path), same replace-and-release-the-displaced-entry behaviour, same
fallback. That is the `tls` group's lesson applied in the direction that avoids it: the question is not
"is the channel dead" but "does the twin carry the **state**, not just the calls".

**The client's own comment named this deletion before it happened.** `packages/client/src/index.ts:354`:
*"A copy of the shell's `kindLabel` in `src/ipc/importExportHandlers.ts`. That file is now dead … and it
goes at step 3's final deletion, at which point this becomes the only copy. Copied rather than moved for
one commit, because moving it would mean editing a file that is about to be deleted."* That is the
pattern the reduction kept rediscovering the hard way — the twin was written first, its divergence
documented, and the shell file retired after. It is worth naming as the **correct order**, because every
group that went wrong (§5b-undecies' `tlsGenerate`, §5b-septendecies' `entity:load`) went wrong by
deleting before the twin was complete.

**The suite cost was one list, and the coverage argument is the strongest of the nine groups.** No suite
drives an `importExport:*` channel by name — the two integration files that mention the path do so in
**comments only** (`importExportHarness.ts:13`, `importExportBlob.integration.test.ts:179`), and both
were updated to point at the client. `handlers.test.ts`'s `expectedChannels` lost its four names, and
the `vi.mock("@bifurc/engine/importExport/registry")` block above it was **removed**: its comment
justified it by *"its handlers register via the captured mockIpcMain"*, and with those handlers gone it
was stubbing five functions nothing in the graph called. Removing it was verified, not assumed — the
suite passes 106 without it, and the mock's only remaining effect would have been to keep a dead
expectation alive.

The equivalence rests on `packages/client/tests/client.test.ts`, which carries **two `describe` blocks
and 22 references** for these four methods — including exactly the assertions this file could not make:
*"exportData asks where BEFORE it asks the engine to render"*, *"exportData writes to the path the user
chose and does not open a second dialog"*, *"preflightImport returns the path it chose, so the next step
can reuse it"*, and *"importData reuses the path and the blob — no second dialog, no second upload"*.
Those are **ordering and count** assertions, which is what the shell file's behaviour consisted of.
Nothing had to be added to prove the twin: like group 8's `runner:exportReport`, the client was already
better tested than the shell copy it replaced.

**Inventory, re-measured 2026-09-21 (`ipcMain.handle(` call sites):**

| | before | after |
| --- | --- | --- |
| literal channels in the handler files | 21 (3 files) | **17 (2 files)** |
| live | 10 | **10** |
| dead | 11 | **0** |

**The reduction is closed: every remaining channel is a keeper.** To state the numbers without the
ambiguity that has caught earlier groups: `grep -c 'ipcMain.handle('` gives **17 in
`clientHandlers.ts` and 1 in `systemHandlers.ts` — 18 total**, against 21 before this group. All of the
17 are live (the client reaches them through `local.*`), and the 1 is `app:checkUpdate`, which is
**client-local by classification** rather than live-legacy. So the table's "17 (2 files)" row counts
`clientHandlers.ts` plus `systemHandlers.ts`'s survivor, and the live/dead split is 10 + 0 against a
remainder of 1 client-local — three labels for 18 channels, which is exactly why §5b-sedecies was
corrected into "re-derive per file" rather than quoting one total. **Zero dead channels remain**, and
`registerIpcHandlers()` is now **the local-handler entry point** the plan said it would be reduced to.

**`registerIpcHandlers()` is reduced, not deleted — and that is now a finished state rather than a
pending one.** The function registers the engine command surface, calls the two remaining local handler
entry points (`registerClientHandlers()` in `src/main.ts:294`, `registerSystemHandlers()` here), and
calls `registerRpcBridge()`. Everything the reduction set out to remove has been removed; what is left
is the set the plan's "Local handlers stay local" table names. The rollback flag in `plan/07` work item
6 is correspondingly weaker than it was — there is no longer a legacy path to revert *to* — which is
worth flagging for anyone reading that item as still current.

**The dialog-owner risk subset is now empty.** Re-derived with `grep -n "dialog\\." src/ipc
src/ipc/handlers`: the only hits are inside `clientHandlers.ts`'s own fourteen local channels
(`dialog:openFile`, `dialog:pickFilePath`, `dialog:pickFolderPath`, `client:pickSavePath`,
`client:writeArtifact`, `client:readArtifactFile`), which are the local surface and not reduction
targets. The three that were tracked — `runner:exportReport` (group 8), `importExport:export` and
`importExport:preflight` (this group) — are all gone, and in each case the dialog moved to the client
where the plan's architecture puts it.

**The spike's fixture did not grow — the seventh non-growth, and its comment is now stale in a way worth
recording.** `tests/spike/protocolPoc.test.ts` drives ten channels and none is an `importExport:*` one,
so it stays **10 of 10**. Its paragraph names `importExportHandlers.ts`'s four among the dead ones it
never touches, and that sentence is now **historical**: it was updated in group 8 to say the surface was
down to eighteen dead in two files, and this group empties it. The file is left correct-by-context
rather than edited a second time in one day, but the next reader should know the paragraph describes the
state at the eighth group, not the ninth.

### 5c. The mirror problem: five engine commands are registered *from the shell*
"""

text = text.replace(ANCHOR, section, 1)

with open(PATH, "w", encoding="utf-8", newline="") as f:
    f.write(text)

print("ok")
