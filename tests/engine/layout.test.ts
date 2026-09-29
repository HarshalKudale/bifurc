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
 * are explicit exceptions. §6 also had **no row at all** for `fileops/`, `importexport/`, `blob/`,
 * `applications/`, `runner/`, `websocket/` or `lifecycle/`, so `CONTRACT` below adds them — each at the
 * §5.1 default (`kernel/`, `utils/`, `workspace/`) unless §6 already said otherwise.
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
  reverseproxy: ["kernel", "utils", "workspace", "mocks"],
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
  blob: ["kernel", "utils", "workspace"],
  applications: ["kernel", "utils", "workspace"],
  runner: ["kernel", "utils", "workspace"],
  transport: ["kernel"],
  runtime: "any",
  lifecycle: "any",
};

/** §4.1's seam — importable from anywhere, because its whole purpose is to avoid a sideways import. */
/** The engine's **seam modules** — importable from anywhere, because each exists to avoid a sideways import. */
const SEAM_MODULES = new Set([
  "runtime/instances",
  "runtime/entityStatus",
]);

/** Modules with no importer *by design*, and why. */
const ORPHANS_ALLOWED: Record<string, string> = {
  websocket:
    "§9 reserved placeholder — no behaviour, so nothing imports it. When the first WebSocket code lands, " +
    "this entry must be deleted and the module wired.",
};

/**
 * Edges that break `CONTRACT` today. **Keyed by `src`-relative `file -> file`**, so each entry names the
 * exact import rather than a module pair — a module pair would silently cover the next violation too.
 */
const ALLOWED_VIOLATIONS: Record<string, string> = {
  // ── utils/ is not pure for two files phase 4 moved into it ──────────────────────────────────────
  "utils/response.ts -> workspace/config.ts":
    "`utils/response.ts` came from `proxy/responseUtils.ts` in phase 4, on the reasoning that both proxy " +
    "halves need it. It is not pure: it reads the proxy rules to decide what to serve. Either it belongs " +
    "back in a feature module or `utils/`'s §5.1 definition ('pure and stateless, no filesystem, no config " +
    "reads') has to change. Phase 4's placement is the thing under question, not this import.",
  "utils/response.ts -> reverseproxy/pages.ts":
    "Same file, second dependency — it serves the reverse-proxy home page. See the entry above.",
  "utils/response.ts -> mocks/scriptExecutor.ts":
    "Same file, third dependency — it runs request/response scripts. See the entry above.",
  "utils/server.ts -> workspace/config.ts":
    "`utils/server.ts` came from `proxy/serverUtils.ts` in phase 4 and reads `loadConfig()` plus the " +
    "enabled sets, so it is stateful persistence-adjacent code sitting in the 'pure' layer. Same question " +
    "as `response.ts`: the placement, not the import.",
  "utils/server.ts -> workspace/workspaceFs.ts": "Same file, second dependency — see the entry above.",

  // ── transport → utils (one line, and the one §6's transport row forbids) ────────────────────────
  "transport/ws.ts -> utils/net.ts":
    "`isLoopbackHost` for the transport's bind guard. `utils/net.ts` exists precisely because `plan/15` work " +
    "item 2 gave the proxy a second bind guard needing the same answer, and it was extracted so the two " +
    "would not drift. §6's `transport/` row predates that and says `kernel/` only; the honest fix is to " +
    "widen the row, not to duplicate the two-element list.",

  // ── workspace → gitops: commands need the git handle ────────────────────────────────────────────
  "workspace/entities/auditCommands.ts -> gitops/gitStore.ts":
    "`audit.list` reads a workspace's commit history, so the command needs the git handle. The generic CRUD " +
    "commands are `workspace/`-side because they own persistence, but git access is a `gitops/` capability.",
  "workspace/entities/configCommands.ts -> gitops/gitStore.ts":
    "Same shape — `workspace.delete` removes the workspace's repository.",
  "workspace/entities/entityCrudCommands.ts -> gitops/gitStore.ts":
    "Same shape — entity delete branches on git-tracking to decide whether the deletion has to be published.",

  // ── the two proxy halves import each other ──────────────────────────────────────────────────────
  "proxy/proxyHandler.ts -> reverseproxy/pages.ts":
    "`sendHtml` for the local 404/home response. §6's `proxy/` row grants `mocks/` and `certificates/` but " +
    "not `reverseproxy/`; the two halves are mutually dependent (see the `reverseproxy → proxy` entry), so " +
    "one of the pair is a genuine cycle.",
  "proxy/routingUtils.ts -> reverseproxy/pages.ts": "Same helper, second caller — see the entry above.",
  "proxy/server.ts -> reverseproxy/pages.ts": "Same helper, third caller — see the entry above.",
  "reverseproxy/edge.ts -> proxy/proxyHandler.ts":
    "The reverse half dispatches matched mappings into the forward half's handler. This is the other side of " +
    "the cycle named above; §6's `reverseproxy/` row does not grant `proxy/`.",

  // ── fileops/ is a command aggregator, like registerAll.ts ───────────────────────────────────────
  "fileops/commands.ts -> blob/store.ts":
    "`fileops/commands.ts` serves `file.*`, `tls.*`, `audit.export` and `runner.exportReport` — a command " +
    "surface that spans blob storage, certificates, git and the runner. §5.1 reserves the 'may import many " +
    "features' exception for `registerAll.ts`; this file is a second instance of it and deserves the same " +
    "explicit treatment rather than a silent exception.",
  "fileops/commands.ts -> blob/publish.ts": "Same aggregator — see the entry above.",
  "fileops/commands.ts -> certificates/certManager.ts": "Same aggregator — see the entry above.",
  "fileops/commands.ts -> gitops/gitStore.ts": "Same aggregator — see the entry above.",
  "fileops/commands.ts -> runner/reportHtml.ts": "Same aggregator — see the entry above.",

  // ── importexport → blob + gitops ────────────────────────────────────────────────────────────────
  "importexport/commands.ts -> blob/store.ts":
    "`import.export`/`import.apply` stream through the blob store, and the workspace importer needs the git " +
    "handle to re-init a repository. Both are real capabilities this module consumes.",
  "importexport/commands.ts -> blob/publish.ts": "Same module — see the entry above.",
  "importexport/importers/workspace-json.ts -> gitops/gitStore.ts": "Same module — see the entry above.",
  "importexport/importers/workspace-zip.ts -> gitops/gitStore.ts": "Same module — see the entry above.",
};

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
