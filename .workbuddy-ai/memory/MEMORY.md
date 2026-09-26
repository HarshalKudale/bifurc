# Bifurc — Project Memory

**Read `TRAPS.md` first** (**111** failure modes — `grep -c '^## '`). Authorities: state → `plan/README.md`
§Status · live phase → `plan/08` · decisions → `plan/00-decisions.md` · suites → `TESTING.md` · ledgers →
`plan/07` · **port model → `plan/17`** (supersedes the per-workspace half of `plan/15`) · procedures → the
`bifurc-*` skills. **A pointer, not a record** — long-form reasoning lives in `plan/`.

> **Electron/GUI commands are the user's to run** (`npm run dev`, `test:e2e`) — an agent-shell failure is
> evidence about the shell, not the product.

## Layout

- **Flat repo — the app IS the root.** Git root is the PARENT dir; branch `standalone-engine`. **`git rm`,
  `git stash` and `git checkout -- <file>` are ALL destructive** — `cp` to `/tmp` before any rollback.
- **App runs `packages/*/dist/`; tests run `src/`** → `npm run build:packages` after a package edit (only
  `@bifurc/engine/*` aliases to `src`; the rest resolve `dist/`, so an un-rebuilt edit is **silently untested**).
  Every package build needs `CODEBUDDY_SAFE_DELETE_ENABLED=0`. **Directory renames are blocked**; a *file*
  `git mv` works. **One `Edit` per file per message** — two same-file edits revert each other.
- **⚠ `build:packages` omits `@bifurc/gui`** — the GUI has no `dist`; `build:renderer` / `build:web` bundle its
  `src` **as source**, so `build:packages` passing says nothing about a GUI edit. **Vitest resolves `@/…` through
  its own alias plugin, not the bundlers'** — a green test is not proof the bundle resolves. Build it.
- **No tsconfig covers `tests/`** — `npm run typecheck` is green while test files are broken. Run the suite.
- **⚠ A `Dockerfile` copying one manifest per workspace is coupled to `package-lock.json`** — `npm ci` validates
  that set and fails **both** ways: a manifest the lockfile lacks (`EUSAGE: Missing: … from lock file`) and a
  lockfile entry whose directory is gone (npm follows the link and asks the **public registry**, 404). Check with
  `npm ci --dry-run` in a temp context, never by reasoning. Reconciling a stale one cost **0 version changes**.

## Measuring the suite

- **A *delta* is the signal; name its mechanism.** Baseline **re-measured, never carried forward**. **There is no
  standing failure — every failure is a flake to identify.** Judge a mutation by **exit code, not failure count**.
- **⚠ Freeze the tree while a run is in flight** — a file renamed mid-run reports `0/0` **while the TOTAL does not
  move**. **⚠ A single-project run is NOT the full run**: reconcile **unit + integration = the full total**.
- **⚠ A fixture's `cleanup()` is not local** — a second fixture un-points the store for every case after it.
  **⚠ A new suite that binds ports degrades every "the scan lands on `base + N`" assertion** — remedy: the shared
  `holdBaseWithFreeSuccessors()`.

**Current: 2,739 tests / 120 files — 2,699 passed, 0 failures of any kind, 40 skipped (2026-09-26).** Unit
**2,264 / 89**, integration **475 / 31** (both measured). e2e: 11 specs, 46/46 owner-run.

## Phase state

**Live: P15, P16, P17.** `plan/17` WI 1/2/3/5/6/7/8/9 landed 2026-09-25/26; **only item 4's Electron half is
open** — and item 5 does **not** depend on it, because the entrypoint pins the companion port so no scan runs.
P7 live: `packages/gui` exports `<App />`; hosts `renderer/` (Electron) and `web/`. **P0–P6 done; P14 CLOSED.**

### Invariants worth not re-deriving

- **Every port is ENGINE/host-level — read `plan/17` §1, not a summary of it.** Edge `80` **bound, not scanned**;
  forward proxy `settings.forwardProxyPort` **bound, never scanned, fails loudly naming the port**; webhook
  random, **off by default**; companion `4747` **restart-only**; GUI = companion `+ 1`, **derived**. **WI 7: a
  `config.save` re-binds the edge, proxy and webhook** — `start(newPort)` *is* the rebind; `bindEdgeInstance()`
  has **no lazy default** (unbound → **503**). **Three compositions bind the edge** — `createEngine()`,
  `runtime/host.ts`, and **`src/main.ts`, which never calls `createEngine()`**; grep the **factory**, not the
  entry point.
- **WI 5: the image's `ENTRYPOINT` is `scripts/docker-entrypoint.mjs`**, and the GUI port is derived **there**,
  once. `CMD` is `--ws-host 127.0.0.1` with **no `--ws-port`** — a flag would **shadow `BIFURC_COMPANION_PORT`**.
  The healthcheck reads the port the entrypoint **bound** from `/tmp/bifurc/companion-port`, so probe and
  listener agree by construction. `npm run verify:docker-entrypoint` is the owner-run end-to-end (it binds
  80/8080/4747/4748). **The `docker build` itself is still unrun** — Docker Desktop was not up.
- **WI 6: one global Ports section** (`panels/settings/PortsSettingsSection.tsx` ← `SettingsPanel`). Its companion
  badge is **static on purpose** — `companionPort` is on **no protocol command** (`serverStatus` is the edge's),
  so "changed vs running" is unverifiable in the renderer. **No web-GUI row**: derived, and **Electron has no GUI
  port**.
- **Since 2026-09-24 the engine is NOT cloud-deployable: `--domain` is REMOVED**, and is a **fatal unrecognised
  flag**. Three survivors are deliberate: the bind opt-ins, `hello`'s `hostDomain`, `EdgeRouterOptions.hostDomain`.
- **WI 8: mocks are REST-only.** The GraphQL/SOAP/gRPC mock kinds, their folder kinds and the three
  `grpc.*MockServer` commands are **gone**; the four *request* kinds and their schemas all stay. **The client
  surface is 132 keys, not 144** (`surface.ts` is the spec; `BifurcApi` declares 128 — the gap stays 4).
- **`packages/gui` is NOT in `npm run typecheck`.** Sole runner: `npm run typecheck --workspace @bifurc/gui`
  (**34 pre-existing errors**). **The instrument is not the count but *which files*.** **`tsc` after any
  type-level change — vitest does not typecheck.**
- **The browser's transport is auth-wrapped; Electron's is not.** Reconnect **reassigns `window.api`**; a token
  rotates per host start, so a rotated-token reconnect is *terminal*.
- **A headless browser DOES run from the agent shell.** `verify:web` / `verify:web:reconnect` found four defects
  in code that passed 471 tests; **neither is self-contained**. **`build:main` after any `src/preload.ts` change.**
- **The companion is RETIRED**; the extension becomes an ordinary authenticated client — **engine half DONE,
  extension half deliberately NOT**. **`onCompanionRefresh` was KEPT** — misleadingly named but *live*.
