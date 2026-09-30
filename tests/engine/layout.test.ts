/**
 * The engine's **module-layout ratchet** — `Engine_Module_Plan.md` §5.2.
 *
 * ## What this is for
 *
 * §5.1 draws a layer order and §6 gives each module a contract ("May import …"). Before this file, both
 * were documentation: nothing failed when an import crossed a layer, so the rule decayed exactly as §5.2
 * predicts — *"Both must be added in the same phase as the first move, or the rule decays."* This test is
 * the enforcement. It parses every relative import under `packages/engine/src`, maps each file to its
 * module, and fails with the offending `file → file` pair.
 *
 * ## §5.1 and §6 disagreed, and §6 won
 *
 * §5.1 says feature modules may **never** import each other. §6's table grants `proxy/` → `mocks/` +
 * `certificates/` and `reverseproxy/` → `mocks/` outright. The two cannot both be the rule. **§6 governs**
 * (decided 2026-09-29): §5.1's blanket ban is the *default where §6 is silent*, and §6's per-module grants
 * are explicit exceptions. §6 also had **no row at all** for `fileops/`, `importexport/`,
 * `applications/`, `runner/`, `websocket/` or `lifecycle/`, so `CONTRACT` below adds them — each at the
 * §5.1 default (`kernel/`, `utils/`, `workspace/`) unless §6 already said otherwise.
 *
 * **`blob/` is not in the table, because it is not a top-level module any more.** The layout cleanup
 * moved it to `workspace/blob/`: it is a disk-backed store under the data dir with a TTL sweeper, i.e.
 * persistence, and `fileops/`/`importexport/`/`certificates/`/`runner/` all reach it through the one
 * import §6 already grants everything.
 *
 * ## Two exemptions, both principled
 *
 * 1. **`import type … from`** is erased at compile time. It creates no runtime edge, so it is not a layer
 *    violation. `kernel/eventBus.ts → webhooks/webhookServer.ts` is the one that relies on this.
 * 2. **The two seam modules are importable from anywhere.** `runtime/instances` is §4.1's listener fan-out
 *    (20 of the engine's edges are that one import, which is the seam working rather than 20 violations);
 *    `runtime/entityStatus` is the read-and-emit composition. §6's `gitops/` row already granted the first;
 *    this generalises the idea and names the set explicitly, so a third seam module is a deliberate act.
 *
 * Root files (`index.ts`, `registerAll.ts`) are exempt: they are the composition root, and §5.1 names
 * `registerAll.ts` as "the single exception that may import every feature module".
 *
 * ## The allowlist is debt, and it can only shrink
 *
 * 22 file-level edges currently break the contract (it was 24 before `kernel/eventBus.ts` was fixed). Each is listed below with its reason, and
 * `the allowlist has no stale entries` fails if one is fixed without being removed — so the list cannot
 * quietly become a permission slip.
 *
 * **The worst one is already fixed, and it is worth keeping as the worked example.** When this file first
 * ran, `kernel/eventBus.ts` imported `getWorkspaceSyncStatus` and `getWorkspaceGitState` from `gitops/` —
 * two **values** — which made §5.1's "`kernel/` imports nothing engine-internal" false, and §5.2 calls that
 * property what makes the rule "mechanically checkable rather than aspirational". `emitEntityStatus()` moved
 * to **`runtime/entityStatus.ts`** (the second seam module), because three of its four callers are
 * `workspace/`-side and §6 lets `workspace/` import only `kernel/` and `utils/` — so relocating it sideways
 * would have turned two `kernel → gitops` edges into three `workspace → gitops` ones and changed nothing.
 * `kernel/eventBus.ts` now has exactly one engine-internal import and it is a type. **Its two allowlist
 * entries were deleted in the same commit, which is the shrink this test exists to force.**
 */
import fs from "node:fs";
import path from "node:path";
import { describe, it, expect } from "vitest";

const ROOT = process.cwd();
const SRC = path.join(ROOT, "packages", "engine", "src");

/**
 * §6's contract table, extended with rows for the modules §6 omitted.
 *
 * `"any"` means the module composes and may import anything: §5.1 puts `runtime/` and `lifecycle/` last in
 * the chain, and §6 says `runtime/` may import "everything except `index`".
 */
const CONTRACT: Record<string, readonly string[] | "any"> = {
  kernel: [],
  utils: ["kernel"],
  workspace: ["kernel", "utils"],
  // `proxy/` is granted: the reverse half dispatches a matched mapping into the forward half's upstream
  // request machinery (`proxyToUpstream` / `fetchUpstreamResponse`), which is the split `plan/15` WI 3
  // drew. **One-way** — `proxy/` no longer imports `reverseproxy/` (the shared `sendHtml` was extracted
  // down to `utils/`), so this is a dependency, not a cycle.
  reverseproxy: ["kernel", "utils", "workspace", "mocks", "proxy"],
  proxy: ["kernel", "utils", "workspace", "mocks", "certificates"],
  mocks: ["kernel", "utils", "workspace"],
  certificates: ["kernel", "utils", "workspace"],
  webhooks: ["kernel", "utils", "workspace"],
  graphql: ["kernel", "utils", "workspace"],
  grpc: ["kernel", "utils", "workspace"],
  soap: ["kernel", "utils", "workspace"],
  rest: ["kernel", "utils", "workspace"],
  // Reserved (§9): the directory exists, the behaviour does not. The contract is the default so that the
  // first real code here starts under the rule rather than outside it.
  websocket: ["kernel", "utils", "workspace"],
  gitops: ["kernel", "utils", "workspace"],
  fileops: ["kernel", "utils", "workspace"],
  importexport: ["kernel", "utils", "workspace"],
  applications: ["kernel", "utils", "workspace"],
  runner: ["kernel", "utils", "workspace"],
  // `plan/19` D24's request pipeline, and `request.send` — the command that drives it. The row is the
  // union of what the module reads: the workspace's environment state and entities (`workspace/`), the
  // resolver (`utils/`), the script sandbox (`mocks/`) and the registry the command dispatches through
  // (`kernel/`).
  //
  // **`kernel/` was added 2026-09-30 with `pipeline/commands.ts`,** and that addition is why the file
  // exists at all: a command that calls the pipeline is a `workspace/ → pipeline/` edge, and `workspace/`'s
  // §6 row is exactly `kernel/` + `utils/`. §5.1's remedy is to place a module in the layer it reads from,
  // so `request.send` moved out of `savedRequestCommands.ts` to the pipeline it drives — the ratchet
  // forced a *placement*, not just a permission.
  //
  // `mocks/` is a **sideways** feature→feature edge and it is granted deliberately, following §6's
  // existing `proxy/` → `mocks/` and `reverseproxy/` → `mocks/` rows: the sandbox is stateless and shared,
  // and the §5.1 remedy — extract it *down* into `utils/` — is the cleanup to make if a third consumer
  // appears. If that happens, `scriptContext.ts` must move with it: `utils/` importing `mocks/` would be
  // the very violation the extraction exists to avoid.
  pipeline: ["kernel", "utils", "workspace", "mocks"],
  capture: ["kernel", "utils", "workspace"],
  // `utils/` is granted for `hostnameOf`/`isLoopbackHost`: §6's row for this module predates `plan/15`
  // work item 2, which gave the proxy a second bind guard needing the same answer and extracted
  // `utils/net.ts` so the two could not drift. The row is the thing that was out of date, not the import.
  transport: ["kernel", "utils"],
  runtime: "any",
  lifecycle: "any",
};

/** §4.1's seam — importable from anywhere, because its whole purpose is to avoid a sideways import. */
/** The engine's **seam modules** — importable from anywhere, because each exists to avoid a sideways import. */
const SEAM_MODULES = new Set([
  "runtime/instances",
  "runtime/entityStatus",
]);

/**
 * Modules with no importer *by design*, and why. **`{}` — empty, and both entries left the way their own
 * text said they would.**
 *
 * - **`pipeline` was here for part of 2026-09-30.** It was added when the pipeline function landed with no
 *   caller, and the entry itself said to delete it when the `*.send` command arrived — which it did, the
 *   same day.
 * - **`websocket` was here from `Engine_Module_Plan.md` §9's reservation until `plan/19` WI 6.** Its entry
 *   read *"reserved placeholder — no behaviour, so nothing imports it. When the first WebSocket code lands,
 *   this entry must be deleted and the module wired"*, and that is exactly what happened: `registerAll.ts`
 *   now imports `websocket/wsCommands`, so the module is not an orphan and the permission would have been
 *   inert.
 *
 * Both removals were **forced** by the assertion below rather than remembered, which is the point of having
 * it: an `ORPHANS_ALLOWED` entry outliving its reason is otherwise **inert** — nothing else reads this map,
 * so the permission would sit there describing a module that is no longer an orphan, and it would
 * pre-excuse the next orphan that appeared under that name.
 *
 * **The map stays empty rather than being deleted**, for the same reason `ALLOWED_VIOLATIONS` below stays:
 * the two assertions that read it are what stop the next orphan from arriving as an exception.
 */
const ORPHANS_ALLOWED: Record<string, string> = {};

/**
 * Edges that break `CONTRACT` today. **Keyed by `src`-relative `file -> file`**, so each entry names the
 * exact import rather than a module pair — a module pair would silently cover the next violation too.
 */
/**
 * **Empty, and that is the goal.** Every one of the 24 violations the ratchet found when it first ran has
 * been fixed rather than excused — 22 in this cleanup, and the `kernel/eventBus.ts → gitops/` pair before
 * it. The object stays because the two assertions that read it are what stop the *next* violation from
 * arriving as an exception: an unknown edge fails `every relative import obeys the §6 contract`, and an
 * entry here that no longer describes one fails `the allowlist has no stale entries`.
 */
const ALLOWED_VIOLATIONS: Record<string, string> = {};

// ── graph construction ────────────────────────────────────────────────────────────────────────────

function walk(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) return walk(p);
    return p.endsWith(".ts") ? [p] : [];
  });
}

/** `packages/engine/src/<module>/...` → `<module>`; a file directly in `src/` → `(root)`. */
function moduleOf(rel: string): string {
  const parts = rel.split("/");
  return parts.length === 1 ? "(root)" : parts[0];
}

/**
 * Every import in `file`, with whether it is `import type`.
 *
 * Accumulates across lines so a multi-line `import type { … } from "…"` is classified correctly — a
 * line-at-a-time scan reads the closing `} from "…";` and cannot see the `type`, which would report a
 * type-only import as a runtime edge.
 */
function importsOf(file: string): { spec: string; typeOnly: boolean }[] {
  const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
  const out: { spec: string; typeOnly: boolean }[] = [];
  let pending: string | null = null;

  for (const line of lines) {
    const t = line.trim();
    if (pending === null) {
      if (!/^import\b/.test(t)) continue;
      pending = t;
    } else {
      pending += " " + t;
    }
    const complete =
      /from\s*["'][^"']+["']/.test(pending) || /^import\s*["']/.test(pending) || /;\s*$/.test(pending);
    if (!complete) continue;
    const fm = pending.match(/from\s*["']([^"']+)["']/);
    if (fm) out.push({ spec: fm[1], typeOnly: /^import\s+type\b/.test(pending) });
    pending = null;
  }
  // Dynamic `import("…")` is always a runtime edge.
  for (const line of lines) {
    const m = line.match(/\bimport\s*\(\s*["']([^"']+)["']\s*\)/);
    if (m) out.push({ spec: m[1], typeOnly: false });
  }
  return out;
}

type Edge = { from: string; to: string; fromMod: string; toMod: string };

/** Every engine-internal relative import, minus the two exemptions. */
function engineEdges(): Edge[] {
  const edges: Edge[] = [];
  for (const abs of walk(SRC)) {
    const from = path.relative(SRC, abs).split(path.sep).join("/");
    for (const imp of importsOf(abs)) {
      if (!imp.spec.startsWith(".")) continue;
      if (imp.typeOnly) continue; // exemption 1
      const target = path.resolve(path.dirname(abs), imp.spec);
      if (!target.startsWith(path.resolve(SRC))) continue;
      const to = path.relative(SRC, target).split(path.sep).join("/") + ".ts";
      if (SEAM_MODULES.has(to.replace(/\.ts$/, ""))) continue; // exemption 2
      edges.push({ from, to, fromMod: moduleOf(from), toMod: moduleOf(to) });
    }
  }
  return edges;
}

const EDGES = engineEdges();

function contractViolations(): Edge[] {
  return EDGES.filter((e) => {
    if (e.fromMod === "(root)" || e.toMod === "(root)" || e.fromMod === e.toMod) return false;
    const allowed = CONTRACT[e.fromMod];
    if (allowed === undefined) return false; // unknown module — covered by its own test
    if (allowed === "any") return false;
    return !allowed.includes(e.toMod);
  });
}

const key = (e: Edge) => `${e.from} -> ${e.to}`;

// ── the ratchet ───────────────────────────────────────────────────────────────────────────────────

describe("engine module layout (§5.1/§5.2)", () => {
  it("every module named in the contract is a real directory", () => {
    const dirs = fs
      .readdirSync(SRC, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => e.name);
    const missing = Object.keys(CONTRACT).filter((m) => !dirs.includes(m));
    expect(
      missing,
      `the contract names modules that do not exist: ${missing.join(", ")}. A renamed module must be ` +
        `renamed here too, or its files silently fall outside the rule.`,
    ).toEqual([]);
  });

  it("every real directory is named in the contract", () => {
    // The **converse** of the assertion above, and it was missing until `plan/19` WI 3 added `pipeline/`.
    // A new module directory that no row names is invisible to this entire file: `contractViolations()`
    // returns `false` for an unknown `fromMod`, so none of its imports are ever checked, and the orphan
    // test only iterates `CONTRACT`'s keys. Adding a directory was therefore the one way to escape the
    // layering rule silently — which is precisely the decay §5.2 exists to prevent, so the rule needs
    // teeth in this direction too.
    const dirs = fs
      .readdirSync(SRC, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => e.name);
    const unnamed = dirs.filter((d) => !(d in CONTRACT));
    expect(
      unnamed,
      `${unnamed.length} director(ies) under packages/engine/src are not named in CONTRACT: ` +
        `${unnamed.join(", ")}. Add a row — an unnamed module's imports are never checked, so the ` +
        `layering rule silently stops applying to it the moment the directory exists.`,
    ).toEqual([]);
  });

  it("every relative import obeys the §6 contract, allowlist aside", () => {
    const unexpected = contractViolations()
      .filter((e) => !(key(e) in ALLOWED_VIOLATIONS))
      .map((e) => `${key(e)}   [${e.fromMod} may not import ${e.toMod}]`);
    expect(
      unexpected,
      `${unexpected.length} import(s) cross a layer boundary and are not in ALLOWED_VIOLATIONS. ` +
        `Either fix the import (extract the shared behaviour down, per §5.1) or add it to the allowlist ` +
        `with a reason. See Engine_Module_Plan.md §5.1/§6.`,
    ).toEqual([]);
  });

  it("the allowlist has no stale entries — it can only shrink", () => {
    const live = new Set(contractViolations().map(key));
    const stale = Object.keys(ALLOWED_VIOLATIONS).filter((k) => !live.has(k));
    expect(
      stale,
      `${stale.length} allowlist entr(ies) no longer describe a violation — the import was fixed or the ` +
        `file moved. Delete the entr(ies); the allowlist must not accumulate permissions for edges that ` +
        `are already legal.`,
    ).toEqual([]);
  });

  it("no contract module is orphaned", () => {
    const imported = new Set(EDGES.map((e) => e.toMod));
    // `kernel`/`utils`/`workspace` are leaves that everything imports; a module is only an orphan if
    // nothing imports it at all.
    const orphans = Object.keys(CONTRACT).filter(
      (m) => !imported.has(m) && !(m in ORPHANS_ALLOWED),
    );
    expect(
      orphans,
      `${orphans.length} module(s) are imported by nothing: ${orphans.join(", ")}. §5.2 counts an orphan ` +
        `as a finding — either wire it or record why it is empty.`,
    ).toEqual([]);
  });

  it("ORPHANS_ALLOWED has no stale entries — it can only shrink", () => {
    // The same rule as `ALLOWED_VIOLATIONS`, applied to the file's **other** permission map, and it was
    // missing until `plan/19` WI 3's `pipeline` entry outlived its reason inside a single day. Nothing
    // else reads `ORPHANS_ALLOWED`, so an entry for a module that *is* imported is **inert**: it neither
    // fails nor explains anything, and it silently pre-excuses the next orphan that appears under that
    // name. That is the same "permission slip" failure the staleness assertion for `ALLOWED_VIOLATIONS`
    // exists to prevent, one map over.
    const imported = new Set(EDGES.map((e) => e.toMod));
    const stale = Object.keys(ORPHANS_ALLOWED).filter((m) => imported.has(m));
    expect(
      stale,
      `${stale.length} ORPHANS_ALLOWED entr(ies) describe module(s) that ARE imported: ` +
        `${stale.join(", ")}. Delete the entr(ies) — an inert permission is worse than none, because it ` +
        `silently excuses a future orphan of the same name.`,
    ).toEqual([]);
  });

  it("every register*Commands owner is reachable from registerAll.ts", () => {
    // §4.3: registerAll.ts is the one file that may import every feature, and the drift guards in
    // tests/integration/workspaceHost.integration.test.ts compare the *shell's* calls against it. This
    // asserts the weaker structural half — that a module exporting a `register*Commands` is not left
    // outside the composition root entirely.
    const byFile = new Map<string, string>();
    for (const abs of walk(SRC)) {
      const rel = path.relative(SRC, abs).split(path.sep).join("/");
      const src = fs.readFileSync(abs, "utf8");
      for (const m of src.matchAll(/export\s+function\s+(register\w+Commands)\b/g)) {
        byFile.set(rel, m[1]);
      }
    }

    const reachable = new Set<string>();
    const queue = ["registerAll.ts"];
    while (queue.length) {
      const rel = queue.shift() as string;
      if (reachable.has(rel)) continue;
      reachable.add(rel);
      const abs = path.join(SRC, rel);
      if (!fs.existsSync(abs)) continue;
      for (const imp of importsOf(abs)) {
        if (!imp.spec.startsWith(".")) continue;
        const t = path.relative(SRC, path.resolve(path.dirname(abs), imp.spec)).split(path.sep).join("/");
        queue.push(t.endsWith(".ts") ? t : `${t}.ts`);
      }
    }

    const unreached = [...byFile.entries()]
      .filter(([rel]) => !reachable.has(rel))
      .map(([rel, fn]) => `${rel} (${fn})`);
    expect(
      unreached,
      `${unreached.length} module(s) export a register*Commands but are not reachable from ` +
        `registerAll.ts: ${unreached.join(", ")}. A consumer that only calls registerAllEngineCommands() ` +
        `would answer UNKNOWN_COMMAND for them.`,
    ).toEqual([]);
  });
});
