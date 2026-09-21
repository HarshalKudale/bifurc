"""Group 9 / reduction closure: append the day-log entry to 2026-09-21.md.

The file is LF and append-only. The entry is asserted absent first, so a re-run is a
no-op rather than a duplicate.
"""

from pathlib import Path

P = Path(".workbuddy-ai/memory/2026-09-21.md")

MARKER = "## 17:40 \u2014 P6 reduction, ninth and last group: `importExportHandlers.ts` deleted"

ENTRY = """
## 17:40 \u2014 P6 reduction, ninth and last group: `importExportHandlers.ts` deleted

**The reduction is closed.** Four channels \u2014 `importExport:formats`, `:export`, `:preflight`, `:import` \u2014 left
`src/ipc/importExportHandlers.ts`, the file is **deleted** with its import and call in `src/ipc/handlers.ts`,
and `src/` goes **18 \u2192 17** `.ts` files. Commit `d7ab9ad`.

**Delta \u22124 and it is the only genuine ratchet shrink the reduction produced.** The four names *were* in
`tests/ipc/handlers.test.ts`'s `expectedChannels`, so four generated `it("registers a handler for \u2026")` cases
went with them: **110 \u2192 106**. Every earlier delta (0 four times, \u22125, \u22123, \u22128, \u221225) had to be *explained* \u2014 a
re-point, a generated-list shrink from another mechanism, a suite re-pointed wholesale. This one needs no
explanation beyond the list, which is what makes it the cleanest measurement of the nine. It arrives *last*
because the file easiest to see was the hardest to delete safely.

**This is the group the criterion was built for.** §5b-decies classified the file as the only one whose bodies
are **not** delegates; it is the P3 client half (dialog + bytes + the `pendingUploads` map). Per channel:

- **`importExport:formats` \u2014 the judgement §5b-novendecies flagged, and it resolves to dead.** Body is
  `call("export.formats", {})`; `packages/engine/src/importExport/commands.ts:59` registers that command, so it
  owns nothing. It *looked* ambiguous because it is the only one of the four whose engine command returns **data
  rather than an artifact** \u2014 a cheap read has the shape of a client-local query and of a delegate at once.
  Decided by whether a protocol command exists, not by the cost of the call.
- **`:export` / `:preflight` \u2014 the reduction's last two dialog-first step-owners.** `:export` = dialog \u2192
  `export.create` (filename only crosses, never the path) \u2192 `writeArtifact`. `:preflight` = dialog \u2192 read +
  size-cap \u2192 `blob.put` \u2192 `import.preflight` \u2192 **remember the upload** \u2014 the step a reader is most likely to
  miss, because it is not a call. Twins `packages/client/src/index.ts:585` / `:626`.
- **`:import` \u2014 a step-owner that *reads* what preflight wrote.** `uploadFor` (the map lookup with a re-upload
  fallback for an app restart between the steps) \u2192 `import.commit` \u2192 a `finally` that forgets the upload and
  releases the blob. Twin `index.ts:653`, same order.

**The `pendingUploads` map was the specific thing that could have been lost silently.** It is the only state P3
added to the shell and it is **cross-channel**: `:preflight` writes it, `:import` reads it, so deleting either
alone breaks the pair in a way no single-channel test sees. Verified rather than assumed \u2014
`packages/client/src/index.ts:493` declares **its own** map with `rememberUpload`/`forgetUpload`/`uploadOnce` at
`:499`/`:512`, same key, same replace-and-release-the-displaced-entry behaviour. The `tls` group's lesson,
applied in the direction that avoids it: the question is not "is the channel dead" but **"does the twin carry
the STATE, not just the calls"**.

**The client's own comment named this deletion before it happened.** `index.ts:354`: *"A copy of the shell's
`kindLabel` in `src/ipc/importExportHandlers.ts`. That file is now dead \u2026 it goes at step 3's final deletion,
at which point this becomes the only copy."* That is the **correct order** \u2014 twin written first, its divergence
documented, shell file retired last \u2014 and every group that went wrong (`tlsGenerate`, `entity:load`) went wrong
by deleting before the twin was complete.

**Suite cost: one list.** No suite drives an `importExport:*` channel by name; the two files that mention the
path do so in **comments only** (`importExportHarness.ts:13`, `importExportBlob.integration.test.ts:179`) and
both were re-pointed at the client. `expectedChannels` lost its four names and the now-orphaned
`vi.mock("@bifurc/engine/importExport/registry", \u2026)` block was **removed** \u2014 it stubbed five functions nothing
in the graph called once those handlers went; verified by the suite passing 106 without it. The equivalence
rests on `packages/client/tests/client.test.ts`: two `describe` blocks, 22 references, carrying exactly the
**ordering and count** assertions the shell file consisted of (\u2026 *"asks where BEFORE it asks the engine to
render"*, *"does not open a second dialog"*, *"reuses the path and the blob \u2014 no second dialog, no second
upload"*) \u2014 and it already carried them, so nothing had to be added to prove the twin.

**Gate: 101 files / 2,458 / 2,417 passed / 40 skipped / 1 failed**; the 1 is the standing environmental
`soap.execute` (`ECONNREFUSED 127.0.0.1:1`). (A first run reported 5 failures \u2014 load pollution from an
overlapping typecheck, not the edit; the clean re-run is the measurement quoted.)

**Inventory, re-derived from the tree: 18 `ipcMain.handle(` call sites, all keepers** \u2014 17 in
`clientHandlers.ts` + 1 client-local in `systemHandlers.ts` (`app:checkUpdate`); down from 21. Counting
`rpcBridge.ts`'s constant-bridged channel gives the other reading. **Zero dead channels remain; every remaining
one is a keeper, and the dialog-owner subset went 3 \u2192 2 \u2192 empty.** `registerIpcHandlers()` is reduced, **not**
deleted, and that is now a *finished state* \u2014 which makes `plan/07` work item 6's rollback flag weaker than it
reads, since there is no longer a legacy path to revert to.

**The spike's fixture did not grow** \u2014 the seventh non-growth, and it drives no `importExport:*` channel, so it
stays at **10 of 10**. Its surrounding paragraph is now **historical** (it describes the state at the eighth
group); left correct-by-context rather than edited a second time in one day.

**Tooling: one script per pair again, and the CRLF trap is now routine.** Four scripts \u2014 ledger insert,
checklist append, memory rewrite, this log \u2014 each with an exactly-one-match guard and, for the append-shaped
ones, the guard **tested against the new text rather than the anchor** (which stays after the write \u2014 the trap
that makes a re-run duplicate). The bare-`\\n` description again failed on 0 matches against the CRLF comment
files and again needed `\\r\\n`. The single-quoted-heredoc whole-file rewrite of `MEMORY.md` and the suffix-append
form used for the day log both worked **when the heredoc is quoted** \u2014 which with the CRLF pair below is the
whole trap family: **a rewrite needs the file's ending carried in the inserted text, an append only needs the
inserted text's own ending right, and neither may be inferred from the other.**

**Card: the cross-pair `ok` problem, now understood.** Group 9's first ledger script printed `ok` per pair but
wrote once at the end, so the printed verdicts were not per-commit verdicts \u2014 the reason for the one-pair-per-
script rule. Related and worth keeping: **an edit script that cannot run is not idempotent in the way the guard
assumes**, so a blocked regeneration reads as duplication and pushes the next reader toward hand-editing
tracked files, which is exactly what this session twice had to repair after the fact. Re-running the script is
the correct response to a failed insert; hand-editing the section is not.
"""


def main() -> None:
    text = P.read_text(encoding="utf-8", newline="")
    assert MARKER not in text, "the ninth-group day-log entry is already present"
    assert not text.endswith("\n\n"), "unexpected trailing blank line; refusing to guess the join"
    out = text.rstrip("\n") + "\n" + ENTRY
    P.write_text(out, encoding="utf-8", newline="")

    raw = P.read_bytes()
    assert raw.count(b"\r\n") == 0, "the day log is LF; it picked up CRLF"
    print(f"ok 2026-09-21.md: {len(raw)} bytes, {raw.count(chr(10).encode())} lines, "
          f"{raw.count(MARKER.encode())} occurrence(s) of the marker")


if __name__ == "__main__":
    main()
