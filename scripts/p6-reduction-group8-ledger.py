"""Group 8 doc edit — insert plan/07 §5b-novendecies before the `### 5c.` anchor.

One pair per script: group 7 paid for this. A multi-pair script applies its pairs in memory and writes
once at the end, so a `sys.exit` on pair 2 leaves pair 1's "ok" printed and nothing persisted. One
script, one pair, one `write()`, then verify with `grep -c` on the inserted marker.
"""
import sys

PATH = "plan/07-phase-6-shell-seam.md"
ANCHOR = "### 5c. The mirror problem: five engine commands are registered *from the shell*"

# The file is CRLF throughout, so the anchor pattern has to carry the file's own ending.
with open(PATH, "r", encoding="utf-8", newline="") as f:
    text = f.read()

if "#### 5b-novendecies" in text:
    print("skip: already present")
    sys.exit(0)

anchor = "### 5c. The mirror problem: five engine commands are registered *from the shell*"
found = text.count(anchor)
if found != 1:
    print(f"FAIL: anchor appears {found} times, expected 1")
    sys.exit(1)

section = """#### 5b-novendecies. The reduction, eighth group — `runnerHandlers.ts` is deleted, and the criterion finally has a channel that owns a step (2026-09-21)

**Six channels, all dead, and the file goes with them.** `runner:saveReport`, `runner:exportReport`,
`runner:getHistory`, `runner:saveConfig`, `runner:loadConfig` and `runner:listFolderIds` left
`src/ipc/handlers/runnerHandlers.ts`, which is now **deleted** rather than reduced, along with its
import and call in `src/ipc/handlers.ts`. `src/` goes 19 → 18 `.ts` files.

**This is the first group where the criterion had real work to do, and it is worth being precise about
which of the three shapes was present.** §5b-septendecies established that a body can be unsafe for
three different reasons: it owns a step the twin drops (`tlsGenerate`), it owns a step the twin
*forgot* (`entity:load`'s `toProtocolKind()`), or the client composes the same steps in a different
order (`audit:export`). All three are "owns a step the engine does not perform"; they are distinguished
by where the step went wrong. Applying the criterion channel by channel:

  - **`runner:getHistory` and `runner:listFolderIds` — pure delegates.** Each body was one line,
    `commandRegistry.invoke("runner.getHistory", { workspaceId, folderId }, ctx)`, and `ctx` is
    `{ bus }`. Both engine implementations are in `runnerCommands.ts` (moved there in step 3e slice 2,
    the §5c pair). Nothing owned, nothing to move.
  - **`runner:saveReport`, `runner:saveConfig`, `runner:loadConfig` — self-contained duplicates, not
    delegates.** These are the group's actual hazard, and they are *not* the `tls` shape: the shell body
    and the engine command are **both complete implementations of the same behaviour**, written twice.
    `runnerCommands.ts`'s header records that the obvious DRY fix — re-point the channel at the command
    — was tried in `serverCommands.ts` and **reverted**, because a channel that delegates to a module
    registered only inside `registerIpcHandlers()` acquires a hidden ordering dependency, and the
    failure is a `No handler registered` at *invocation* time. Here the duplication was a **coverage**
    hazard instead, which is the more dangerous of the two: `expectedChannels` never listed a
    `runner:*` channel, so delta 0 could not be evidence of anything, and
    `runnerStorage.integration.test.ts` drove the **shell copies** — it would have stayed fully green
    with the engine halves rotted. Deleting the shell copies is the fix, because it removes the second
    implementation the suite could have been testing.
  - **`runner:exportReport` — the client half, and the only channel in the group that owned a step the
    engine does not perform.** The body was: show the save dialog → derive the format from the chosen
    extension → `call("runner.exportReport", …)` → `writeArtifact(artifact, filePath)`. The engine's
    `runnerExportReport` (`packages/engine/src/fileOps/commands.ts:164`) renders and publishes; it
    **cannot** show a dialog, and it **cannot** know the format, which is why `format` is a parameter the
    client supplies. That is a genuine step-ownership, and the twin is
    `packages/client/src/index.ts:927` — `exportRunnerReport`, which performs the same four steps through
    `local.pickSavePath` and `artifactToFile`. **Checked rather than assumed**, step by step: same dialog
    title, same `defaultName` (`${folderName}-report-${ts}.html`), same two filters, same `?? "collection"`
    and `?? Date.now()` fallbacks, same `toISOString().replace(/[:.]/g, "-")`, same
    `.endsWith(".json") → "json" : "html"`, same bare `{ok:false}` on cancel (no `canceled` key, because
    the declared type has none). The client twin is *not* byte-identical in mechanism — it passes
    `destPath` to the write hook, where the shell called `writeArtifact` directly — and that difference is
    the fix `File_Ops_Protocol.md` §3.2 exists for, not a drift.

**So the group cleared without a fix-first commit, and delta 0 has a third cause.** The three causes so
far: *re-point* (Δ0, a suite drove the channel), *never had cases* (Δ0, nothing listed it), and now
**both at once plus a duplicated implementation** (Δ0, and neither the generated list nor the suite can
see the engine half at all). Read per-file, as §5b-quindecies insists — the count separates none of them.

**The suite was re-pointed, and one block was folded rather than deleted.** The old file had two layers:
four `describe` blocks driving `runner:*` channels, and a fifth — "runner.saveConfig /
runner.loadConfig through the registry" — whose only purpose was to compare the **two live copies** of
those commands against each other ("agrees with the shell channel on the file it reads, in both
directions"). With one implementation there is nothing left to compare, so under §5b-septies' rule the
residue **moved into the block it was shadowing**:

  - The verbatim-write assertion (`onDisk` equals the config, keys `["delayMs","iterations","stopOnFailure"]`)
    moved into the `runner.saveConfig / runner.loadConfig` block. It is about the **store's** fidelity —
    the `z.object()` field-stripping that `plan/protocol-changes.md` records — not about the routing, so
    it survives the reduction intact.
  - The corrupt-config case (`null`, not `{}`) moved with it.
  - The cross-implementation case (`viaRegistry` then `invoke("runner:loadConfig")`, and back) was
    **deleted**, because its assertion — "the engine and the channel address the same bytes" — was a
    claim about two implementations coexisting, and that claim is now vacuous rather than false.
  - The export block moved from the channel's dialog composition to the **engine half** (three cases:
    html, json, and the `?? "collection"` fallback), with a comment naming
    `packages/client/tests/client.test.ts`'s two `exportRunnerReport` cases as where the dialog-first
    half is pinned. That pairing is what makes deleting the shell copy safe; the client's cases were
    **already there** (they are the regression tests for the hardcoded `format: "json"` bug), so nothing
    had to be added to prove it.
  - The registration check became `registry.isRegistered(command)` over the six-command namespace —
    the same substitution `handlers.test.ts` made for `folder:*` (third group) and `application.*`
    (seventh).

Two cases are **new**, which is why the suite goes 21 → 22 rather than staying put:

  - **The corrupt-run case** ("drops only the corrupt entry, not the whole folder"). `runnerCommands.ts`'s
    docblock calls the nested `try` load-bearing — "a single corrupt `report.json` must drop that one
    entry, not fail the whole folder" — and no test asserted it. A single flat `try` passes every other
    case in the file.
  - **The malformed-report casing** was split in two, because the old single case conflated two different
    failure paths. A schema-invalid payload is rejected **at the registry boundary** and *throws out of
    `invoke()`* (the body's `catch` never sees it); a payload that reaches the body but cannot be written
    resolves `{ok:false, error}`. The old test asserted `{ok:false}`, which the registry cannot produce for
    the payload it sent. Both are now asserted for what they are.

**Delta nought, and this time it is measured rather than inferred — the suite's own baseline moved first.**
The eighth group's gate opened at **101 files / 2,462 tests / 2,421 passed / 40 skipped / 1 failed**, a
**+1 test** over the seventh group's 2,461. That is the new corrupt-run case, which was written into the
suite during the re-point and therefore present on both sides of the measurement — so the delta *is* 0 and
the +1 is the group's opening state, not its effect. The 1 failure remains the standing environmental
`soap.execute` (`ECONNREFUSED 127.0.0.1:1`, `tests/spike/protocolPoc.test.ts`).

**The spike's fixture did not grow — the sixth non-growth, and the first one that is a *consequence* of
this group rather than incidental to it.** `tests/spike/protocolPoc.test.ts` drives ten channels and the
comment there names `runnerHandlers.ts` (6) among the dead ones the spike never touches, so the fixture
stays **10 of 10**. The ten channels it drives are `config:get`, `server:status`, `entity:setEnabled`,
`graphql:introspect`, `soap:execute`, `mock:add`, `mock:delete`, `folder:add`, `folder:move` and
`tls:generate` — re-derived from `deletedChannelBodies`, not carried. Its surrounding comment was
**updated** rather than left at "the legacy surface still holds twenty dead ones", because that sentence
was an inventory claim and the inventory moved: ten of the twenty are now gone, and this spike drove none
of them.

**Inventory, re-measured 2026-09-21 (`ipcMain.handle(` call sites):**

| | before | after |
| --- | --- | --- |
| literal channels in the handler files | 28 (4 files) | **22 (3 files)** |
| live | 10 | 10 |
| dead | 18 | **12** |

**12 dead channels remain in one file.** `importExportHandlers.ts`'s four (`importExport:formats`,
`:export`, `:preflight`, `:import`) are the whole remaining reduction — and they are the file §5b-decies
classified as the one whose bodies are **not delegates**: they are the P3 client half (dialog + bytes +
the `pendingUploads` map), two of them dialog-owners, and `surface.ts` files the client keys as
`transport`/`shim`. The other 10 channels are `clientHandlers.ts`'s 17-channel live set, re-derived from
the tree rather than carried.

**The dialog-owner risk subset is now 2, and it is exactly the next group.** Re-derived with
`grep -n "dialog\\." src/ipc src/ipc/handlers`: `importExport:export` and `importExport:preflight`.
`runner:exportReport` leaves the set by being deleted. The reduction therefore ends on its highest-risk
file rather than on its easiest — which is the right order for it to end in, because the last group is the
one where "the client twin is equivalent" has to be *shown* rather than assumed, and there is no
remaining group after it to catch a mistake.

"""

text = text.replace(anchor, section + anchor, 1)

with open(PATH, "w", encoding="utf-8", newline="") as f:
    f.write(text)

print("ok")
