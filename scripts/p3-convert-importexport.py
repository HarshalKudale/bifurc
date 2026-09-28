"""
P3 work item 2 — convert the 32 content-shaped exporter/importer files from a path-based
interface to a content-based one (File_Ops_Protocol.md §2.1).

Every replacement is an exact (old, new) pair with an expected occurrence count. A pattern that
does not match fails the run loudly, and after each file is rewritten the script asserts that no
`fs.` reference survives — which is what catches "I removed the write but the import is still
needed" (or vice versa) rather than leaving a dangling reference for the type checker, which does
not flag unused imports here.
"""

import re
import sys
from pathlib import Path

ROOT = Path("packages/engine/src/importExport")
EXPORTERS = ROOT / "exporters"
IMPORTERS = ROOT / "importers"

FS_IMPORT = 'import * as fs from "fs";\n'
TYPES_IMPORT_OLD = 'import type { PreflightResult, ImportResult, CollisionStrategy } from "../types";'
TYPES_IMPORT_NEW = 'import type { PreflightResult, ImportResult, CollisionStrategy, ImportSource } from "../types";'

failures: list[str] = []
edits_applied = 0


def apply(path: Path, pairs: list[tuple[str, str, int]]) -> None:
    """Apply exact replacements, asserting each one matched exactly `count` times.

    Idempotent: a pattern that matches nothing but whose replacement is already present counts as
    already-applied rather than as a failure. That makes re-running the script after a partial run
    safe, which matters because the two halves (exporters, importers) are independent and either
    can be re-run on its own.
    """
    global edits_applied
    text = path.read_text(encoding="utf-8")
    original = text
    for old, new, count in pairs:
        found = text.count(old)
        if found == 0 and (not new or new in text):
            continue
        if found != count:
            failures.append(
                f"{path.name}: expected {count} occurrence(s) of\n"
                f"    {old!r}\n  found {found}"
            )
            continue
        text = text.replace(old, new)
        edits_applied += found
    if text != original:
        path.write_text(text, encoding="utf-8")


# ─────────────────────────────────────────────────────────────────────────────
# Exporters — stop writing to a path, return the rendered content
# ─────────────────────────────────────────────────────────────────────────────

# `<kind>-export.<first extension>` — deliberately the same default the shell's save dialog
# already suggests today (`defaultPath: `${safeKind}-export.${ext}`` in the old
# `importExport:export` handler), so the file name a user sees does not change.
EXPORTER_NAMES = {
    "environments-dotenv": "environments-export.env",
    "environments-json": "environments-export.json",
    "environments-postman": "environments-export.json",
    "mappings-json": "mappings-export.json",
    "mocks-json": "mocks-export.json",
    "mocks-postman": "mocks-export.json",
    "mocks-wiremock": "mocks-export.json",
    "proxyrules-json": "proxy-rules-export.json",
    "requests-curl": "requests-export.sh",
    "requests-har": "requests-export.har",
    "requests-insomnia": "requests-export.json",
    "requests-openapi": "requests-export.json",
    "requests-postman": "requests-export.json",
    "webhooks-json": "webhooks-export.json",
    "websockets-json": "websockets-export.json",
    "workspace-json": "workspace-export.json",
}

WRITE_RE = re.compile(r'^(?P<indent>[ \t]*)fs\.writeFileSync\(filePath, (?P<expr>.+), "utf-8"\);$', re.M)


def convert_exporter(name: str) -> None:
    path = EXPORTERS / f"{name}.ts"
    text = path.read_text(encoding="utf-8")

    # The one multi-branch exporter: two writes with different payloads. Handled explicitly
    # rather than by the single-write regex below.
    if name == "environments-postman":
        old = (
            "    if (environments.length === 1) {\n"
            '      fs.writeFileSync(filePath, JSON.stringify(envToPostman(environments[0]), null, 2), "utf-8");\n'
            "    } else {\n"
            "      // Multiple environments: export as array wrapped in LP container\n"
            "      const payload = {\n"
            '        schema: "lp-postman-environments-v1",\n'
            "        environments: environments.map(envToPostman),\n"
            "      };\n"
            '      fs.writeFileSync(filePath, JSON.stringify(payload, null, 2), "utf-8");\n'
            "    }\n"
        )
        new = (
            "    const content =\n"
            "      environments.length === 1\n"
            "        ? JSON.stringify(envToPostman(environments[0]), null, 2)\n"
            "        : JSON.stringify(\n"
            '            { schema: "lp-postman-environments-v1", environments: environments.map(envToPostman) },\n'
            "            null,\n"
            "            2,\n"
            "          );\n"
        )
        apply(
            path,
            [
                (FS_IMPORT, "", 1),
                ('export async function run(wsId: string, filePath: string): Promise<ExportResult> {',
                 'export async function run(wsId: string): Promise<ExportResult> {', 1),
                (old, new, 1),
                ("    return { ok: true, filePath };",
                 f'    return {{ ok: true, content, suggestedName: "{EXPORTER_NAMES[name]}" }};', 1),
            ],
        )
    else:
        text = text.replace(FS_IMPORT, "")
        matches = WRITE_RE.findall(text)
        if len(matches) == 1:
            text = WRITE_RE.sub(lambda m: f"{m.group('indent')}const content = {m.group('expr')};", text)
        elif "const content = " not in text:
            failures.append(f"{path.name}: expected exactly 1 fs.writeFileSync(filePath, …), found {len(matches)}")
            return
        path.write_text(text, encoding="utf-8")
        apply(
            path,
            [
                ('export async function run(wsId: string, filePath: string): Promise<ExportResult> {',
                 'export async function run(wsId: string): Promise<ExportResult> {', 1),
                ("    return { ok: true, filePath };",
                 f'    return {{ ok: true, content, suggestedName: "{EXPORTER_NAMES[name]}" }};', 1),
            ],
        )

    after = path.read_text(encoding="utf-8")
    if "fs." in after:
        failures.append(f"{path.name}: an `fs.` reference survived the conversion")


for exporter_name in EXPORTER_NAMES:
    convert_exporter(exporter_name)


# ─────────────────────────────────────────────────────────────────────────────
# Importers — read the uploaded content instead of a path
# ─────────────────────────────────────────────────────────────────────────────

PREFLIGHT_SIG = "export function preflight(wsId: string, filePath: string): PreflightResult {"
PREFLIGHT_SIG_PRIVATE = "export function preflight(_wsId: string, filePath: string): PreflightResult {"
PREFLIGHT_NEW = "export function preflight(wsId: string, source: ImportSource): PreflightResult {"
PREFLIGHT_NEW_PRIVATE = "export function preflight(_wsId: string, source: ImportSource): PreflightResult {"

RUN_SIG = (
    "export async function run(\n"
    "  wsId: string,\n"
    "  filePath: string,\n"
    "  strategy: CollisionStrategy,\n"
    "): Promise<ImportResult> {"
)
RUN_SIG_PRIVATE = (
    "export async function run(\n"
    "  wsId: string,\n"
    "  filePath: string,\n"
    "  _strategy: CollisionStrategy,\n"
    "): Promise<ImportResult> {"
)
RUN_SIG_NO_WS = (
    "export async function run(\n"
    "  _wsId: string,\n"
    "  filePath: string,\n"
    "  _strategy: CollisionStrategy,\n"
    "): Promise<ImportResult> {"
)
RUN_NEW = RUN_SIG.replace("  filePath: string,\n", "  source: ImportSource,\n")
RUN_NEW_PRIVATE = RUN_SIG_PRIVATE.replace("  filePath: string,\n", "  source: ImportSource,\n")
RUN_NEW_NO_WS = RUN_SIG_NO_WS.replace("  filePath: string,\n", "  source: ImportSource,\n")


def parse_helper(fn: str, ret: str, extra_imports: str = "") -> list[tuple[str, str, int]]:
    """`function <fn>(filePath)` + a JSON.parse(fs.readFileSync(...)) body → content-based."""
    return [
        (
            f"function {fn}(filePath: string): {ret} {{\n"
            '  const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));',
            f"function {fn}(content: string): {ret} {{\n  const data = JSON.parse(content);",
            1,
        ),
    ]


def convert_importer(name: str, pairs: list[tuple[str, str, int]]) -> None:
    path = IMPORTERS / f"{name}.ts"
    apply(path, [(FS_IMPORT, "", 1), (TYPES_IMPORT_OLD, TYPES_IMPORT_NEW, 1)] + pairs)
    after = path.read_text(encoding="utf-8")
    if "fs." in after:
        failures.append(f"{path.name}: an `fs.` reference survived the conversion")


JSON_IMPORTERS = {
    "environments-json": ("parse", "Environment[]", "envs", "envs.length"),
    "mappings-json": ("parse", "LocalMapping[]", "mappings", "mappings.length"),
    "mocks-json": ("parse", "{ mocks: MockRule[]; folders: Folder[] }", "{ mocks }", "mocks.length"),
    "proxyrules-json": ("parse", "ProxyRule[]", "rules", "rules.length"),
    "webhooks-json": ("parse", "{ webhooks: SavedWebhook[]; folders: Folder[] }", "{ webhooks }", "webhooks.length"),
    "websockets-json": ("parse", "{ wsConnections: SavedWsConnection[]; folders: Folder[] }", "{ wsConnections }", "wsConnections.length"),
}

for name, (fn, ret, destructure, count_expr) in JSON_IMPORTERS.items():
    # The `run` bodies destructure two names for the folder-carrying formats.
    run_destructure = destructure
    if name in ("mocks-json", "webhooks-json", "websockets-json"):
        run_destructure = destructure.replace(" }", ", folders }")

    convert_importer(
        name,
        parse_helper(fn, ret)
        + [
            (
                f"{PREFLIGHT_SIG}\n  try {{\n    const {destructure} = {fn}(filePath);",
                f"{PREFLIGHT_NEW}\n  try {{\n    const {destructure} = {fn}(source.content);",
                1,
            ),
            (
                f"    return {{ ok: true, filePath, itemCount: {count_expr}, collisionIds }};",
                f"    return {{ ok: true, itemCount: {count_expr}, collisionIds }};",
                1,
            ),
            (
                f"{RUN_SIG}\n  try {{\n    const {run_destructure} = {fn}(filePath);",
                f"{RUN_NEW}\n  try {{\n    const {run_destructure} = {fn}(source.content);",
                1,
            ),
        ],
    )

# ── The remaining nine, each with its own shape ──────────────────────────────

convert_importer(
    "environments-dotenv",
    [
        (
            f"{PREFLIGHT_SIG_PRIVATE}\n  try {{\n"
            '    const text = fs.readFileSync(filePath, "utf-8");\n'
            "    const vars = parseDotenv(text);\n"
            "    return { ok: true, filePath, itemCount: Object.keys(vars).length, collisionIds: [] };",
            f"{PREFLIGHT_NEW_PRIVATE}\n  try {{\n"
            "    const vars = parseDotenv(source.content);\n"
            "    return { ok: true, itemCount: Object.keys(vars).length, collisionIds: [] };",
            1,
        ),
        (
            f"{RUN_SIG_PRIVATE}\n  try {{\n"
            '    const text = fs.readFileSync(filePath, "utf-8");\n'
            "    const vars = parseDotenv(text);",
            f"{RUN_NEW_PRIVATE}\n  try {{\n"
            "    const vars = parseDotenv(source.content);",
            1,
        ),
        (
            '    const name = filePath.split(/[/\\\\]/).pop()?.replace(/\\.env.*$/, "") ?? "Imported";',
            "    const name = dotenvEnvName(source.filename);",
            1,
        ),
        (
            "export function preflight(",
            "// ── The one place a display name used to come from a path ────────────────────\n"
            "//\n"
            "// `File_Ops_Protocol.md` §8 singles this out: under a remote engine the path does not\n"
            "// exist locally, so the name would either be garbage or leak a fragment of the user's\n"
            "// own filesystem into an entity name. The name now comes from the `filename` the client\n"
            "// picked, which is the only side that knows it.\n"
            "//\n"
            "// `.env` and `.env.local` both reduce to \"env\", and `myapp.env` keeps `myapp` — i.e. the\n"
            "// behaviour for real-world filenames is unchanged. The `|| \"Imported\"` does fix a latent\n"
            "// bug: a file named exactly `.env` used to yield an EMPTY name, because `\"\".replace(...)`\n"
            "// is `\"\"` and `??` only fires on nullish.\n"
            "function dotenvEnvName(filename: string): string {\n"
            '  const base = filename.split(/[/\\\\]/).pop() ?? filename;\n'
            '  const stripped = base.replace(/\\.env.*$/, "");\n'
            '  return stripped || "Imported";\n'
            "}\n"
            "\n"
            "export function preflight(",
            1,
        ),
    ],
)

convert_importer(
    "environments-postman",
    [
        (
            "function parseFile(filePath: string): PostmanEnvironment[] {\n"
            '  const data = JSON.parse(fs.readFileSync(filePath, "utf-8"));',
            "function parseFile(content: string): PostmanEnvironment[] {\n  const data = JSON.parse(content);",
            1,
        ),
        (
            f"{PREFLIGHT_SIG}\n  try {{\n"
            "    const parsed = parseFile(filePath);\n"
            "    return { ok: true, filePath, itemCount: parsed.length, collisionIds: [] };",
            f"{PREFLIGHT_NEW}\n  try {{\n"
            "    const parsed = parseFile(source.content);\n"
            "    return { ok: true, itemCount: parsed.length, collisionIds: [] };",
            1,
        ),
        (
            f"{RUN_SIG_PRIVATE}\n  try {{\n    const parsed = parseFile(filePath);",
            f"{RUN_NEW_PRIVATE}\n  try {{\n    const parsed = parseFile(source.content);",
            1,
        ),
    ],
)

convert_importer(
    "mocks-postman",
    [
        (
            f"{PREFLIGHT_SIG}\n  try {{\n"
            '    const raw = fs.readFileSync(filePath, "utf-8");\n'
            "    const parsed = parsePostmanMocks(raw);\n"
            "    return { ok: true, filePath, itemCount: parsed.mocks.length, collisionIds: [] };",
            f"{PREFLIGHT_NEW}\n  try {{\n"
            "    const parsed = parsePostmanMocks(source.content);\n"
            "    return { ok: true, itemCount: parsed.mocks.length, collisionIds: [] };",
            1,
        ),
        (
            f"{RUN_SIG_PRIVATE}\n  try {{\n"
            '    const raw = fs.readFileSync(filePath, "utf-8");\n'
            "    const parsed = parsePostmanMocks(raw);",
            f"{RUN_NEW_PRIVATE}\n  try {{\n    const parsed = parsePostmanMocks(source.content);",
            1,
        ),
    ],
)

convert_importer(
    "mocks-wiremock",
    [
        (
            f"{PREFLIGHT_SIG}\n  try {{\n    const data = JSON.parse(fs.readFileSync(filePath, \"utf-8\"));",
            f"{PREFLIGHT_NEW}\n  try {{\n    const data = JSON.parse(source.content);",
            1,
        ),
        (
            "    return { ok: true, filePath, itemCount: stubs.length, collisionIds };",
            "    return { ok: true, itemCount: stubs.length, collisionIds };",
            1,
        ),
        (
            f"{RUN_SIG}\n  try {{\n    const data = JSON.parse(fs.readFileSync(filePath, \"utf-8\"));",
            f"{RUN_NEW}\n  try {{\n    const data = JSON.parse(source.content);",
            1,
        ),
    ],
)

convert_importer(
    "requests-curl",
    [
        (
            f"{PREFLIGHT_SIG_PRIVATE}\n  try {{\n"
            '    const text = fs.readFileSync(filePath, "utf-8");\n'
            "    const blocks = splitCurlBlocks(text);",
            f"{PREFLIGHT_NEW_PRIVATE}\n  try {{\n    const blocks = splitCurlBlocks(source.content);",
            1,
        ),
        (
            "    return { ok: true, filePath, itemCount: valid.length, collisionIds: [] };",
            "    return { ok: true, itemCount: valid.length, collisionIds: [] };",
            1,
        ),
        (
            f"{RUN_SIG_PRIVATE}\n  try {{\n"
            '    const text = fs.readFileSync(filePath, "utf-8");\n'
            "    const blocks = splitCurlBlocks(text);",
            f"{RUN_NEW_PRIVATE}\n  try {{\n"
            "    const text = source.content;\n"
            "    const blocks = splitCurlBlocks(text);",
            1,
        ),
    ],
)

for name, cls in (("requests-har", "Har"), ("requests-insomnia", "InsomniaExport")):
    convert_importer(
        name,
        [
            (
                f'{PREFLIGHT_SIG_PRIVATE}\n  try {{\n    const data = JSON.parse(fs.readFileSync(filePath, "utf-8")) as {cls};',
                f"{PREFLIGHT_NEW_PRIVATE}\n  try {{\n    const data = JSON.parse(source.content) as {cls};",
                1,
            ),
            (
                "    return { ok: true, filePath, itemCount: count, collisionIds: [] };",
                "    return { ok: true, itemCount: count, collisionIds: [] };",
                1,
            ),
            (
                f'{RUN_SIG_PRIVATE}\n  try {{\n    const data = JSON.parse(fs.readFileSync(filePath, "utf-8")) as {cls};',
                f"{RUN_NEW_PRIVATE}\n  try {{\n    const data = JSON.parse(source.content) as {cls};",
                1,
            ),
        ],
    )

convert_importer(
    "requests-openapi",
    [
        (
            "async function loadSpec(filePath: string): Promise<OpenApiSpec> {\n"
            '  const text = fs.readFileSync(filePath, "utf-8");\n'
            "  const ext = path.extname(filePath).toLowerCase();",
            "// ── The second path-derivation site, which `plan/04` does not name ───────────\n"
            "//\n"
            "// `plan/04` work item 2 says to \"grep for any other `filePath.split` / `path.basename(filePath)`\"\n"
            "// before declaring the name-derivation fix done. That grep does not match `path.extname`,\n"
            "// which is what this function used. It is worse than a naming bug: a staged blob's content\n"
            "// file has no extension at all, so `path.extname()` on it is `\"\"` and EVERY YAML spec would\n"
            "// have silently taken the JSON branch and failed to parse.\n"
            "//\n"
            "// The extension now comes from the client-supplied filename. `path` is a pure string\n"
            "// helper, so keeping the import does not reintroduce filesystem access.\n"
            "async function loadSpec(source: ImportSource): Promise<OpenApiSpec> {\n"
            "  const text = source.content;\n"
            "  const ext = path.extname(source.filename).toLowerCase();",
            1,
        ),
        (
            f'{PREFLIGHT_SIG_PRIVATE}\n  try {{\n    const text = fs.readFileSync(filePath, "utf-8");',
            f"{PREFLIGHT_NEW_PRIVATE}\n  try {{\n    const text = source.content;",
            1,
        ),
        (
            '      return { ok: true, filePath, itemCount: Math.floor(lines / 3), collisionIds: [] };',
            "      return { ok: true, itemCount: Math.floor(lines / 3), collisionIds: [] };",
            1,
        ),
        (
            "    return { ok: true, filePath, itemCount: count, collisionIds: [] };",
            "    return { ok: true, itemCount: count, collisionIds: [] };",
            1,
        ),
        (
            f"{RUN_SIG_PRIVATE}\n  try {{\n    const spec = await loadSpec(filePath);",
            f"{RUN_NEW_PRIVATE}\n  try {{\n    const spec = await loadSpec(source);",
            1,
        ),
    ],
)

convert_importer(
    "requests-postman",
    [
        (
            f"{PREFLIGHT_SIG}\n  try {{\n"
            '    const raw = fs.readFileSync(filePath, "utf-8");\n'
            "    const parsed = parsePostmanRequests(raw);",
            f"{PREFLIGHT_NEW}\n  try {{\n    const parsed = parsePostmanRequests(source.content);",
            1,
        ),
        (
            "    return {\n      ok: true,\n      filePath,\n      itemCount: parsed.requests.length,\n      collisionIds: [],\n    };",
            "    return {\n      ok: true,\n      itemCount: parsed.requests.length,\n      collisionIds: [],\n    };",
            1,
        ),
        (
            f"{RUN_SIG_PRIVATE}\n  try {{\n"
            '    const raw = fs.readFileSync(filePath, "utf-8");\n'
            "    const parsed = parsePostmanRequests(raw);",
            f"{RUN_NEW_PRIVATE}\n  try {{\n    const parsed = parsePostmanRequests(source.content);",
            1,
        ),
    ],
)

convert_importer(
    "workspace-json",
    [
        (
            f"{PREFLIGHT_SIG}\n  try {{\n"
            '    const raw = fs.readFileSync(filePath, "utf-8");\n'
            "    const snapshot = JSON.parse(raw);",
            f"{PREFLIGHT_NEW}\n  try {{\n    const snapshot = JSON.parse(source.content);",
            1,
        ),
        (
            "    return { ok: true, filePath, itemCount, collisionIds: [] };",
            "    return { ok: true, itemCount, collisionIds: [] };",
            1,
        ),
        (
            f"{RUN_SIG_NO_WS}\n  try {{\n"
            '    const raw = fs.readFileSync(filePath, "utf-8");\n'
            "    const snapshot = JSON.parse(raw);",
            f"{RUN_NEW_NO_WS}\n  try {{\n    const snapshot = JSON.parse(source.content);",
            1,
        ),
    ],
)

# ─────────────────────────────────────────────────────────────────────────────

print(f"replacements applied: {edits_applied}")
if failures:
    print(f"\n!! {len(failures)} FAILURE(S):\n", file=sys.stderr)
    for f in failures:
        print(f"  - {f}\n", file=sys.stderr)
    sys.exit(1)
print("all files converted cleanly")
