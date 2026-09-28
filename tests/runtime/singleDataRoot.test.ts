/**
 * P14 §8 criterion 16 — "`store/paths.ts` still has **exactly one** data root in the process
 * (`_dataRootOverride` set once) — assert it, do not assume it."
 *
 * ## Why this is a real risk rather than a formality
 *
 * `plan/14` WI 3's highest-risk decision was **where the host lives**, and the answer was "in
 * `packages/engine`, not a new package" for exactly this reason: `tsup` runs `bundle: false` so that
 * `store/paths.ts` stays **one module instance per process**, and a second package — or a bundling change —
 * would silently give the process a *second* data root. `host.ts`'s own header calls that out: *"a host in
 * its own package would be a second copy of `_dataRootOverride` waiting to happen."*
 *
 * The failure mode is the quiet kind: a `setDataRoot()` call made through one entry point would not be seen
 * through another, and paths would resolve to the wrong root rather than throwing. So the criterion says
 * assert it, and this file does — structurally for the declaration, and behaviourally for the sharing.
 *
 * ## The two roots, and why there are two *variables* but one *root*
 *
 * `store/paths.ts#dataRoot` is **the** root (`setDataRoot`). `store/workspaceFs.ts#_dataRootOverride` is a
 * **test-only override layered on top of it** — `dataRoot()` returns the override when set and falls through
 * to `path.join(dataDir(), "data")` otherwise. Two variables, one root; this file pins the layering so a
 * change that made the override independent would fail here rather than in production.
 */

import { describe, it, expect, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

import { dataDir, setDataRoot, resetDataRootForTests } from "@bifurc/engine/store/paths";
import { dataRoot, setDataRootOverride } from "@bifurc/engine/store/workspaceFs";

const ENGINE_SRC = path.resolve(process.cwd(), "packages/engine/src");

/** Every `.ts` file under the engine's source — the scope "exactly one in the process" has to cover. */
function engineSources(dir: string = ENGINE_SRC, out: string[] = []): string[] {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) engineSources(full, out);
    else if (entry.name.endsWith(".ts")) out.push(full);
  }
  return out;
}

/** Repo-relative, forward-slashed — so the failure message reads the same on either platform. */
function rel(file: string): string {
  return path.relative(process.cwd(), file).replace(/\\/g, "/");
}

describe("one data root per process (P14 §8 criterion 16)", () => {
  afterEach(() => {
    setDataRootOverride(null);
    resetDataRootForTests();
    vi.unstubAllEnvs();
  });

  it("declares each root exactly once in the whole engine", () => {
    const files = engineSources();
    // A sanity check on the instrument itself: a walk that found nothing would make both assertions below
    // vacuously true.
    expect(files.length).toBeGreaterThan(50);

    const sources = files.map((f) => ({ file: f, text: fs.readFileSync(f, "utf-8") }));
    const declaredRoot = sources.filter((s) => /^let dataRoot\b/m.test(s.text)).map((s) => rel(s.file));
    const declaredOverride = sources.filter((s) => /^let _dataRootOverride\b/m.test(s.text)).map((s) => rel(s.file));

    // **Exactly one of each, and named.** `toEqual` on the path list rather than `toHaveLength(1)` so the
    // failure says *which* second module appeared — that is the diagnostic, since a duplicate is what this
    // criterion exists to catch.
    expect(declaredRoot).toEqual(["packages/engine/src/store/paths.ts"]);
    expect(declaredOverride).toEqual(["packages/engine/src/store/workspaceFs.ts"]);
  });

  it("keeps `bundle: false` in the engine's tsup config — the mechanism that makes one declaration one instance", () => {
    const cfg = fs.readFileSync(path.resolve(process.cwd(), "packages/engine/tsup.config.ts"), "utf-8");
    // The declaration count above is a *source* fact. `bundle: false` is what keeps it a *runtime* fact:
    // bundling would inline a private copy of `store/paths.ts` into every entry point, so a `setDataRoot()`
    // through one entry would not be seen through another. Both halves are needed, so both are asserted.
    expect(cfg).toMatch(/bundle:\s*false/);
  });

  it("shares the single root across modules: `workspaceFs.dataRoot()` is derived from `paths.dataDir()`", () => {
    // `dataRoot()`'s win32 branch short-circuits on `%LOCALAPPDATA%` **before** consulting the root (the
    // `--data-dir` caveat `MEMORY.md` records), so the env is cleared to reach the engine-side fallback —
    // which is the branch that proves the sharing.
    vi.stubEnv("LOCALAPPDATA", "");
    setDataRootOverride(null);

    const base = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-root-"));
    try {
      setDataRoot(base);
      expect(dataDir()).toBe(path.resolve(base));
      // The load-bearing line: `workspaceFs` has no root of its own, it resolves *through* `paths`.
      expect(dataRoot()).toBe(path.join(path.resolve(base), "data"));
    } finally {
      fs.rmSync(base, { recursive: true, force: true });
    }
  });

  it("layers the override on the one root rather than being a second root", () => {
    vi.stubEnv("LOCALAPPDATA", "");
    const base = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-root-"));
    const override = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-root-ov-"));
    try {
      setDataRoot(base);
      setDataRootOverride(override);

      // The override wins for the workspace store...
      expect(dataRoot()).toBe(override);
      // ...and the single root is untouched, which is what makes it an override rather than a second root.
      expect(dataDir()).toBe(path.resolve(base));

      // Clearing it falls back to the one root rather than to anything of its own.
      setDataRootOverride(null);
      expect(dataRoot()).toBe(path.join(path.resolve(base), "data"));
    } finally {
      fs.rmSync(base, { recursive: true, force: true });
      fs.rmSync(override, { recursive: true, force: true });
    }
  });
});
