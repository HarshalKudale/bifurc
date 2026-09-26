# Bifurc — Project Memory

**Read `TRAPS.md` first** (**111** failure modes — `grep -c '^## '`). Authorities: state → `plan/README.md`
§Status · decisions → `plan/00-decisions.md` · suites → `TESTING.md` · **port model → `plan/17`** ·
edge/mapping → `plan/15` · procedures → the `bifurc-*` skills. **A pointer, not a record.**

> **⚠ ONE BUILD TARGET SINCE 2026-09-26: the Electron desktop app.** The browser host (`web/`) and the
> container image are **deleted** — P7 and P9 are decommissioned, P16 was never built. `plan/README.md`'s
> 2026-09-26 block is the authority; the phase rows below it are the record.
>
> **Electron/GUI commands are the user's to run** (`npm run dev`, `test:e2e`) — an agent-shell failure is
> evidence about the shell, not the product.

## Layout

- **Flat repo — the app IS the root.** The repo root is the **project dir itself** (`git rev-parse
  --show-toplevel` → `I:/workspace/Bifurc-Worskspace/Bifurc`) — **corrected 2026-09-26**. The *parent* dir
  holds a separate, **empty** `.git` (no commits, no remote, 0 tracked files), so it is not a publish path.
  Branch `standalone-engine`. **`git rm`,
  `git stash` and `git checkout -- <file>` are ALL destructive** — `cp` to `/tmp` before any rollback.
  **⚠ In this sandbox `rm` is a shim function and `git rm` under `scripts/` wipes the whole directory** — use
  `python -c "import os; os.remove(...)"`, in small batches, verifying each. See `~/.workbuddy-ai/MEMORY.md`.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after a package edit (only
  `@bifurc/engine/*` aliases to `src`; the rest resolve `dist/`, so an un-rebuilt edit is **silently untested**).
  Every package build needs `CODEBUDDY_SAFE_DELETE_ENABLED=0`. **Directory renames are blocked**; a *file*
  `git mv` works. **One `Edit` per file per message** — two same-file edits revert each other.
- **⚠ `build:packages` omits `@bifurc/gui`** — the GUI has no `dist`; `build:renderer` bundles its `src` **as
  source**, so `build:packages` passing says nothing about a GUI edit. **Vitest resolves `@/…` through its own
  alias plugin, not the bundler's** — a green test is not proof the bundle resolves. Build it.
- **No tsconfig covers `tests/`** — `npm run typecheck` is green while test files are broken. Run the suite.
  **Reproduced 2026-09-26:** a deleted string left its assertion behind, `tsc` exited 0, the suite found it.
- **⚠ `package-lock.json`'s workspace set is a coupling** — `npm ci` validates it and fails **both** ways: a
  manifest the lockfile lacks (`EUSAGE: Missing: … from lock file`) and a lockfile entry whose directory is gone
  (npm follows the link and asks the **public registry**, 404). Check with `npm ci --dry-run` in a temp context,
  never by reasoning. It broke the Dockerfile first; it breaks a fresh clone and CI just the same.

## Measuring the suite

- **A *delta* is the signal; name its mechanism.** Baseline **re-measured, never carried forward**. **There is no
  standing failure — every failure is a flake to identify.** Judge a mutation by **exit code, not failure count**.
- **⚠ Freeze the tree while a run is in flight** — a file renamed mid-run reports `0/0` **while the TOTAL does not
  move**. **⚠ A single-project run is NOT the full run**: reconcile **unit + integration = the full total**.
- **⚠ A fixture's `cleanup()` is not local** — a second fixture un-points the store for every case after it.
  **⚠ A new suite that binds ports degrades every "the scan lands on `base + N`" assertion** — remedy: the shared
  `holdBaseWithFreeSuccessors()`.

**Current: 2,638 tests / 115 files — 2,598 passed, 0 failures of any kind, 40 skipped (2026-09-26).** Unit
**2,178 / 85**, integration **460 / 30** (both measured). e2e: 11 specs, 46/46 owner-run.

## Phase state

**Live: P18** — enterprise SSO / org workspaces / permission gating, opened 2026-09-26; **WI 0 done** (the
plan doc and D19–D21), nothing else started. **P17** has only item 4's Electron half open. **P0–P6 and P14
done. P7, P9 and P16 DECOMMISSIONED 2026-09-26.** `packages/gui` still exports `<App />`, and `renderer/` is
now its only consumer.

**⚠ P18 adds a SECOND authorization axis — do not merge it with the first.** The engine already gates per
**session** (`transport/auth/scopes.ts`: a total command→scope table asserted at import, vocabulary
`read|write|execute|admin` with no hierarchy, fail-closed, enforced by the `authenticated` decorator — it
answers *"who is calling?"*). P18 gates per **workspace** (`PermissionContext` on the workspace record,
answering *"which workspace is being changed?"*). Conflating them makes "the org revoked this right"
indistinguishable from "this session may not call this command". **Local actor ⇒ allow-all** (that is what
keeps every existing suite byte-identical); **org actor ⇒ fail closed**, with its **own** error code rather
than `FORBIDDEN`. The enterprise package itself is **private** — a sibling repo, `../bifurc-enterprise`
(D20); only the permission plumbing is public.

### Invariants worth not re-deriving

- **Every port is ENGINE/host-level — read `plan/17` §1, not a summary.** Edge `80` **bound, not scanned**;
  forward proxy `settings.forwardProxyPort` **bound, never scanned, fails loudly naming the port**; webhook
  random, **off by default**; companion `4747` **restart-only**, now found as a **single port** — the
  GUI-neighbour *pair* went with the browser host, so a taken neighbour no longer refuses a start. **WI 7: a
  `config.save` re-binds the edge, proxy and webhook** — `start(newPort)` *is* the rebind; `bindEdgeInstance()`
  has **no lazy default** (unbound → **503**). **Three compositions bind the edge** — `createEngine()`,
  `runtime/host.ts`, and **`src/main.ts`, which never calls `createEngine()`**; grep the **factory**, not the
  entry point.
- **The edge takes NO `hostDomain`** (removed 2026-09-26 — its only reader was the GUI mapping's row G), and its
  routing table is rows **0 / 2 / 5**: no workspace → **503**, a mapped domain → reverse proxy, otherwise the
  **home page**. **`hello`'s `hostDomain` is the wire field and stays.**
- **Since 2026-09-24 the engine is NOT cloud-deployable: `--domain` is REMOVED**, and is a fatal unrecognised
  flag. Survivors: the bind opt-ins and `hello`'s `hostDomain`.
- **Mocks are REST-only**; the four *request* kinds and their schemas all stay. **The client surface is 132 keys**
  (`surface.ts` is the spec; `BifurcApi` declares 128 — the gap stays 4).
- **`packages/gui` is NOT in `npm run typecheck`.** Sole runner: `npm run typecheck --workspace @bifurc/gui`
  (**34 pre-existing errors**), now the CI step that replaced `typecheck:web`. **The instrument is not the count
  but *which files*.** **`tsc` after any type-level change — vitest does not typecheck.**
- **The browser's transport is auth-wrapped; Electron's is not** — `hello` is mandatory before the first real
  request. `packages/web-client` survives for the extension. **`build:main` after any `src/preload.ts` change.**
- **The companion is RETIRED as a bridge**; the extension becomes an ordinary authenticated client — engine half
  DONE, extension half deliberately NOT. **`onCompanionRefresh` was KEPT** — misleadingly named but *live*.
