"""
P3 work item 6 — convert the format-level import/export tests to the content-shaped interface.

The interface change (paths -> content) is mechanical at the call site:

    exporter.run(TEST_WS, file)            -> exportToFile(TEST_WS, exporter, file)
    importer.preflight(TEST_WS, file)      -> preflightFile(importer, TEST_WS, file)
    importer.run(TEST_WS, file, "override")-> importFile(importer, TEST_WS, file, "override")

so it is done by script rather than by hand, with the replacement counts asserted. A silent
no-match would leave a call site on the removed signature, and tests are NOT typechecked
(`tsconfig.json` has `include: ["src/**/*"]`), so the breakage would only appear at runtime.

Run:  python scripts/p3-convert-importexport-tests.py [--check]
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TESTS = ROOT / "tests" / "integration"

HARNESS_IMPORT = 'import { exportToFile, importFile, preflightFile } from "./importExportHarness";\n'

# A receiver is either a registry lookup with flat args, or a local const. The `!` is optional
# because some tests hold the result of `getExporter(...)` directly.
RECV_EXP = r'(?:getExporter\([^()]*\)|exporter)!?'
RECV_IMP = r'(?:getImporter\([^()]*\)|importer)!?'

# Args are flat at this point (the one nested case is handled explicitly below), so a
# non-greedy group is safe: it stops at the first `)` that closes the call.
RE_EXP_RUN = re.compile(r"(?P<recv>" + RECV_EXP + r")\.run\(TEST_WS, (?P<args>[^()]*)\)")
RE_IMP_RUN = re.compile(r"(?P<recv>" + RECV_IMP + r")\.run\(TEST_WS, (?P<args>[^()]*)\)")
RE_IMP_PF = re.compile(r"(?P<recv>" + RECV_IMP + r")\.preflight\(TEST_WS, (?P<args>[^()]*)\)")


def convert(text: str, label: str) -> str:
    counts: dict[str, int] = {}

    def bump(key: str) -> None:
        counts[key] = counts.get(key, 0) + 1

    # ── 1. The one nested argument list. Done first, by exact match, so the flat regexes below
    #       never have to deal with a `path.join(...)` inside the arguments. ──
    nested_old = (
        'getImporter("mocks", "mocks-json")!.preflight(TEST_WS, '
        'path.join(outDir, "does-not-exist.json"))'
    )
    nested_new = (
        'preflightFile(getImporter("mocks", "mocks-json")!, TEST_WS, '
        'path.join(outDir, "does-not-exist.json"))'
    )
    if nested_old in text:
        text = text.replace(nested_old, nested_new)
        bump("preflight:nested")

    # ── 2. Flat call sites. Order matters: the importer patterns must run before the exporter
    #       ones, because `importer`/`getImporter` would otherwise be partially matched by the
    #       exporter pattern's `exporter` alternative? It would not (`Importer` vs `Exporter`
    #       differ at the first letter), but keeping the order fixed makes the intent obvious. ──
    text, n = RE_IMP_PF.subn(lambda m: f"preflightFile({m['recv']}, TEST_WS, {m['args']})", text)
    counts["preflight"] = n

    text, n = RE_IMP_RUN.subn(lambda m: f"importFile({m['recv']}, TEST_WS, {m['args']})", text)
    counts["import"] = n

    text, n = RE_EXP_RUN.subn(lambda m: f"exportToFile(TEST_WS, {m['recv']}, {m['args']})", text)
    counts["export"] = n

    # ── 3. Add the harness import, right after the registry import it pairs with. ──
    if "importExportHarness" not in text:
        anchor = 'import { getExporter, getImporter } from "@bifurc/engine/importExport/registry";\n'
        assert anchor in text, f"{label}: registry import anchor not found"
        text = text.replace(anchor, anchor + HARNESS_IMPORT, 1)

    total = sum(counts.values())
    detail = ", ".join(f"{k}={v}" for k, v in sorted(counts.items()))
    print(f"  {label}: {total} call sites converted ({detail})")
    return text


def main() -> int:
    check = "--check" in sys.argv
    files = [
        TESTS / "importExport.integration.test.ts",
        TESTS / "importExportFormats.integration.test.ts",
    ]

    for f in files:
        original = f.read_text(encoding="utf-8")
        updated = convert(original, f.name)
        if updated == original:
            print(f"  {f.name}: already converted")
            continue
        if check:
            print(f"  {f.name}: WOULD CHANGE")
        else:
            f.write_text(updated, encoding="utf-8")

    # A leftover old-signature call site is the failure this script exists to prevent.
    stale = re.compile(r"(?:exporter|importer|getExporter|getImporter)\(?[^;\n]*!?\.(?:run|preflight)\(TEST_WS, file")
    bad = False
    for f in files:
        for i, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
            if stale.search(line):
                print(f"  STALE {f.name}:{i}: {line.strip()}")
                bad = True
    if bad:
        print("FAILED — old-signature call sites remain")
        return 1

    print("OK — no old-signature call sites remain")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
