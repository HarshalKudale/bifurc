"""
P6 reduction, first handler group — the `tlsHandlers.ts` deletion's doc and wiring edits.

The file itself is deleted by `rm` + `git add` (**not** `git rm`, which on this repo deletes the
whole `src/` tree — see TRAPS.md). This script handles everything that *named* it, because a comment
pointing at a deleted file is the same class of defect as a doc quoting a stale count.

Every replacement asserts **exactly one** match, so a silent no-op is impossible: if an anchor has
moved, the script fails loudly instead of leaving a half-edited file behind.
"""

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def patch(rel: str, edits: list[tuple[str, str]]) -> None:
    path = ROOT / rel
    text = path.read_text(encoding="utf-8")
    for before, after in edits:
        count = text.count(before)
        if count != 1:
            sys.exit(f"{rel}: expected exactly 1 match, found {count}:\n---\n{before[:200]}\n---")
        text = text.replace(before, after)
    path.write_text(text, encoding="utf-8")
    print(f"patched {rel} ({len(edits)} edits)")


# ── 1. the wiring itself ─────────────────────────────────────────────────────────────────────────
patch(
    "src/ipc/handlers.ts",
    [
        (
            'import { registerTlsHandlers } from "@/ipc/handlers/tlsHandlers";\n'
            'import { registerSyncHandlers } from "@/ipc/handlers/syncHandlers";\n',
            'import { registerSyncHandlers } from "@/ipc/handlers/syncHandlers";\n',
        ),
        (
            "  // P3 work item 5: `tls.generate` / `tls.certStatus` / `tls.removeCert`. Moved here from\n"
            "  // `handlers/tlsHandlers.ts`, which was serving engine commands from the client half — it works\n"
            "  // only while the two are the same process. `tls:installCA` did **not** come with them: the host\n"
            "  // trust store is CLIENT-classified and stays in `clientHandlers.ts`.\n"
            "  registerCertCommands(commandRegistry);\n",
            "  // P3 work item 5: `tls.generate` / `tls.certStatus` / `tls.removeCert`. Moved here from\n"
            "  // `handlers/tlsHandlers.ts`, which was serving engine commands from the client half — it works\n"
            "  // only while the two are the same process. `tls:installCA` did **not** come with them: the host\n"
            "  // trust store is CLIENT-classified and stays in `clientHandlers.ts`.\n"
            "  //\n"
            "  // P6 reduction, first group (2026-09-21): the six `tls:*` channels are deleted, so the file is\n"
            "  // gone. It went last of its class rather than first because three of its bodies were **not**\n"
            "  // passthroughs — they appended the CA paths the engine refuses to report, and the client twins\n"
            "  // had dropped them (`plan/07` §5b-undecies). Deleting the channels before fixing the twins\n"
            "  // would have destroyed the only correct implementation of a live behaviour.\n"
            "  registerCertCommands(commandRegistry);\n",
        ),
        (
            "  // Register all categorized handlers\n"
            "  registerTlsHandlers();\n"
            "  registerSyncHandlers();\n",
            "  // Register all categorized handlers\n"
            "  registerSyncHandlers();\n",
        ),
    ],
)

# ── 2. the two docblocks that cited the file as the live implementation ──────────────────────────
patch(
    "src/ipc/certLifecycle.ts",
    [
        (
            " * Extracted 2026-09-20 so the two channels that need it cannot drift apart. Before the extraction the\n"
            " * composition lived only inside `tlsHandlers.ts`'s `tls:certStatus` channel, which is why the routed\n"
            " * client twin could deliver the engine half alone and nothing pointed at the three missing fields:\n",
            " * Extracted 2026-09-20 so the two channels that need it cannot drift apart. Before the extraction the\n"
            " * composition lived only inside the `tls:certStatus` channel (`tlsHandlers.ts`, deleted 2026-09-21 with\n"
            " * the rest of the reduction's first group), which is why the routed\n"
            " * client twin could deliver the engine half alone and nothing pointed at the three missing fields:\n",
        ),
        (
            " * (`plan/07` §5b-bis), and the legacy channel delegates here too, so there is one implementation.\n"
            " *\n"
            " * `certPath` / `keyPath` are computed here rather than returned by the engine — `tlsHandlers.ts`\n"
            " * carries the full argument (category 3 in `plan/04` item 6; the engine deliberately reports no path\n"
            " * since `plan/protocol-changes.md` 2026-09-17, and P7 is where it stops being a path).\n",
            " * (`plan/07` §5b-bis), and the legacy channel delegated here too, so there is one implementation.\n"
            " *\n"
            " * `certPath` / `keyPath` are computed here rather than returned by the engine (category 3 in `plan/04`\n"
            " * item 6; the engine deliberately reports no path since `plan/protocol-changes.md` 2026-09-17, and P7\n"
            " * is where it stops being a path). **This hook is now the only source of those two paths**, and the\n"
            " * client's `tlsGenerate` / `tlsImportCert` / `tlsImportKey` shims read them from here — the same\n"
            " * role the deleted `tlsHandlers.ts` played, which is why §5b-undecies' criterion had to widen.\n",
        ),
    ],
)

patch(
    "src/ipc/handlers/clientHandlers.ts",
    [
        (
            " * `tls:installCA` is the one exception worth calling out: it lives here (not in\n"
            " * `tlsHandlers.ts`) because it mutates the *host's* trust store — a host-scoped, client-local\n"
            " * mutation per `File_Ops_Protocol.md` §6, unlike the rest of `tlsHandlers.ts` which reads/writes\n"
            " * the engine's own CA files. P3 work item 5 made that split explicit: this handler now delegates to\n",
            " * `tls:installCA` is the one exception worth calling out: it lives here (not in the since-deleted\n"
            " * `tlsHandlers.ts`) because it mutates the *host's* trust store — a host-scoped, client-local\n"
            " * mutation per `File_Ops_Protocol.md` §6, unlike the rest of that file, which read and wrote the\n"
            " * engine's own CA files. P3 work item 5 made that split explicit: this handler now delegates to\n",
        ),
        (
            "   * the Settings panel's \"remove certificate\" button. `tlsHandlers.ts`'s `tls:certStatus` /\n"
            "   * `tls:removeCert` channels are the superseded twins — same bodies, unreachable now that\n"
            "   * `window.api` is `{ ...client }`.\n",
            "   * the Settings panel's \"remove certificate\" button. The superseded twins — `tls:certStatus` and\n"
            "   * `tls:removeCert` in `tlsHandlers.ts` — were unreachable once `window.api` became\n"
            "   * `{ ...client }`, and were deleted with that file on 2026-09-21.\n",
        ),
    ],
)

# ── 3. the acceptance test that pointed at the file for its counter-example ──────────────────────
patch(
    "tests/integration/fileOps.integration.test.ts",
    [
        (
            " * renderer convenience — the tempting regression, since `tls:importCert` still has to return one\n"
            " * (see `src/ipc/handlers/tlsHandlers.ts`) — fails here.\n",
            " * renderer convenience — the tempting regression, since `tlsImportCert` still has to return one, as\n"
            " * the client's shim does (`packages/client/src/index.ts`) — fails here. The shim's `path` is the\n"
            " * **client's** fact and never crosses the wire, which is the distinction this assertion protects.\n",
        ),
    ],
)

print("all patches applied")
