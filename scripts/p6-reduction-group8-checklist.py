"""Group 8 doc edit, pair 3 — append the eighth group's progress record to `plan/13-checklist.md`.

Anchored on the seventh group's record's final sentence, and guarded on the **new** text only: an
append-shaped edit is not idempotent under a `found_old == 0` guard (group 7's lesson).
"""
import sys

PATH = "plan/13-checklist.md"
ANCHOR = (
    "      two groups still to go**, which makes the remainder the reduction's highest-risk part rather than\r\n"
    "      its tail.\r\n"
)

with open(PATH, "r", encoding="utf-8", newline="") as f:
    text = f.read()

if "eighth group" in text:
    print("skip: already present")
    sys.exit(0)

found = text.count(ANCHOR)
if found != 1:
    print(f"FAIL: anchor appears {found} times, expected 1")
    sys.exit(1)

record = (
    "      **Progress 2026-09-21, eighth group — `runnerHandlers.ts` is deleted, and the criterion finally\r\n"
    "      has a channel that owns a step** (`plan/07` §5b-novendecies). All six `runner:*` channels are gone\r\n"
    "      and the file is **deleted**, along with its import and call in `src/ipc/handlers.ts`; `src/` goes\r\n"
    "      19 → 18 files. **This is the first group where the criterion had real work to do**, and the three\r\n"
    "      shapes §5b-septendecies named were all checked for rather than one being assumed:\r\n"
    "      `runner:getHistory` / `runner:listFolderIds` were **pure delegates** (one-line\r\n"
    "      `commandRegistry.invoke()`, nothing owned); `runner:saveReport` + the `runner:saveConfig` /\r\n"
    "      `runner:loadConfig` pair were **self-contained duplicates** — not delegates, and the dangerous\r\n"
    "      kind, because `expectedChannels` never listed them *and* `runnerStorage.integration.test.ts` drove\r\n"
    "      the shell copies, so the suite would have stayed green with the engine halves rotted;\r\n"
    "      `runner:exportReport` was the **only channel owning a step the engine does not perform** — the\r\n"
    "      dialog, the format-from-extension decision, and the artifact write — and its twin is\r\n"
    "      `packages/client/src/index.ts:927` (`exportRunnerReport`). That equivalence was **shown, not\r\n"
    "      assumed**: same title, same `defaultName`, same two filters, same `?? \"collection\"` / `??\r\n"
    "      Date.now()` fallbacks, same `toISOString().replace(/[:.]/g, \"-\")`, same `.endsWith(\".json\")`\r\n"
    "      format test, same bare `{ok:false}` on cancel. The client's two regression cases in\r\n"
    "      `packages/client/tests/client.test.ts` were **already there**, so nothing had to be added to\r\n"
    "      prove it. **Delta 0, with a third cause**: not a re-point, not \"never had cases\", but *both at\r\n"
    "      once plus a duplicated implementation* — which is why the count separates none of the three and\r\n"
    "      the per-file read is the only way through. The suite was **re-pointed and its redundant fifth\r\n"
    "      block folded**: the verbatim-write assertion and the corrupt-config case moved into the\r\n"
    "      `runner.saveConfig / runner.loadConfig` block (they test the *store's* fidelity, not the\r\n"
    "      routing), the cross-implementation case was deleted as vacuous, the export block moved to the\r\n"
    "      engine half with a comment naming the client's dialog-first cases, and the registration check\r\n"
    "      became `isRegistered(command)` over the namespace. **Two cases are new**, which is why the suite\r\n"
    "      goes **21 → 22**: the corrupt-run case (\"drops only the corrupt entry, not the whole folder\") —\r\n"
    "      the nested `try` `runnerCommands.ts` calls load-bearing, which nothing asserted — and a split of\r\n"
    "      the malformed-report case, because a schema-invalid payload throws **at the registry boundary**\r\n"
    "      while a payload that reaches the body resolves `{ok:false}`; the old test asserted the second\r\n"
    "      for the first's cause. **Gate: 101 files / 2,462 / 2,421 / 40 / 1** — the +1 test over the\r\n"
    "      seventh group's 2,461 is the new corrupt-run case and was present on both sides of the\r\n"
    "      measurement, so the delta *is* 0; the 1 is the standing environmental `soap.execute`. (One run\r\n"
    "      reported 3 failures — two `EBUSY: rmdir` temp-dir teardown races under load plus the `gitStore`\r\n"
    "      timeout flake group 7 already named. Both dismissed **with evidence**: `tests/sync/gitOps.test.ts`\r\n"
    "      runs 4 passed (4) in isolation, and the re-measured full run is back to 1.) The spike's fixture\r\n"
    "      did **not** grow (its sixth non-growth, and the first that is a *consequence* of the group rather\r\n"
    "      than incidental to it): it drives ten channels, none a `runner:*`, so it stays at **10 of 10** —\r\n"
    "      its surrounding comment was updated because it made an inventory claim and the inventory moved.\r\n"
    "      **Inventory: 22 literal channels in 3 files = 10 live + 12 dead**, so **12 dead in one file**\r\n"
    "      remain: `importExportHandlers.ts`'s four, the one file §5b-decies classified as **not** delegates\r\n"
    "      — the P3 client half (dialog + bytes + the `pendingUploads` map), filed `transport`/`shim` in\r\n"
    "      `surface.ts`. Dialog-owner subset **now 2** (`importExport:export`, `importExport:preflight`,\r\n"
    "      re-derived with `grep -n \"dialog\\.\"`), so the reduction ends on its highest-risk file rather\r\n"
    "      than its easiest — the right order, because the last group is the one where \"the twin is\r\n"
    "      equivalent\" must be demonstrated and there is no group after it to catch a mistake.\r\n"
)

with open(PATH, "w", encoding="utf-8", newline="") as f:
    f.write(text.replace(ANCHOR, ANCHOR + record, 1))

print("ok")
