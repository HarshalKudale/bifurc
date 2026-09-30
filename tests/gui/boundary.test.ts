/**
 * The **renderer boundary ratchet** — `plan/19-engine-owned-processing.md` WI 1 (decision D22).
 *
 * ## What this is for
 *
 * The owner's rule, 2026-09-29: **the renderer holds no domain logic and no domain state.** The test to
 * apply to any line under `packages/gui/src` is *"would this line be different if the UI were a
 * terminal, a web page, or a test harness?"* — if yes, it is engine work.
 *
 * That rule was already broken five times over when it was written down (`plan/19` §1): the renderer
 * carried its own environment resolver, its own `{{random.*}}` generator, the pre/post/test pipeline
 * written **twice**, a second HTML report renderer, a second cURL parser and a second set of
 * entity-path helpers. It grew while nobody was looking, which is the property that makes a written
 * rule worthless on its own — §5.2 of `Engine_Module_Plan.md` says it plainly: *"Both must be added in
 * the same phase as the first move, or the rule decays."*
 *
 * This file is the enforcement, and it is deliberately the **first** work item of P19. Every later item
 * deletes a copy of something; without this test, "the migration is in progress" is indistinguishable
 * from "the migration added a seventh duplicate". The same idiom already works one directory over —
 * see `tests/engine/layout.test.ts`, whose allowlist was driven from 24 violations to `{}` by exactly
 * the two assertions below.
 *
 * ## What it scans, and why the constructs are defined the way they are
 *
 * It reads every `.ts`/`.tsx` file under `packages/gui/src` and looks for the constructs of §1's table.
 * Two decisions there are load-bearing:
 *
 * 1. **Detection is by *definition*, not by *use*.** `App.tsx` calls `mergeEnvVars` and
 *    `RandomizerHint.tsx` reads the token catalogue; neither is a domain *implementation*, and neither
 *    is an allowlist entry. Only a file that **declares** the behaviour is a violation. Flagging the
 *    ~25 call sites instead would produce an allowlist nobody can read and a staleness assertion that
 *    fires on every unrelated edit.
 *
 * 2. **The pipeline is the one construct with no named function**, because it is an *orchestration* —
 *    §1 rows 3 and 4 are the same five steps written at two call sites. So `pipeline` is detected by a
 *    **call** to a pre/post/test step, and the step wrappers themselves are a separate construct
 *    (`pipelineStep`). Splitting them matters: they are deleted by different work items, and an
 *    allowlist entry that cannot name its deleting WI is the thing §9's staleness rule exists to catch.
 *
 * The `authority` column is not decoration. It is where each construct must live, and the last test
 * reads it — so "fix a violation by deleting the engine's copy too" fails here rather than passing
 * quietly. Four authorities have landed already (the resolver, the randomizer, the report renderer, the
 * path helpers); two are still `planned:` and are created by WI 3 and WI 5.
 *
 * ## The allowlist is debt, and it can only shrink
 *
 * **Four entries today, in four files — and the count is a *reading*, not a target.** It started at 9 on
 * 2026-09-29, which was itself a correction of `plan/19` §6: that section predicts `−2 / −1 / −1 / −2` across
 * WI 2–5, summing to 6, because it counts *files deleted* while this file counts *constructs present* —
 * `collectionRunner.ts` carried the pipeline, the run loop and the report-renderer's only caller, and
 * `scriptRunner.ts` / `testRunner.ts` are not in §1's table at all.
 *
 * **The count has moved twice, both times downward, and that is the whole design.** 9 → 8 when
 * `useRestActions.ts` stopped orchestrating (WI 3, REST), then 8 → 4 when WI 4 deleted the run loop and
 * everything that hung off it. **The staleness assertion forced every one of the five retirements** — with the
 * construct gone the entry could not be kept, which is what makes the shrink checkable rather than remembered.
 *
 * Each entry names the work item that deletes it, and `the allowlist has no stale entries` fails if one
 * is fixed without being removed — so the list cannot quietly become a permission slip.
 *
 * ## What this does not catch, and why it is still worth having
 *
 * Detection is by **name**, so a copy re-implemented under a different name is not caught. That is a
 * real gap, recorded rather than papered over:
 *
 *  - The mistake this guards against is not an adversary — it is a contributor adding a seventh
 *    duplicate because the engine's copy was not where they looked. A name-based scan catches that, at
 *    the moment the file is written.
 *  - The stronger alternative is a **content fingerprint** per construct (the `{{key}}` substitution
 *    regex, the `env.variables.find` lookup). It cannot be adopted yet: `savedEntitySend.ts` legitimately
 *    contains the token fingerprint until WI 3 shrinks the gate to `!isDraft && !hasLocalChanges`, so a
 *    fingerprint list would open with a false positive that has to be allowlisted itself. Add the
 *    fingerprint in the WI that deletes the last renderer copy of each construct, when it has exactly
 *    one honest home.
 *  - The structurally complete answer is `Engine_Module_Plan.md` §5.2's ESLint rule, which this repo
 *    cannot have — it has no ESLint.
 */
import fs from "node:fs";
import path from "node:path";
import { describe, it, expect } from "vitest";

const ROOT = process.cwd();
const GUI_SRC = path.join(ROOT, "packages", "gui", "src");
const ENGINE_SRC = path.join(ROOT, "packages", "engine", "src");

/**
 * The domain constructs of `plan/19` §1, each with how to spot a **definition** of it and where its
 * authority lives.
 *
 * `authority` is either `<src-relative path> — <symbol[, symbol]>`, which the last test verifies, or
 * `planned: WI N — <what will land>`, which records the destination so a reader does not have to
 * re-derive it from the plan. A `planned:` entry becoming real is itself the signal that a WI landed.
 */
interface Construct {
  what: string;
  detect: RegExp;
  authority: string;
}

const CONSTRUCTS: Record<string, Construct> = {
  /**
   * Environment-variable and header resolution. §1 row 1: the engine's copy was `mockHandler.ts`'s
   * private `resolveVars`; WI 2's engine half moved it **down** into `utils/resolve.ts` so the mocks and
   * the new pipeline share one.
   *
   * **The global+active merge is not there yet, and that is a refinement of §8 trap 2 rather than an
   * omission.** The merge decides *which* environment to resolve against, so it moves with the **call**
   * — WI 3, when the engine becomes the resolving party — not with the function. Landing it beside the
   * resolver now would add a second merge implementation while `App.tsx:288`'s stayed authoritative,
   * which is trap 1's duplication introduced rather than removed.
   */
  resolver: {
    what: "environment/header resolution",
    detect: /export\s+function\s+(resolveVars|resolveHeaders|mergeEnvVars)\s*\(/,
    authority: "packages/engine/src/utils/resolve.ts — resolveVars, resolveHeaders",
  },

  /**
   * The `{{random.*}}` **generator**. §1 row 2: the two implementations' data pools are identical and
   * *nothing pins them equal* — they agree by discipline. WI 2 deletes the renderer's generator.
   *
   * **The token catalogue is deliberately NOT part of this construct.** The owner's call, 2026-09-29
   * (`plan/19` §3): the UI may keep a **copy of the list** for autocomplete while the *resolver* stays
   * engine-side — so the split is list-vs-resolver, not renderer-vs-engine, and flagging
   * `RANDOMIZER_TOKENS` here would contradict that decision. The copy is pinned instead by
   * `tests/gui/randomizerCatalogue.test.ts`, which asserts the UI list equals the engine's **and** that
   * every entry has a matching `case` in the generator. A copy with no pin is the very defect §1 row 2
   * names, so the test is what makes this copy legitimate rather than a fresh instance of the problem.
   */
  randomizer: {
    what: "{{random.*}} generator",
    detect: /export\s+function\s+(generateRandom|resolveRandomizers)\s*\(/,
    authority: "packages/engine/src/utils/randomizer.ts — generateRandom, resolveRandomizers",
  },

  /**
   * The request pipeline — `resolve → pre-script → send → post-script → test` — detected at its
   * **orchestration** sites. §1 rows 3 and 4 are the same five steps written twice; D24 makes them one
   * engine function that both input forms call.
   *
   * The lookbehind excludes a *declaration* (`export async function runPreScript(`), which is
   * `pipelineStep` below, so the two constructs never double-count the same line.
   */
  pipeline: {
    what: "pre/post/test script orchestration",
    detect: /(?<!function\s)\b(runPreScript|runPostScript|runTestScript)\s*\(/,
    authority: "planned: WI 3 — one engine function implementing D24, called by both input forms",
  },

  /**
   * The renderer's own IPC wrappers for the sandbox steps. Not in §1's table, and they are a construct
   * anyway: each carries an `envToMap` that duplicates engine knowledge, and each exists only to be
   * called by `pipeline` above. WI 3 deletes the orchestration, which leaves these unreferenced.
   *
   * **`script.execute` is not this.** It is the engine's sandbox entry point and stays — see §8 trap 7.
   */
  pipelineStep: {
    what: "renderer-side pre/post/test step wrappers",
    detect: /export\s+async\s+function\s+(runPreScript|runPostScript|runTestScript)\s*\(/,
    authority: "planned: WI 3 — deleted with the orchestration; the sandbox itself stays engine-side",
  },

  /** §1's footnote: a pure parse of a wire format the engine already parses elsewhere. WI 5. */
  curlParser: {
    what: "cURL command parsing",
    detect: /export\s+function\s+parseCurl\s*\(|export\s+const\s+SKIP_CURL_HEADERS\b/,
    authority: "planned: WI 5 — request.parseCurl",
  },

  /** §1 row 5: the engine already renders the report (`runner.exportReport` uses it); this is the copy. */
  reportRenderer: {
    what: "collection-run HTML report rendering",
    detect: /export\s+function\s+generateHtmlReport\s*\(/,
    authority: "packages/engine/src/runner/reportHtml.ts — renderRunnerHtml",
  },

  /** §1's last row and §5.1: entity → on-disk path computation, which the storage layer owns. */
  pathHelpers: {
    what: "entity → on-disk path computation",
    detect: /export\s+function\s+(sanitizeDirName|entityRelPath|flatEntityRelPath)\s*\(/,
    authority:
      "packages/engine/src/workspace/workspaceFs.ts — sanitizeDirName, entityRelPath, flatEntityRelPath",
  },
};

/**
 * Constructs that break the boundary today. **Keyed by `gui/src`-relative `file#construct`**, so each
 * entry names one exact definition rather than a whole file — a file-level key would silently cover the
 * next violation added to that file, and two of these files legitimately keep other things
 * (`utils.ts` keeps the presentation helpers; `useRestActions.ts` keeps `handleTest`).
 *
 * **Four entries. The goal is `{}`.** The value must name the work item that deletes the entry, which
 * `every allowlist entry names the work item that deletes it` asserts — an entry with no owner is a violation
 * nobody has agreed to fix. It was nine on 2026-09-29 and is four now; **all five retirements are recorded in
 * the map below, with what forced each**, because a shrink nobody can account for is indistinguishable from a
 * construct that was quietly dropped out of the scan.
 */
const ALLOWED: Record<string, string> = {
  // **Still present, and the reason changed on 2026-09-30.** All four protocol tabs stopped resolving in the
  // renderer, so this entry is no longer "waiting for the last tab" — it is waiting for **WI 6**, because
  // `useWebSocket.ts` resolves a url and headers and `HealthBarPanel.tsx` resolves a service url. `App.tsx`'s
  // `mergeEnvVars` also still runs, for the panes' active-env hint. See `plan/19` §6's WI 3 completion note.
  "lib/resolveVars.ts#resolver":
    "WI 6 — deleted with the last renderer resolver (`useWebSocket` + `HealthBarPanel`), not with the tabs.",
  "lib/randomizer.ts#randomizer":
    "WI 6 — the generator is only reachable through `resolveVars`, so it goes with it; the token catalogue " +
    "stays as a pinned UI copy (`plan/19` §3, D28).",
  // **Five entries have left this allowlist on 2026-09-30, in two groups, and neither group was tidying up.**
  // The staleness assertion forced all five — each construct is gone from the renderer, so an entry that
  // stayed would have failed it.
  //
  //   `components/rest/useRestActions.ts#pipeline`   WI 3's REST slice — the first entry ever retired
  //   `lib/collectionRunner.ts#pipeline`             WI 4 — the run loop, now `runner/runCollection.ts`
  //   `lib/scriptRunner.ts#pipelineStep`             WI 4 — its last caller was that loop
  //   `lib/testRunner.ts#pipelineStep`               WI 4 — likewise
  //   `lib/runnerReport.ts#reportRenderer`           WI 4 — the engine's `runner/reportHtml.ts` is the one
  //
  // The four WI 4 entries went together because they were one feature: the runner's loop was the last caller
  // of the two step wrappers, and its report was the last reason the renderer had a second HTML renderer at
  // all. `script.execute` itself stays — trap 7 — and so do the two storage wrappers that shared
  // `runnerReport.ts` with the renderer.
  "lib/curlParser.ts#curlParser": "WI 5 — deleted; request.parseCurl.",
  "lib/utils.ts#pathHelpers":
    "WI 5 — deleted; the engine already computes these (workspaceFs).",
};

// ── scanning ──────────────────────────────────────────────────────────────────────────────────────

function walk(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) return walk(p);
    return /\.(ts|tsx)$/.test(p) ? [p] : [];
  });
}

/** `packages/gui/src/lib/utils.ts` → `lib/utils.ts`. */
function relToGui(abs: string): string {
  return path.relative(GUI_SRC, abs).split(path.sep).join("/");
}

type Hit = { file: string; construct: string };

/** Every definition of a construct, anywhere under the renderer. */
function scan(): Hit[] {
  const hits: Hit[] = [];
  for (const abs of walk(GUI_SRC)) {
    const src = fs.readFileSync(abs, "utf8");
    for (const [name, c] of Object.entries(CONSTRUCTS)) {
      if (c.detect.test(src)) hits.push({ file: relToGui(abs), construct: name });
    }
  }
  return hits;
}

const HITS = scan();
const key = (h: Hit) => `${h.file}#${h.construct}`;

// ── the ratchet ───────────────────────────────────────────────────────────────────────────────────

describe("the renderer is a client, not an engine (plan/19 D22)", () => {
  it("the scanner is looking at a real, non-empty renderer tree", () => {
    // Without this, a renamed or moved `packages/gui/src` makes every assertion below pass by
    // measuring nothing — the same vacuous-green failure `layout.test.ts` guards with its
    // "every module named in the contract is a real directory" case.
    expect(
      fs.existsSync(GUI_SRC),
      `${GUI_SRC} does not exist. The scan would find zero files and pass vacuously; a moved ` +
        `renderer must move this constant with it.`,
    ).toBe(true);
    expect(
      walk(GUI_SRC).length,
      "the renderer tree scanned to zero files — the ratchet is measuring nothing.",
    ).toBeGreaterThan(0);
  });

  it("no engine-owned construct is defined in the renderer, allowlist aside", () => {
    const unexpected = HITS.filter((h) => !(key(h) in ALLOWED)).map(
      (h) => `packages/gui/src/${h.file}   [defines ${h.construct}: ${CONSTRUCTS[h.construct].what}]`,
    );
    expect(
      unexpected,
      `${unexpected.length} domain construct(s) are defined in the renderer and are not in ALLOWED. ` +
        `The renderer holds no domain logic (plan/19 D22) — implement it in the engine and call it, ` +
        `or, if the copy is pre-existing debt, add it to ALLOWED naming the work item that deletes it.`,
    ).toEqual([]);
  });

  it("the allowlist has no stale entries — it can only shrink", () => {
    const live = new Set(HITS.map(key));
    const stale = Object.keys(ALLOWED).filter((k) => !live.has(k));
    expect(
      stale,
      `${stale.length} allowlist entr(ies) no longer describe a violation — the construct was moved ` +
        `out of the renderer or the file was renamed. Delete the entr(ies); the allowlist must not ` +
        `accumulate permission for code that is already engine-side.`,
    ).toEqual([]);
  });

  it("every allowlist entry names the work item that deletes it", () => {
    const unowned = Object.entries(ALLOWED)
      .filter(([, reason]) => !/^WI \d+\b/.test(reason))
      .map(([k, reason]) => `${k} — "${reason}"`);
    expect(
      unowned,
      `${unowned.length} allowlist entr(ies) do not name a work item. Debt with no owner is the ` +
        `failure mode plan/19 §11 warns about: the entry outlives the phase and becomes permanent.`,
    ).toEqual([]);
  });

  it("each landed authority is where the table says it is", () => {
    // The teeth in the other direction: a violation cannot be "fixed" by deleting the engine's copy as
    // well, which would leave the renderer as the only implementation. `planned:` entries are the three
    // WIs that have not landed yet and are asserted once their `authority` string stops saying so.
    const broken: string[] = [];
    for (const [name, c] of Object.entries(CONSTRUCTS)) {
      if (c.authority.startsWith("planned:")) continue;
      const [rel, symbols] = c.authority.split(" — ");
      const abs = path.join(ROOT, rel);
      // An authority that is not in the engine is not an authority. This is what stops a violation
      // being "fixed" by repointing the table at another renderer file.
      if (!abs.startsWith(ENGINE_SRC + path.sep)) {
        broken.push(`${name}: authority ${rel} is not inside packages/engine/src`);
        continue;
      }
      if (!fs.existsSync(abs)) {
        broken.push(`${name}: ${rel} does not exist`);
        continue;
      }
      const src = fs.readFileSync(abs, "utf8");
      for (const symbol of (symbols ?? "").split(",").map((s) => s.trim()).filter(Boolean)) {
        if (!new RegExp(`export\\s+(?:async\\s+)?(?:function|const)\\s+${symbol}\\b`).test(src)) {
          broken.push(`${name}: ${rel} does not export ${symbol}`);
        }
      }
    }
    expect(
      broken,
      `${broken.length} construct authority/authorities are missing or renamed. plan/19 moves these ` +
        `constructs INTO the engine — if the engine copy is gone too, the renderer's is the only one ` +
        `left and the boundary has been broken rather than repaired.`,
    ).toEqual([]);
  });
});
