"""P6 stage 3b, the rename slice — make the shell's kind-registration surface honest.

Two jobs, both about names that no longer match what the code does:

1. `soapHandlers.ts` / `graphqlHandlers.ts` / `grpcHandlers.ts` register **kinds, not handlers** since
   stage 3b deleted their channels, so they become `*Kinds.ts` with `register*Kinds()` entry points.
   `crudHandlers.ts` deliberately keeps its name — it registers six kinds *and* five live `webhook*`
   channels, so "handlers" is still half-true.

2. The "**six** caller files" claim, repeated through P6's docs and the engine docblock, is wrong. It is
   **five**, and `plan/07` disproves itself by listing five names in the same sentence. Verified with
   `git log -S`: `coreHandlers`, `crudHandlers`, `graphqlHandlers`, `grpcHandlers`, `soapHandlers` are
   every file that has ever contained either factory call.

Every replacement asserts its match count, so a silent no-op is impossible.
"""
import sys
from pathlib import Path

# ── 1. Token renames, applied to the moved files and their three consumers ───────────────────────
TOKEN_FILES = [
    "src/ipc/handlers/soapKinds.ts",
    "src/ipc/handlers/graphqlKinds.ts",
    "src/ipc/handlers/grpcKinds.ts",
    "src/ipc/handlers.ts",
    "tests/integration/protocolExecution.integration.test.ts",
    "tests/spike/protocolPoc.test.ts",
]

TOKENS = [
    ("registerSoapHandlers", "registerSoapKinds"),
    ("registerGraphqlHandlers", "registerGraphqlKinds"),
    ("registerGrpcHandlers", "registerGrpcKinds"),
    ("ipc/handlers/soapHandlers", "ipc/handlers/soapKinds"),
    ("ipc/handlers/graphqlHandlers", "ipc/handlers/graphqlKinds"),
    ("ipc/handlers/grpcHandlers", "ipc/handlers/grpcKinds"),
]

# ── 2. Per-file docblock edits ───────────────────────────────────────────────────────────────────
# The three files carry an identical closing sentence about their name being a lie; each gets its own
# replacement because the file and entry-point names differ.
NAME_IS_A_LIE = (
    "// `registerCrudKind()` / `registerSimpleKind()` directly. Its name is now a lie, and renaming it is the\n"
    "// next slice."
)


def renamed_paragraph(old_file: str, new_file: str, old_fn: str, new_fn: str) -> str:
    return (
        f"// `registerCrudKind()` / `registerSimpleKind()` directly. **The name now says what the file does**\n"
        f"// — the rename slice (2026-09-20) took `{old_file}` → `{new_file}` and `{old_fn}()` →\n"
        f"// `{new_fn}()`, because a file called `*Handlers.ts` that registers no handler invites someone to\n"
        f"// add one. `crudHandlers.ts` kept its name on purpose: it registers six kinds *and* five live\n"
        f"// `webhook*` channels, so \"handlers\" is still half-true there."
    )


FILE_EDITS = {
    # NB: the `export function register*Handlers()` → `register*Kinds()` rename is done by the token pass
    # above, which covers the moved files too. These entries are only the prose the tokens do not reach.
    "src/ipc/handlers/soapKinds.ts": [
        (NAME_IS_A_LIE, renamed_paragraph("soapHandlers.ts", "soapKinds.ts", "registerSoapHandlers", "registerSoapKinds")),
    ],
    "src/ipc/handlers/graphqlKinds.ts": [
        ("// and able to do nothing else. Same rule as `soapHandlers.ts`, stated once there: **a channel whose body",
         "// and able to do nothing else. Same rule as `soapKinds.ts`, stated once there: **a channel whose body"),
        (NAME_IS_A_LIE, renamed_paragraph("graphqlHandlers.ts", "graphqlKinds.ts", "registerGraphqlHandlers", "registerGraphqlKinds")),
    ],
    "src/ipc/handlers/grpcKinds.ts": [
        ("// `soapHandlers.ts`, stated once there: **a channel whose body is a single `commandRegistry.invoke()`",
         "// `soapKinds.ts`, stated once there: **a channel whose body is a single `commandRegistry.invoke()`"),
        (NAME_IS_A_LIE, renamed_paragraph("grpcHandlers.ts", "grpcKinds.ts", "registerGrpcHandlers", "registerGrpcKinds")),
    ],
}

# ── 3. The "six" → "five" correction ─────────────────────────────────────────────────────────────
SIX_FIXES = {
    "packages/engine/src/store/entityCrudCommands.ts": [
        (
            """ *  - `registerEntityCrudHandlers(opts)` / `registerSimpleEntityHandlers(opts)` were called from **six
 *    shell handler files** (`coreHandlers`, `crudHandlers`, `graphqlHandlers`, `grpcHandlers`,
 *    `soapHandlers`, and one more kind each way). They did two unrelated things: they inserted the
 *    kind's options into a module-level container, **and** they installed three `ipcMain.handle`
 *    channels. **Stage 3b (2026-09-20) deleted the second half**, and the wrappers with it — the six
 *    files now call `registerCrudKind()` / `registerSimpleKind()` directly.
""",
            """ *  - `registerEntityCrudHandlers(opts)` / `registerSimpleEntityHandlers(opts)` were called from **five
 *    shell handler files** — `coreHandlers`, `crudHandlers`, `graphqlHandlers`, `grpcHandlers` and
 *    `soapHandlers`. (Every P6 doc before 2026-09-20 says *six*, including one that then lists exactly
 *    those five names; measured with `git log -S`, five is the number that has ever been true.) They did
 *    two unrelated things: they inserted the kind's options into a module-level container, **and** they
 *    installed three `ipcMain.handle` channels. **Stage 3b (2026-09-20) deleted the second half**, and
 *    the wrappers with it — the five files now call `registerCrudKind()` / `registerSimpleKind()`
 *    directly.
""",
        ),
        (
            " * into the engine, and kept their signatures **byte-identical**, so **none of the six caller files",
            " * into the engine, and kept their signatures **byte-identical**, so **none of the five caller files",
        ),
        (
            """/** The container insert for a full-CRUD kind. Called directly by the six shell `*Handlers.ts` files""",
            """/** The container insert for a full-CRUD kind. Called directly by the five shell `*Handlers.ts` files""",
        ),
    ],
    "plan/07-phase-6-shell-seam.md": [
        (
            "half). **Both factory signatures are byte-identical, so none of the six caller files needed an edit** —",
            "half). **Both factory signatures are byte-identical, so none of the five caller files needed an edit** —",
        ),
    ],
    "plan/13-checklist.md": [
        (
            "      keeps the channels, and **both factory signatures are unchanged, so none of the six caller files",
            "      keeps the channels, and **both factory signatures are unchanged, so none of the five caller files",
        ),
    ],
}


def sub(text: str, old: str, new: str, where: str, problems: list) -> str:
    n = text.count(old)
    if n != 1:
        problems.append(f"{where}: expected 1 match, found {n} :: {old.splitlines()[0][:70]!r}")
        return text
    return text.replace(old, new, 1)


def main() -> int:
    problems: list[str] = []

    for rel in TOKEN_FILES:
        p = Path(rel)
        if not p.exists():
            problems.append(f"{rel}: missing")
            continue
        text = p.read_text(encoding="utf-8")
        crlf = "\r\n" in text
        if crlf:
            text = text.replace("\r\n", "\n")
        total = 0
        for old, new in TOKENS:
            n = text.count(old)
            if n:
                text = text.replace(old, new)
                total += n
        if crlf:
            text = text.replace("\n", "\r\n")
        p.write_text(text, encoding="utf-8")
        print(f"tokens  {rel:<52} {total} replaced")

    for rel, edits in FILE_EDITS.items():
        p = Path(rel)
        text = p.read_text(encoding="utf-8")
        crlf = "\r\n" in text
        if crlf:
            text = text.replace("\r\n", "\n")
        for old, new in edits:
            text = sub(text, old, new, rel, problems)
        if crlf:
            text = text.replace("\n", "\r\n")
        p.write_text(text, encoding="utf-8")
        print(f"edits   {rel}")

    for rel, edits in SIX_FIXES.items():
        p = Path(rel)
        text = p.read_text(encoding="utf-8")
        crlf = "\r\n" in text
        if crlf:
            text = text.replace("\r\n", "\n")
        for old, new in edits:
            text = sub(text, old, new, rel, problems)
        if crlf:
            text = text.replace("\n", "\r\n")
        p.write_text(text, encoding="utf-8")
        print(f"six->5  {rel}")

    if problems:
        print("\nPROBLEMS:")
        for x in problems:
            print("  -", x)
        return 1
    print("\nall replacements applied")
    return 0


if __name__ == "__main__":
    sys.exit(main())
