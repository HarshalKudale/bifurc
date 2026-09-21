#!/usr/bin/env python3
"""P6 reduction, third group (`folderHandlers.ts`) — the test re-point.

Every replacement asserts its own match count, so a silent no-op is impossible: a stale
`old` string fails the script instead of leaving a `getHandler("folder:…")` call behind
for the deleted channel to blow up on at invocation time.

Run from the repository root:
    python scripts/p6-reduction-folder-docs.py
"""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FILE = ROOT / "tests" / "ipc" / "handlers.test.ts"

# (old, new, expected_count)
EDITS = [
    # ── 1. `expectedChannels` loses the three folder names it listed ──────────────────────────────
    (
        '      "folder:add", "folder:rename", "folder:delete",\n      "env:setActive",\n',
        '      // ── The three folder channels are gone (P6 reduction, third group, 2026-09-21) ──────────\n'
        '      // `folder:add` / `folder:rename` / `folder:delete` were deleted with `folderHandlers.ts`\n'
        '      // (`plan/07` §5b-quaterdecies). Each was a one-line `commandRegistry.invoke()` delegate\n'
        '      // reachable only through the client, which routes to the `folder.*` commands directly —\n'
        '      // the cases below now drive those commands. `folder:move` was never in this list.\n'
        '      "env:setActive",\n',
        1,
    ),
    # ── 2. The helpers, beside the `entity.*` and `history.*` ones ────────────────────────────────
    (
        'const historyDiff = <T = any>(commitHash: string, filePath: string, workspaceId: string): Promise<T> =>\n'
        '  commandRegistry.invoke("history.diff", { commitHash, filePath, workspaceId }, ENTITY_CTX);\n',
        'const historyDiff = <T = any>(commitHash: string, filePath: string, workspaceId: string): Promise<T> =>\n'
        '  commandRegistry.invoke("history.diff", { commitHash, filePath, workspaceId }, ENTITY_CTX);\n'
        '\n'
        '/**\n'
        ' * The `folder.*` commands, driven directly — the same substitution again, for the reduction\'s\n'
        ' * **third** group (`plan/07` §5b-quaterdecies).\n'
        ' *\n'
        ' * `folder:add` / `folder:rename` / `folder:move` / `folder:delete` were deleted with\n'
        ' * `src/ipc/handlers/folderHandlers.ts` on 2026-09-21; all four were one-line\n'
        ' * `commandRegistry.invoke()` adapters, so calling the command is what the channel did minus the\n'
        ' * channel. **`folderAdd` keeps the channel\'s spread** (`{ kind, ...folder }`) rather than the\n'
        ' * client\'s field-picking, because the cases below pass a folder object and the spread is the\n'
        ' * behaviour being reproduced — see the note in `foldersAndWorkspaces.integration.test.ts` for why\n'
        ' * the two shapes are equivalent for the renderer but not for that suite.\n'
        ' */\n'
        'const folderAdd = (kind: string, folder: any): any =>\n'
        '  commandRegistry.invoke("folder.add", { kind, ...folder }, ENTITY_CTX);\n'
        'const folderRename = (kind: string, id: string, name: string): any =>\n'
        '  commandRegistry.invoke("folder.rename", { kind, id, name }, ENTITY_CTX);\n'
        'const folderMove = (kind: string, id: string, parentId: string | null): any =>\n'
        '  commandRegistry.invoke("folder.move", { kind, id, parentId }, ENTITY_CTX);\n'
        'const folderDelete = (kind: string, id: string): any =>\n'
        '  commandRegistry.invoke("folder.delete", { kind, id }, ENTITY_CTX);\n',
        1,
    ),
    # ── 3. The fourteen call sites ────────────────────────────────────────────────────────────────
    ('getHandler("folder:add")(EVENT, "mock", baseFolder)', 'folderAdd("mock", baseFolder)', 1),
    ('getHandler("folder:add")(EVENT, "request", baseFolder)', 'folderAdd("request", baseFolder)', 1),
    ('getHandler("folder:rename")(EVENT, "mock", "f1", "New Name")', 'folderRename("mock", "f1", "New Name")', 1),
    ('getHandler("folder:rename")(EVENT, "request", "f2", "New")', 'folderRename("request", "f2", "New")', 1),
    ('getHandler("folder:rename")(EVENT, "mock", "f1", "Y")', 'folderRename("mock", "f1", "Y")', 1),
    # Twice: "deletes a folder from mockFolders" and "returns ok when deleting a missing folder".
    ('getHandler("folder:delete")(EVENT, "mock", "f1")', 'folderDelete("mock", "f1")', 2),
    ('getHandler("folder:delete")(EVENT, "request", "f2")', 'folderDelete("request", "f2")', 1),
    (
        'expect(() => getHandler("folder:rename")(EVENT, "mock", "nonexistent", "New Name")).not.toThrow()',
        'expect(() => folderRename("mock", "nonexistent", "New Name")).not.toThrow()',
        1,
    ),
    (
        'getHandler("folder:add")(EVENT, "mock", { name: "F", parentId: null })',
        'folderAdd("mock", { name: "F", parentId: null })',
        1,
    ),
    (
        'getHandler("folder:add")(EVENT, "request", { name: "F", parentId: null })',
        'folderAdd("request", { name: "F", parentId: null })',
        1,
    ),
    (
        'getHandler("folder:add")(EVENT, "ws", { name: "WS Folder", parentId: null })',
        'folderAdd("ws", { name: "WS Folder", parentId: null })',
        1,
    ),
    ('getHandler("folder:rename")(EVENT, "ws", "f1", "New")', 'folderRename("ws", "f1", "New")', 1),
    ('getHandler("folder:delete")(EVENT, "ws", "f1")', 'folderDelete("ws", "f1")', 1),
]


def main() -> int:
    text = FILE.read_text(encoding="utf-8")
    for old, new, want in EDITS:
        got = text.count(old)
        if got != want:
            print(f"FAIL  expected {want} match(es), found {got}: {old[:80]!r}", file=sys.stderr)
            return 1
        text = text.replace(old, new)
        print(f"ok    {got}x  {old[:72]!r}")

    FILE.write_text(text, encoding="utf-8")

    # Nothing may be left pointing at a deleted channel.
    leftovers = [ln for ln in text.splitlines() if 'getHandler("folder:' in ln]
    if leftovers:
        print("FAIL  residual getHandler folder call sites:", file=sys.stderr)
        for ln in leftovers:
            print("      " + ln.strip(), file=sys.stderr)
        return 1
    print("ok    no residual getHandler(\"folder:…\") call sites")
    return 0


if __name__ == "__main__":
    sys.exit(main())
