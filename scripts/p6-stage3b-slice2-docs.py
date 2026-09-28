"""P6 stage 3b slice 2 — correct the docblocks that stage 3b made false.

`packages/engine/src/store/entityCrudCommands.ts` was written when the shell still held
`registerEntityCrudHandlers` / `registerSimpleEntityHandlers`. Slice 2 deleted both, so its header
described a world that no longer exists: a wrong doc regenerates the bug it describes (`TRAPS.md`),
and here it would have told the next reader that the six shell files still own a wrapper to edit.

Every replacement is exact-match with a count assertion, so a silent no-op is impossible.
"""
import sys
from pathlib import Path

TARGET = Path("packages/engine/src/store/entityCrudCommands.ts")

REPLACEMENTS = [
    # R1 — the factory's two jobs, now past tense + the slice that removed the second.
    (
        """ *  - `registerEntityCrudHandlers(opts)` / `registerSimpleEntityHandlers(opts)` are called from **six
 *    shell handler files** (`coreHandlers`, `crudHandlers`, `graphqlHandlers`, `grpcHandlers`,
 *    `soapHandlers`, and one more kind each way). They do two unrelated things: they insert the kind's
 *    options into a module-level container, **and** they install three `ipcMain.handle` channels.
""",
        """ *  - `registerEntityCrudHandlers(opts)` / `registerSimpleEntityHandlers(opts)` were called from **six
 *    shell handler files** (`coreHandlers`, `crudHandlers`, `graphqlHandlers`, `grpcHandlers`,
 *    `soapHandlers`, and one more kind each way). They did two unrelated things: they inserted the
 *    kind's options into a module-level container, **and** they installed three `ipcMain.handle`
 *    channels. **Stage 3b (2026-09-20) deleted the second half**, and the wrappers with it — the six
 *    files now call `registerCrudKind()` / `registerSimpleKind()` directly.
""",
    ),
    # R2 — "the wrappers stay in the shell" is now false; record why the split had to precede the delete.
    (
        """ * two factory functions. `registerEntityCrudHandlers` / `registerSimpleEntityHandlers` stay in the
 * shell (they own `ipcMain`), call into the engine, and keep their signatures **byte-identical**, so
 * **none of the six caller files needed an edit**. That is the property to check first when a factory
 * — rather than a command list — is what has to move.
""",
        """ * two factory functions. At that slice the wrappers stayed in the shell (they owned `ipcMain`), called
 * into the engine, and kept their signatures **byte-identical**, so **none of the six caller files
 * needed an edit**. That is the property to check first when a factory — rather than a command list —
 * is what has to move.
 *
 * **Stage 3b then deleted the shell half**, so the six files import from here directly. The split could
 * not be skipped: the wrappers did the container insert *as well as* the channels, so deleting them
 * outright would have left all 16 kinds throwing `"<kind>" is not routed through the CommandRegistry
 * yet` — the same message a genuinely unregistered kind throws (`plan/07` §5b-ter).
""",
    ),
    # R3 — the constraint now belongs to `registerCrudKind`, not the deleted wrapper.
    (
        """/** The shape every CRUD-factory entity satisfies — pulled out so the extracted `*Core`
 * functions below can share it with `registerEntityCrudHandlers`'s own generic constraint. */""",
        """/** The shape every CRUD-factory entity satisfies — pulled out so the extracted `*Core`
 * functions below can share it with `registerCrudKind`'s own generic constraint. */""",
    ),
    # R4 — `registerCrudKind`'s own docblock: the shell no longer keeps the channels.
    (
        """/** The engine half of `registerEntityCrudHandlers()` — the shell keeps the three `ipcMain.handle`
 * channels and calls this for the container insert. Split deliberately: the container is engine state,
 * and letting the shell mutate it directly is what made this file's dependency direction wrong. */""",
        """/** The container insert for a full-CRUD kind. Called directly by the six shell `*Handlers.ts` files
 * since stage 3b deleted `registerEntityCrudHandlers()` — the wrapper that used to hold this call and
 * the three `ipcMain.handle` channels. Split deliberately: the container is engine state, and letting
 * the shell mutate it directly is what made this file's dependency direction wrong. */""",
    ),
    # R5 — `registerSimpleKind`'s docblock still named the deleted wrapper.
    (
        """/** The engine half of `registerSimpleEntityHandlers()` — see `registerCrudKind()`. */""",
        """/** The container insert for a no-`AppConfig`-array kind — see `registerCrudKind()`. */""",
    ),
    # R6 — the `*Core` extraction rationale: one of its two callers is gone.
    (
        """// ── Core logic, extracted so both the legacy per-kind `ipcMain.handle` channels and the
// collapsed `entity.create`/`entity.update`/`entity.delete` commands below share one
// implementation (P2 work item 7's "CRUD collapse" — see `registry.ts`'s own status note). Each
// function's body is byte-for-byte what used to live directly inside the corresponding
// `ipcMain.handle` callback; nothing here changes behaviour, only where the code lives.
""",
        """// ── Core logic, extracted so the legacy per-kind `ipcMain.handle` channels and the collapsed
// `entity.create`/`entity.update`/`entity.delete` commands below could share one implementation
// (P2 work item 7's "CRUD collapse" — see `registry.ts`'s own status note). Each function's body is
// byte-for-byte what used to live directly inside the corresponding `ipcMain.handle` callback;
// nothing here changes behaviour, only where the code lives. **The channels are gone as of stage 3b**,
// so the four commands below are the only caller left — which is what the extraction was for.
""",
    ),
]


def main() -> int:
    text = TARGET.read_text(encoding="utf-8")
    # The file is LF today, but a future checkout under `core.autocrlf` would hand us CRLF; match
    # either so the script is not a one-shot. Writes back the form it found.
    crlf = "\r\n" in text
    if crlf:
        text = text.replace("\r\n", "\n")

    failures = 0
    for i, (old, new) in enumerate(REPLACEMENTS, start=1):
        count = text.count(old)
        if count != 1:
            print(f"R{i}: FAIL - expected exactly 1 match, found {count}")
            failures += 1
            continue
        text = text.replace(old, new, 1)
        print(f"R{i}: ok")

    if failures:
        print(f"\n{failures} replacement(s) failed; nothing written.")
        return 1

    if crlf:
        text = text.replace("\n", "\r\n")
    TARGET.write_text(text, encoding="utf-8")
    print(f"\nwrote {TARGET}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
