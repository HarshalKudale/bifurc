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
 * ## The allowlist is debt, and it can only shrink — **and it is now empty**
 *
 * **Zero entries. `plan/19` §9's acceptance criterion is met**: *"`tests/gui/boundary.test.ts` passes with an
 * **empty** allowlist; the renderer contains no resolver, randomizer, pipeline, curl parser or report
 * renderer."* It opened at 9 on 2026-09-29, which was itself a correction of `plan/19` §6 — that section
 * predicts `−2 / −1 / −1 / −2` across WI 2–5, summing to 6, because it counts *files deleted* while this file
 * counts *constructs present*.
 *
 * **The count moved five times, always downward, and every retirement was forced rather than chosen:**
 *
 * | from → to | what |
 * |---|---|
 * | 9 → 8 | WI 3 (REST) — `useRestActions.ts#pipeline` stopped orchestrating |
 * | 8 → 4 | WI 4 — the run loop, the two step wrappers it was the last caller of, and the second HTML renderer |
 * | 4 → 3 | WI 5 — `lib/curlParser.ts` |
 * | 3 → 2 | WI 5 item 3 — the `lib/utils.ts` path helpers |
 * | 2 → 0 | **WI 6 — `lib/resolveVars.ts` and `lib/randomizer.ts`'s generator** |
 *
 * **The staleness assertion forced every one of the nine.** With a construct gone from the renderer its entry
 * could not be kept — which is what makes the shrink checkable rather than remembered, and it is why an entry
 * whose owner never lands is visible as a *permanent* entry rather than as an unclosed checkbox.
 *
 * **⚠ The last two were WI 6's and not WI 3's, and §6's table had them in the wrong row.** The resolver and
 * the randomizer outlived the four tabs because resolution is not only the tabs' business: `useWebSocket.ts`
 * resolved a url and its headers, and `HealthBarPanel.tsx` resolved a service url. WI 5's item 4 moved the
 * second and left the first, and the two entries are keyed on the resolver's **definition** — so they could
 * only retire when the last *user* stopped, which is WI 6.
 *
 * **⚠ `lib/randomizer.ts` survives with its catalogue, and that is not a missed deletion.** The construct is
 * the *generator* (`generateRandom` / `resolveRandomizers`), both of which are gone; `RANDOMIZER_TOKENS` stays
 * for `RandomizerHint`'s autocomplete under D28's list-vs-resolver split, pinned by
 * `tests/gui/randomizerCatalogue.test.ts`. This is the same outcome `lib/runnerReport.ts` had in WI 4 — the
 * `file#construct` keying doing its job: the file stayed, the construct the ratchet tracked did not.
 *
 * ## What this does not catch, and why it is still worth having
 *
 * Detection is by **name**, so a copy re-implemented under a different name is not caught. That is a
 * real gap, recorded rather than papered over:
 *
 *  - The mistake this guards against is not an adversary — it is a contributor adding a duplicate
 *    because the engine's copy was not where they looked. A name-based scan catches that, at
 *    the moment the file is written.
 *  - The stronger alternative is a **content fingerprint** per construct (the `{{key}}` substitution
 *    regex, the `env.variables.find` lookup). Its precondition is now met — the renderer holds **zero**
 *    copies of every construct, so a fingerprint list would open with no false positive to allowlist —
 *    and it is still deliberately **not added here**: it is a second scan category with its own
 *    unmeasured false-positive surface (a fingerprint for `resolver` is `/\{\{(\w+)\}\}/`, which the
 *    editor's own hint components legitimately touch), and the phase's goal is reached without it.
 *    **Recorded as the next strengthening rather than smuggled in with the retirement that enables it.**
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
   * **The global+active merge is in that file too, and it got there the way §8 trap 2 requires — with the
   * call, not with the function.** The merge decides *which* environment to resolve against, so it landed in
   * WI 3 when the engine became the resolving party (`envState.ts` reads it); landing it beside the resolver
   * earlier would have been a *second* merge implementation while `App.tsx`'s stayed authoritative, which is
   * trap 1's duplication introduced rather than removed.
   *
   * **The renderer's last two users were `useWebSocket.ts` and `HealthBarPanel.tsx`, and WI 6 is what retired
   * the entry.** WI 5 item 4 moved the panel's resolution to `env.resolveText`; WI 6 moved the socket's to the
   * engine, which also replaced `App.tsx`'s `mergeEnvVars` with `env.getMerged`. Only then could
   * `lib/resolveVars.ts` be *deleted* — and the entry is keyed on the definition, so it could not have gone a
   * moment earlier.
   */
  resolver: {
    what: "environment/header resolution",
    detect: /export\s+function\s+(resolveVars|resolveHeaders|mergeEnvVars)\s*\(/,
    authority: "packages/engine/src/utils/resolve.ts — resolveVars, resolveHeaders, mergeEnvVars",
  },

  /**
   * The `{{random.*}}` **generator**. §1 row 2: the two implementations' data pools are identical and
   * *nothing pins them equal* — they agree by discipline. WI 2 moved the authority into the engine and WI 6
   * deleted the renderer's generator, which was reachable only through `lib/resolveVars.ts`.
   *
   * **The token catalogue is deliberately NOT part of this construct.** The owner's call, 2026-09-29
   * (`plan/19` §3): the UI may keep a **copy of the list** for autocomplete while the *resolver* stays
   * engine-side — so the split is list-vs-resolver, not renderer-vs-engine, and flagging
   * `RANDOMIZER_TOKENS` here would contradict that decision. The copy is pinned instead by
   * `tests/gui/randomizerCatalogue.test.ts`, which asserts the UI list equals the engine's **and** that
   * every entry has a matching `case` in the generator. A copy with no pin is the very defect §1 row 2
   * names, so the test is what makes this copy legitimate rather than a fresh instance of the problem.
   *
   * **Which is why `lib/randomizer.ts` still exists.** The construct is gone from it; the file is not. That is
   * the `file#construct` keying working as designed, and the same shape `lib/runnerReport.ts` had in WI 4.
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
    authority: "packages/engine/src/pipeline/runPipeline.ts — runRequestPipeline",
  },

  /**
   * The renderer's own IPC wrappers for the sandbox steps. Not in §1's table, and they are a construct
   * anyway: each carries an `envToMap` that duplicates engine knowledge, and each existed only to be
   * called by `pipeline` above. WI 3 deleted the orchestration and WI 4 deleted the run loop that was their
   * last caller, so the wrappers are gone.
   *
   * **`script.execute` is not this.** It is the engine's sandbox entry point and stays — see §8 trap 7. The
   * authority below is that sandbox, which is what the wrappers were thin adapters over.
   */
  pipelineStep: {
    what: "renderer-side pre/post/test step wrappers",
    detect: /export\s+async\s+function\s+(runPreScript|runPostScript|runTestScript)\s*\(/,
    authority: "packages/engine/src/mocks/scriptExecutor.ts — executeIpcScript",
  },

  /**
   * §1's footnote: a pure parse of a wire format the engine already parses elsewhere. WI 5.
   *
   * **The construct is the *parser* only — `SKIP_CURL_HEADERS` is deliberately not flagged.** The
   * header list is *data*, and the renderer needs a copy of it for a read-side job the engine cannot do
   * for it (filtering a **stored** mock's `capturedHeaders` on load — see `restTabHelpers.ts`). That is
   * the same **list vs logic** split D28 made for `RANDOMIZER_TOKENS`, and the same consequence
   * follows: the copy is pinned by a test (`tests/gui/curlHeaderSkip.test.ts`) rather than by this scan.
   * Flagging the list here would contradict the decision, exactly as flagging the randomizer catalogue
   * would contradict D28 — so the `detect` pattern matches the parser and nothing else.
   */
  curlParser: {
    what: "cURL command parsing",
    detect: /export\s+function\s+parseCurl\s*\(/,
    authority: "packages/engine/src/utils/curl.ts — parseCurl",
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
 * Constructs that break the boundary today. **Keyed by `gui/src`-relative `file#construct`**, so each entry
 * names one exact definition rather than a whole file — a file-level key would silently cover the
 * next violation added to that file, and two of these files legitimately keep other things
 * (`utils.ts` keeps the presentation helpers; `useRestActions.ts` keeps `handleTest`).
 *
 * **`{}` — empty, and that is `plan/19` §9's acceptance criterion rather than a milestone on the way to one.**
 * The renderer defines no resolver, no randomizer, no pipeline, no curl parser and no report renderer.
 *
 * The nine retirements, in the order they happened, each with what forced it — because a shrink nobody can
 * account for is indistinguishable from a construct that was quietly dropped out of the scan:
 *
 * ```
 *   components/rest/useRestActions.ts#pipeline     WI 3 (REST) — the first entry ever retired
 *   lib/collectionRunner.ts#pipeline               WI 4 — the run loop, now runner/runCollection.ts
 *   lib/scriptRunner.ts#pipelineStep               WI 4 — its last caller was that loop
 *   lib/testRunner.ts#pipelineStep                 WI 4 — likewise
 *   lib/runnerReport.ts#reportRenderer             WI 4 — the engine's runner/reportHtml.ts is the one
 *   lib/curlParser.ts#curlParser                   WI 5 — engine/utils/curl.ts, file deleted
 *   lib/utils.ts#pathHelpers                       WI 5 item 3 — workspace/entityPaths.ts computes them
 *   lib/resolveVars.ts#resolver                    WI 6 — the last user was useWebSocket.ts; file deleted
 *   lib/randomizer.ts#randomizer                   WI 6 — reachable only through resolveVars; generator gone
 * ```
 *
 * **The last two are the pair §6's table put in WI 3's row**, and the correction matters: they could not
 * retire with the tabs, because `useWebSocket.ts` still resolved a url and its headers. See this file's
 * docblock and `plan/19` §6's WI 3 completion note.
 *
 * **⚠ A construct's `detect` may be narrowed, and WI 5 narrowed one.** `curlParser` used to match the header
 * *list* as well as the parser. It matches the parser only now, because the renderer legitimately keeps a
 * pinned copy of the list — the D28 list-vs-logic split. Narrowing a pattern is the one edit to this file that
 * *loosens* the scan, so it must be paired with a pin (`tests/gui/curlHeaderSkip.test.ts` here), or it is
 * indistinguishable from deleting a construct to silence a violation.
 *
 * **`every allowlist entry names the work item that deletes it` now passes vacuously** — there is nothing to
 * own. It is kept rather than deleted for the reason the whole file is kept: the next entry added to it is
 * exactly the case it exists for, and a rule that has to be re-added under pressure is a rule that gets
 * skipped.
 */
const ALLOWED: Record<string, string> = {};

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
    // well, which would leave the renderer as the only implementation. **Every entry is now a real authority
    // — there are no `planned:` rows left** — so this assertion covers all eight constructs, which is the
    // first time that has been true. It is also what stops the retirement of the last two entries from being
    // "fixed" by repointing the table at nothing.
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
