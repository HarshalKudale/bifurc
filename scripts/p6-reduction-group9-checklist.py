"""Group 9 (importExportHandlers.ts, the reduction's last group): append the progress
record to plan/13-checklist.md.

One script, one edit pair. plan/13 is CRLF, so every inserted line ends \r\n.

The anchor is the closing sentence of the eighth-group record. It is asserted to match
exactly once, and the inserted text is asserted absent before the write, so a re-run is
a no-op rather than a duplicate.
"""

from pathlib import Path

CHECKLIST = Path("plan/13-checklist.md")

ANCHOR = (
    "      than its easiest \u2014 the right order, because the last group is the one where \"the twin is\r\n"
    "      equivalent\" must be demonstrated and there is no group after it to catch a mistake.\r\n"
)

RECORD = (
    "      **Progress 2026-09-21, ninth and last group \u2014 `importExportHandlers.ts` is deleted, the\r\n"
    "      reduction is closed, and every remaining channel is a keeper** (`plan/07` \u00a75b-vicendecies).\r\n"
    "      All four `importExport:*` channels are gone and the file is **deleted**, along with its import\r\n"
    "      and call in `src/ipc/handlers.ts`; `src/` goes 18 \u2192 17 `.ts` files. **Delta \u22124, and it is the\r\n"
    "      reduction's only genuine ratchet shrink**: the four names *were* in `handlers.test.ts`'s\r\n"
    "      `expectedChannels`, so four generated `it(\"registers a handler for \u2026\")` cases went with them\r\n"
    "      (110 \u2192 106). Every earlier delta had to be *explained* \u2014 a re-point, a generated-list shrink\r\n"
    "      from a different mechanism, a suite re-pointed wholesale; this one needs no explanation beyond\r\n"
    "      the list, which is what makes it the cleanest measurement the reduction produced, and it\r\n"
    "      arrives *last* because the file easiest to see was the hardest to delete safely. **This is the\r\n"
    "      group the whole criterion was built for.** \u00a75b-decies classified the file as the only one\r\n"
    "      whose bodies are not delegates \u2014 the P3 client half (dialog + bytes + the `pendingUploads`\r\n"
    "      map). Per channel:\r\n"
    "      \u2022 `importExport:formats` \u2014 **the judgement \u00a75b-novendecies flagged, and it resolves to\r\n"
    "      dead.** Its body is `call(\"export.formats\", {})` and\r\n"
    "      `packages/engine/src/importExport/commands.ts:59` registers that command, so it owns nothing.\r\n"
    "      **The reason it looked ambiguous is that it is the only one of the four whose engine command\r\n"
    "      returns data rather than an artifact** \u2014 a cheap read has the shape of a client-local query\r\n"
    "      and of a delegate at once. It is decided by whether a protocol command exists, not by the cost\r\n"
    "      of the call.\r\n"
    "      \u2022 `importExport:export` / `:preflight` \u2014 **genuine step-owners and the reduction's last two\r\n"
    "      dialog-first egresses.** `:export` = dialog \u2192 `export.create` (filename only crosses) \u2192\r\n"
    "      `writeArtifact`; `:preflight` = dialog \u2192 read + size-cap \u2192 `blob.put` \u2192 `import.preflight`\r\n"
    "      \u2192 **remember the upload**, the last step a reader is most likely to miss because it is not a\r\n"
    "      call. Twins `index.ts:585` / `:626`.\r\n"
    "      \u2022 `importExport:import` \u2014 a step-owner that *reads* what preflight wrote: `uploadFor` (the map\r\n"
    "      lookup with a re-upload fallback for an app restart between the steps) \u2192 `import.commit` \u2192 a\r\n"
    "      `finally` that forgets the upload and releases the blob. Twin `index.ts:653`.\r\n"
    "      **The `pendingUploads` map was the specific thing that could have been lost silently**, because\r\n"
    "      it is the only state P3 added to the shell and it is *cross-channel*: `:preflight` writes it,\r\n"
    "      `:import` reads it, so deleting either alone breaks the pair in a way no single-channel test\r\n"
    "      sees. Verified rather than assumed \u2014 `packages/client/src/index.ts:493` declares **its own**\r\n"
    "      map with `rememberUpload`/`forgetUpload`/`uploadOnce` at `:499`/`:512`, same key, same\r\n"
    "      replace-and-release-the-displaced-entry behaviour. The question is not \"is the channel dead\"\r\n"
    "      but \"does the twin carry the **state**, not just the calls\". The client's own comment at\r\n"
    "      `index.ts:354` **named this deletion before it happened** \u2014 *\"That file is now dead \u2026 it goes\r\n"
    "      at step 3's final deletion, at which point this becomes the only copy\"* \u2014 which is the correct\r\n"
    "      order, and every group that went wrong (\u00a75b-undecies' `tlsGenerate`, \u00a75b-septendecies'\r\n"
    "      `entity:load`) went wrong by deleting before the twin was complete. **Suite cost: one list.** No\r\n"
    "      suite drives an `importExport:*` channel by name \u2014 the two files that mention the path do so in\r\n"
    "      comments only \u2014 and `expectedChannels` lost its four names (the mock block above it, which\r\n"
    "      stubbed five functions nothing called once those handlers went, was **removed**; 106 pass\r\n"
    "      without it). The equivalence rests on `packages/client/tests/client.test.ts`: two `describe`\r\n"
    "      blocks and 22 references, carrying exactly the **ordering and count** assertions the shell file\r\n"
    "      consisted of \u2014 and it already carried them, so nothing had to be added. **Gate: 101 files /\r\n"
    "      2,458 / 2,417 / 40 / 1**; the 1 is the standing environmental `soap.execute`. **Inventory:\r\n"
    "      18 `ipcMain.handle(` call sites, all keepers** (17 in `clientHandlers.ts`, 1 client-local in\r\n"
    "      `systemHandlers.ts`), down from 21; **zero dead remain**, and **the dialog-owner subset is\r\n"
    "      empty**. `registerIpcHandlers()` is reduced, **not** deleted, and that is now a *finished state*\r\n"
    "      rather than a pending one \u2014 which makes `plan/07` work item 6's rollback flag weaker than it\r\n"
    "      reads, since there is no longer a legacy path to revert to. The spike's fixture did **not** grow\r\n"
    "      (the seventh non-growth; it drives no `importExport:*` channel) and stays at **10 of 10**; its\r\n"
    "      surrounding paragraph is now **historical** \u2014 it describes the state at the eighth group \u2014 and\r\n"
    "      is deliberately left correct-by-context rather than edited twice in one day.\r\n"
)


def main() -> None:
    text = CHECKLIST.read_text(encoding="utf-8", newline="")

    found_anchor = text.count(ANCHOR)
    assert found_anchor == 1, f"anchor matched {found_anchor} times, expected exactly 1"

    # Idempotence: test the *new* text, never the anchor (the anchor stays after the write).
    assert RECORD not in text, "the ninth-group record is already present"
    assert "5b-vicendecies" not in text or True  # referenced by the record itself

    text = text.replace(ANCHOR, ANCHOR + RECORD, 1)
    CHECKLIST.write_text(text, encoding="utf-8", newline="")

    raw = CHECKLIST.read_bytes()
    crlf = raw.count(b"\r\n")
    bare = raw.count(b"\n") - crlf
    print(f"ok  plan/13-checklist.md: CRLF={crlf} bareLF={bare}")
    assert bare == 0, "the file picked up bare LF lines"


if __name__ == "__main__":
    main()
