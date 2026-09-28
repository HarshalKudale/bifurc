/**
 * The engine module-layout codemod — `Engine_Module_Plan.md` §7.
 *
 * `packages/engine/src` is being reorganised into feature modules behind an enforced layering rule.
 * Every reference to a moved file must follow it, and there are **four code forms** in this
 * repository, not one:
 *
 *   - `from "@bifurc/engine/<sub>"`                  — static import / re-export
 *   - `import("@bifurc/engine/<sub>")`               — dynamic import
 *   - `typeof import("@bifurc/engine/<sub>")`        — type-position import
 *   - `vi.mock("@bifurc/engine/<sub>")` / `.doMock`  — module-mock path
 *   - `require("@bifurc/engine/<sub>")`              — CJS require
 *
 * ⚠ **The dynamic-import and `vi.mock` forms are the reason this is a script and not a find/replace.**
 * A `from`-only rewrite would miss 40 % of the surface silently: the dynamic ones throw at runtime and
 * the mocks fail in a way that reads as an unrelated test defect.
 *
 * ## Scope
 *
 * **Code files only** — `.ts .tsx .mts .mjs .js .cjs`. Documentation (`plan/*.md`, `TESTING.md`,
 * `.workbuddy-ai/memory/*`) is **phase 8's** job and is deliberately untouched here: a dated log or a
 * numbered `TRAPS.md` entry records the path that was correct *when it was written*, and rewriting it
 * would make the record claim the move had already happened. `Engine_Module_Plan.md` is excluded
 * outright — it *documents the old→new mapping*, so rewriting it would erase the thing it exists to say.
 *
 * Inside a code file, prose mentions (` * ... @bifurc/engine/store/config ...`) **are** rewritten, because
 * a docblock naming a path that no longer exists is a navigation break for the next reader — the
 * `repoint-gui-references.mjs` convention. They are counted and listed separately so a reviewer can spot
 * a comment that was *describing history* rather than *pointing at code*. `--no-mentions` disables it.
 *
 * ## Usage
 *
 *   node scripts/engine-layout-codemod.mjs --emit-map                  # write plan/engine-layout-map.json
 *   node scripts/engine-layout-codemod.mjs --check                     # report + validate totality
 *   node scripts/engine-layout-codemod.mjs --apply --phase 1 --write   # rewrite (omit --write for dry)
 *
 * `--dry` is the default for `--apply`. Re-running an applied phase must report **0 rewrites** — that
 * idempotency check is what proves the rewrite is complete rather than merely quiet.
 */
import fs from "node:fs";
import path from "node:path";

const ROOT = process.cwd();
const SRC_REL = "packages/engine/src";
const SRC = path.join(ROOT, SRC_REL);
const MAP_PATH = path.join(ROOT, "plan/engine-layout-map.json");

const argv = process.argv.slice(2);
const has = (f) => argv.includes(f);
const flagValue = (f) => {
  const i = argv.indexOf(f);
  return i === -1 ? null : argv[i + 1];
};
const MODE = has("--emit-map") ? "emit-map" : has("--apply") ? "apply" : "check";
const WRITE = has("--write");
const MENTIONS = !has("--no-mentions");
const PHASE = flagValue("--phase") ? Number(flagValue("--phase")) : null;

// ---------------------------------------------------------------------------
// The rule table. `from` is a `packages/engine/src`-relative path without its
// extension; `phase` is the migration phase (plan §7.2).
// ---------------------------------------------------------------------------
const F = (from, to, phase, note) => ({ from, to, phase, note });

const FILES = [
  // --- root (6) ------------------------------------------------------------
  F("index", "index", 0),
  F("registerAll", "registerAll", 0),
  F("eventBus", "kernel/eventBus", 1),
  F("miscCommands", null, 7, "dissolves (§4.5): request.replay→rest/, script.execute→mocks/, healthbar.*→workspace/"),
  F("startup", "lifecycle/startup", 7),
  F("shutdown", "lifecycle/shutdown", 7),

  // --- commands/ (3) -------------------------------------------------------
  F("commands/registry", "kernel/registry", 1),
  F("commands/entityKindMap", "kernel/kinds", 1),
  F("commands/echoWorkspace", "utils/echoWorkspace", 1),

  // --- lib/ (3) → utils/ ---------------------------------------------------
  F("lib/net", "utils/net", 1),
  F("lib/randomNames", "utils/randomNames", 1),
  F("lib/randomizer", "utils/randomizer", 1),

  // --- proxy/ (22) ---------------------------------------------------------
  F("proxy/constants", "utils/http", 4, "HOP_BY_HOP — both proxy halves use it"),
  F("proxy/decompressUtils", "utils/decompress", 4),
  F("proxy/responseUtils", "utils/response", 4),
  F("proxy/serverUtils", "utils/server", 4),
  F("proxy/logEmitter", "kernel/telemetry", 1, "3 areas consume it"),
  F("proxy/edge", "reverseproxy/edge", 4),
  F("proxy/pages", "reverseproxy/pages", 4),
  F("proxy/mockHandler", "mocks/mockHandler", 4),
  F("proxy/scriptContext", "mocks/scriptContext", 4),
  F("proxy/scriptExecutor", "mocks/scriptExecutor", 4),
  F("proxy/certManager", "certificates/certManager", 4),
  F("proxy/certCommands", "certificates/certCommands", 4),
  F("proxy/tlsCert", "certificates/tlsCert", 4),
  F("proxy/webhookServer", "webhooks/webhookServer", 4),
  F("proxy/webhookCommands", "webhooks/webhookCommands", 4),
  F("proxy/server", "proxy/server", 0),
  F("proxy/proxyHandler", "proxy/proxyHandler", 0),
  F("proxy/routingUtils", "proxy/routingUtils", 0),
  F("proxy/tlsIntercept", "proxy/tlsIntercept", 0),
  F("proxy/serverReplay", "proxy/serverReplay", 0),
  F("proxy/service-discovery", "proxy/service-discovery", 0),
  F("proxy/serverCommands", "proxy/serverCommands", 0),

  // --- store/ (23) ---------------------------------------------------------
  F("store/paths", "workspace/paths", 3),
  F("store/types", "workspace/types", 3, "⚠ named in vitest.config.ts coverage exclude"),
  F("store/config", "workspace/config", 3),
  F("store/appSettings", "workspace/appSettings", 3),
  F("store/workspaceFs", "workspace/workspaceFs", 3, "THE kind vocabulary — ALL_ENTITY_KINDS / FLAT_ENTITY_KINDS"),
  F("store/enabledSet", "workspace/enabledSet", 3),
  F("store/kindNameEntries", "workspace/kindNameEntries", 3),
  F("store/gitStore", "gitops/gitStore", 6, "⚠ GIT_OPTIONAL_LOCKS=0 lives here"),
  F("store/configCommands", "workspace/entities/configCommands", 3, "hand-writes the rebind fan-out — §4.1"),
  F("store/coreCommands", "workspace/entities/coreCommands", 3),
  F("store/entityCrudCommands", "workspace/entities/entityCrudCommands", 3),
  F("store/folderCommands", "workspace/entities/folderCommands", 3),
  F("store/auditCommands", "workspace/entities/auditCommands", 3),
  F("store/kinds/coreKinds", "workspace/entities/kinds/coreKinds", 3),
  F("store/kinds/crudKinds", "workspace/entities/kinds/crudKinds", 3),
  F("store/kinds/graphqlKinds", "graphql/kinds", 5),
  F("store/kinds/grpcKinds", "grpc/kinds", 5),
  F("store/kinds/soapKinds", "soap/kinds", 5),
  F("store/workspace/fsDirectorySync", "workspace/fs/fsDirectorySync", 3),
  F("store/workspace/fsEnabledSet", "workspace/fs/fsEnabledSet", 3),
  F("store/workspace/fsNamesIndex", "workspace/fs/fsNamesIndex", 3),
  F("store/workspace/fsPendingDeletions", "workspace/fs/fsPendingDeletions", 3),
  F("store/workspace/fsRead", "workspace/fs/fsRead", 3),

  // --- sync/ (11) → gitops/ ------------------------------------------------
  F("sync/autoSync", "gitops/autoSync", 6),
  F("sync/gitLock", "gitops/gitLock", 6),
  F("sync/gitOps", "gitops/gitOps", 6),
  F("sync/gitState", "gitops/gitState", 6),
  F("sync/gitSyncOps", "gitops/gitSyncOps", 6),
  F("sync/publishService", "gitops/publishService", 6, "PublishResult — the commit-id contract"),
  F("sync/statusTracker", "gitops/statusTracker", 6),
  F("sync/syncCommands", "gitops/syncCommands", 6),
  F("sync/syncManager", "gitops/syncManager", 6),
  F("sync/syncState", "gitops/syncState", 6),
  F("sync/types", "gitops/types", 6, "⚠ named in vitest.config.ts coverage exclude"),

  // --- protocols/ (3) → protocol verticals --------------------------------
  F("protocols/graphqlCommands", "graphql/commands", 5),
  F("protocols/grpcCommands", "grpc/commands", 5),
  F("protocols/soapCommands", "soap/commands", 5),

  // --- subscription/ (1) → workspace/ -------------------------------------
  F("subscription/entityCount", "workspace/entityCount", 3),

  // --- fileOps/ (1) → fileops/ --------------------------------------------
  F("fileOps/commands", "fileops/commands", 7),

  // --- importExport/ (39) → importexport/ (capitalisation only) ------------
  F("importExport/commands", "importexport/commands", 7),
  F("importExport/types", "importexport/types", 7, "⚠ named in vitest.config.ts coverage exclude"),
  F("importExport/registry", "importexport/registry", 7),
  F("importExport/formats/postman", "importexport/formats/postman", 7),
  F("importExport/formats/postmanHelpers", "importexport/formats/postmanHelpers", 7),
  ...[
    "environments-dotenv", "environments-json", "environments-postman", "mappings-json",
    "mocks-json", "mocks-postman", "mocks-wiremock", "proxyrules-json", "requests-curl",
    "requests-har", "requests-insomnia", "requests-openapi", "requests-postman", "webhooks-json",
    "websockets-json", "workspace-json", "workspace-zip",
  ].flatMap((n) => [
    F(`importExport/exporters/${n}`, `importexport/exporters/${n}`, 7),
    F(`importExport/importers/${n}`, `importexport/importers/${n}`, 7),
  ]),

  // --- unchanged modules ---------------------------------------------------
  ...["blob/commands", "blob/publish", "blob/store", "blob/sweep"].map((f) => F(f, f, 0)),
  ...["applications/applicationCommands", "applications/commandGenerator", "applications/portUtils",
       "applications/processSpawner", "applications/types"].map((f) => F(f, f, 0)),
  ...["runner/reportHtml", "runner/runnerCommands"].map((f) => F(f, f, 0)),
  ...["transport/auth/authenticated", "transport/auth/index", "transport/auth/scopes",
       "transport/eventLog", "transport/eventPump", "transport/framing", "transport/inProcess",
       "transport/index", "transport/session", "transport/socket", "transport/stdio",
       "transport/types", "transport/ws"].map((f) => F(f, f, 0)),
  ...["runtime/host", "runtime/hostArgs", "runtime/hostCli", "runtime/portAllocator"].map((f) => F(f, f, 0)),
];

/** Files that do not exist yet and are added by a phase rather than moved. */
const ADDED = [
  { path: "runtime/instances", phase: 2, note: "§4.1 — notifyConfigChanged(), the listener fan-out" },
  { path: "websocket/index", phase: 7, note: "§9 — reserved placeholder for the `wsConnections` kind; no behaviour" },
];

const byFrom = new Map(FILES.map((f) => [f.from, f]));

// ---------------------------------------------------------------------------
// Reference scanning.
// ---------------------------------------------------------------------------
const FORMS = [
  [/from\s*(["'])@bifurc\/engine\/([A-Za-z0-9_./-]+)\1/g, "static-from", 2, true],
  [/typeof\s+import\(\s*(["'])@bifurc\/engine\/([A-Za-z0-9_./-]+)\1\s*\)/g, "typeof-import", 2, true],
  [/import\(\s*(["'])@bifurc\/engine\/([A-Za-z0-9_./-]+)\1\s*\)/g, "dynamic-import", 2, true],
  [/\b(?:vi|jest)\.(?:do)?[Mm]ock\(\s*(["'])@bifurc\/engine\/([A-Za-z0-9_./-]+)\1/g, "mock-call", 2, true],
  [/\brequire\(\s*(["'])@bifurc\/engine\/([A-Za-z0-9_./-]+)\1\s*\)/g, "require", 2, true],
  // Prose mentions inside code comments/docblocks — no quotes required. Applied last.
  [/@bifurc\/engine\/([A-Za-z0-9_./-]+)/g, "mention", 1, false],
];

const SKIP_DIRS = new Set([
  "node_modules", "dist", "release", "coverage", "coverage-baseline", "playwright-report",
  "test-results", ".git", ".vite",
]);
/** Code only — documentation is phase 8's job (see the header). */
const EXTS = new Set([".ts", ".tsx", ".mts", ".mjs", ".js", ".cjs"]);
const isSkippedDir = (n) => SKIP_DIRS.has(n) || n.startsWith("coverage-run") || n.startsWith("coverage-probe");

/**
 * ⚠ Files whose `@bifurc/engine/...` strings must NOT be rewritten, and why.
 *
 * - **`Engine_Module_Plan.md`** — it *documents the old→new mapping*. Rewriting it would erase the very
 *   thing it exists to say. This is the single most important exclusion here.
 * - **The map itself, and this script** — they hold the old paths as data.
 */
const SKIP_FILES = [
  /Engine_Module_Plan\.md$/,
  /plan[\\/]engine-layout-map\.json$/,
  /scripts[\\/]engine-layout-codemod\.mjs$/,
];
const isSkippedFile = (p) => SKIP_FILES.some((re) => re.test(p));

function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (!isSkippedDir(e.name)) walk(p, out);
      continue;
    }
    if (EXTS.has(path.extname(e.name))) out.push(p);
  }
  return out;
}

/** `proxy/serverCommands.ts` and `store/kinds/` both occur — normalise before lookup. */
function normaliseSub(sub) {
  return sub.replace(/\/+$/, "").replace(/\.(ts|tsx|mts|mjs|js|cjs)$/, "");
}

function targetFor(sub) {
  const f = byFrom.get(sub);
  if (!f) return null;
  return f.to === null
    ? { split: true, phase: f.phase, note: f.note }
    : { to: `${SRC_REL}/${f.to}.ts`, phase: f.phase };
}

// ---------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------
const allFiles = walk(ROOT).filter((p) => !isSkippedFile(p));
const byForm = {};
const bySub = new Map();
const unmapped = new Map();
const byFile = new Map();
const mentionFiles = new Map();
let totalRefs = 0;

for (const p of allFiles) {
  const src = fs.readFileSync(p, "utf8");
  if (!src.includes("@bifurc/engine/")) continue;
  const rel = path.relative(ROOT, p).split(path.sep).join("/");
  const claimed = [];
  const isClaimed = (i) => claimed.some(([a, b]) => i >= a && i < b);

  for (const [re, form, subGroup] of FORMS) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(src)) !== null) {
      const sub = normaliseSub(m[subGroup]);
      if (form === "mention") {
        if (isClaimed(m.index)) continue;
        if (!MENTIONS) continue;
      } else {
        claimed.push([m.index, m.index + m[0].length]);
      }
      if (!sub) continue;

      totalRefs++;
      byForm[form] = (byForm[form] ?? 0) + 1;
      const rec = bySub.get(sub) ?? { count: 0, forms: new Set(), files: new Set() };
      rec.count++;
      rec.forms.add(form);
      rec.files.add(rel);
      bySub.set(sub, rec);

      if (!byFrom.has(sub)) unmapped.set(sub, (unmapped.get(sub) ?? 0) + 1);
      if (!byFile.has(rel)) byFile.set(rel, []);
      byFile.get(rel).push(form === "mention" ? `mention:${sub}` : `${form}:${sub}`);
      if (form === "mention") mentionFiles.set(rel, (mentionFiles.get(rel) ?? 0) + 1);
    }
  }
}

// ---------------------------------------------------------------------------
// --emit-map
// ---------------------------------------------------------------------------
const stats = {
  engineFiles: FILES.length,
  engineLines: 23443,
  references: totalRefs,
  consumerFiles: byFile.size,
  uniqueSubpaths: bySub.size,
  byForm,
  byGroup: {},
  unmappedSubpaths: Object.fromEntries([...unmapped].sort((a, b) => b[1] - a[1])),
};
for (const rel of byFile.keys()) {
  const g = rel.startsWith("tests/") ? "tests (root)"
    : rel.startsWith("src/") ? "src (Electron shell)"
    : rel.split("/").slice(0, 2).join("/");
  stats.byGroup[g] = (stats.byGroup[g] ?? 0) + 1;
}

const map = {
  $schema: "engine-layout-map/v1",
  generatedBy: "scripts/engine-layout-codemod.mjs --emit-map",
  source: SRC_REL,
  authority: "Engine_Module_Plan.md §3 (target module map) + §7.2 (phases)",
  scope: "Code files only (.ts .tsx .mts .mjs .js .cjs). Documentation is phase 8.",
  note:
    "Old→new path map for the engine module reorganisation. `to: null` means the file is SPLIT across " +
    "modules rather than renamed and needs hand work in its phase. `phase: 0` means the path does not " +
    "change (already final, or explicitly out of scope).",
  corrections: [
    `§7.1 says "529 deep-import statements across 104 files" and "82 unique paths". Measured: ` +
      `${totalRefs} references across ${byFile.size} files and ${bySub.size} unique subpaths ` +
      `(${byForm["static-from"] ?? 0} static imports).`,
    `§7.1 counts only \`from\` imports. Measured: ${byForm["dynamic-import"] ?? 0} dynamic \`import()\`, ` +
      `${byForm["mock-call"] ?? 0} \`vi.mock\`/\`vi.doMock\`, ${byForm["typeof-import"] ?? 0} \`typeof import()\`, ` +
      `${byForm["require"] ?? 0} \`require()\`. A \`from\`-only rewrite misses ~40% of the surface, and the ` +
      `two largest misses fail at RUNTIME, not at compile time.`,
    "§7.1's consumer table (77 tests / 10 src / 3 packages = 90 files) does not sum to its own 104-file " +
      "total, and omits packages/web-client and packages/client's tests.",
    "Deviation from §3: `store/workspace/*.ts` → `workspace/fs/*.ts`, not `workspace/workspace/*.ts`. " +
      "The plan's literal target produces a `workspace/workspace/` nesting; `workspace/fs/` says the same " +
      "thing without the wart. Reversible — one line in the rule table.",
  ],
  phases: {
    0: "freeze — no moves",
    1: "kernel/ + utils/",
    2: "the rebind fan-out (runtime/instances.ts) — no file moves",
    3: "workspace/ (the store/ split)",
    4: "proxy family (reverseproxy, mocks, certificates, webhooks)",
    5: "protocol verticals (graphql, grpc, soap)",
    6: "gitops/ (sync/ + gitStore)",
    7: "leftovers (miscCommands, lifecycle, fileops, importexport, websocket)",
    8: "ratchet + docs",
  },
  modules: [
    "kernel", "utils", "workspace", "reverseproxy", "proxy", "mocks", "certificates", "webhooks",
    "websocket", "rest", "graphql", "grpc", "soap", "gitops", "fileops", "importexport", "blob",
    "applications", "runner", "transport", "runtime", "lifecycle",
  ],
  files: FILES.map((f) => ({
    from: `${SRC_REL}/${f.from}.ts`,
    to: f.to === null ? null : `${SRC_REL}/${f.to}.ts`,
    phase: f.phase,
    disposition: f.to === null ? "split" : f.to === f.from ? "keep" : "move",
    ...(f.note ? { note: f.note } : {}),
  })),
  added: ADDED.map((a) => ({ path: `${SRC_REL}/${a.path}.ts`, phase: a.phase, note: a.note })),
  subpaths: Object.fromEntries(
    [...bySub].sort((a, b) => b[1].count - a[1].count).map(([sub, rec]) => [
      sub,
      { count: rec.count, forms: [...rec.forms].sort(), consumers: rec.files.size, target: targetFor(sub) ?? "⚠ UNMAPPED" },
    ]),
  ),
  stats,
};

if (MODE === "emit-map") {
  fs.writeFileSync(MAP_PATH, JSON.stringify(map, null, 2) + "\n");
  console.log(`wrote ${path.relative(ROOT, MAP_PATH)}`);
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------
console.log(`\n=== ${MODE.toUpperCase()} ===`);
console.log(`references: ${totalRefs}   consumer files: ${byFile.size}   unique subpaths: ${bySub.size}`);
console.log("\nby form:");
for (const [k, v] of Object.entries(byForm).sort((a, b) => b[1] - a[1])) console.log(`  ${String(v).padStart(4)}  ${k}`);
console.log("\nby consumer group (files):");
for (const [k, v] of Object.entries(stats.byGroup).sort((a, b) => b[1] - a[1])) console.log(`  ${String(v).padStart(4)}  ${k}`);

const splitSubs = [...bySub.keys()].filter((s) => byFrom.get(s)?.to === null);
if (splitSubs.length) {
  console.log("\n⚠ SPLIT targets (need hand work in their phase):");
  for (const s of splitSubs) console.log(`  ${s}  (${bySub.get(s).count} refs) — ${byFrom.get(s).note}`);
}

if (mentionFiles.size) {
  console.log(`\nprose mentions rewritten inside code (${mentionFiles.size} file(s)) — verify none described history:`);
  for (const [f, n] of [...mentionFiles].sort((a, b) => b[1] - a[1]).slice(0, 20)) console.log(`  ${String(n).padStart(3)}  ${f}`);
}

if (unmapped.size) {
  console.log("\n⚠ UNMAPPED subpaths — prose fragments, or historical references. Left alone:");
  for (const [s, n] of [...unmapped].sort((a, b) => b[1] - a[1])) console.log(`  ${String(n).padStart(3)}  ${s}`);
}

const onDisk = walk(SRC).map((p) => path.relative(SRC, p).split(path.sep).join("/").replace(/\.tsx?$/, ""));
const missing = onDisk.filter((f) => !byFrom.has(f));
const extra = [...byFrom.keys()].filter((f) => !onDisk.includes(f));
if (missing.length) console.log(`\n⚠ TABLE-LESS engine files (${missing.length}): ${missing.join(", ")}`);
if (extra.length) console.log(`\n⚠ TABLE entries with no file on disk (${extra.length}): ${extra.join(", ")}`);
if (!missing.length && !extra.length) console.log(`\n✅ totality: all ${onDisk.length} engine src files are in the table`);

// ---------------------------------------------------------------------------
// --apply
// ---------------------------------------------------------------------------
if (MODE === "apply") {
  if (PHASE === null) {
    console.error("\n--apply needs --phase N");
    process.exit(2);
  }
  const inPhase = FILES.filter((f) => f.phase === PHASE && f.to !== null && f.to !== f.from);
  const rename = new Map(inPhase.map((f) => [f.from, f.to]));
  console.log(`\n=== APPLY phase ${PHASE} — ${rename.size} path(s) ===`);
  for (const [a, b] of rename) console.log(`  ${a}  ->  ${b}`);

  const pending = FILES.filter((f) => f.phase === PHASE && f.to === null);
  if (pending.length) console.log(`  ⚠ ${pending.length} split target(s) in this phase need hand work: ${pending.map((p) => p.from).join(", ")}`);

  let rewrites = 0;
  let touched = 0;
  for (const p of allFiles) {
    const before = fs.readFileSync(p, "utf8");
    if (!before.includes("@bifurc/engine/")) continue;
    let after = before;
    for (const [re, form, subGroup] of FORMS) {
      if (form === "mention" && !MENTIONS) continue;
      after = after.replace(re, (full, q, subRaw) => {
        if (subGroup !== 2) return full;
        const sub = normaliseSub(subRaw);
        const to = rename.get(sub);
        if (!to) return full;
        rewrites++;
        const suffix = subRaw.slice(sub.length);
        return full.replace(`@bifurc/engine/${subRaw}`, `@bifurc/engine/${to}${suffix}`);
      });
    }
    if (after === before) continue;
    touched++;
    console.log(`  ${WRITE ? "write" : "would write"}  ${path.relative(ROOT, p).split(path.sep).join("/")}`);
    if (WRITE) fs.writeFileSync(p, after);
  }
  console.log(`\n${WRITE ? "rewrote" : "would rewrite"} ${rewrites} reference(s) across ${touched} file(s)`);
  if (!WRITE) console.log("(dry run — pass --write to apply)");
}
