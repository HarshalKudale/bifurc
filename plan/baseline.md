# Baseline

Captured **before** any phase-1+ change. Every later phase compares against these numbers.

Captured **2026-09-15** on `master` @ `476d871`, app version **0.3.3**, Node 22.22.2.

## Test suites

| Metric | Value | Date | Command |
|---|---|---|---|
| Unit test files | **45** | 2026-09-15 | `npm run test:unit` |
| Unit tests passing | **1035 / 1035** | 2026-09-15 | |
| Unit tests failing | **0** | 2026-09-15 | |
| Coverage (statements) | see note below | 2026-09-15 | `npm run test:coverage` |
| Integration tests passing | **323 / 325** | 2026-09-15 | `npm run test:integration` |
| E2E specs passing | *not runnable here* | — | `npm run test:e2e` |
| E2E specs guarded/skipped | 5 known (`protocols`, `environments`, `capture-ws-webhooks`, `settings`, `screenshots`) | | |
| `npm run typecheck` errors | **0** | 2026-09-15 | |
| `npm run typecheck:renderer` errors | **152** (pre-existing) | 2026-09-15 | |

**Full suite: 61 files / 1360 tests. 1358 pass, 2 fail — see "Environment caveats".**

### Coverage note — two numbers, know which you are comparing against

| Scope | Stmts | Branches | Functions | Lines | Files / tests |
|---|---|---|---|---|---|
| **This sandbox**, 2 test files excluded (see below) | 44.35 | 30.45 | 32.38 | 46.47 | 59 / 1304 |
| **Full suite**, last CI-equivalent run (2026-09-14) | 46.14 | 31.30 | 34.34 | 48.39 | 61 / 1360 |

The sandbox figure is **not** comparable to CI — it omits `protocolExecution.integration.test.ts`
and `upstreamFetch.integration.test.ts`, which exercise `src/proxy` heavily. Use the 2026-09-14 row
as the like-for-like reference; use the sandbox row only to compare future sandbox runs against each
other.

### Environment caveats (this sandbox, not the project)

1. **A transparent network interceptor answers dead endpoints.** `127.0.0.1:1` returns a synthetic
   `404` (`content-length: 0`, `connection: close`) instead of refusing the connection. Two
   negative-path tests rely on connection-refused and therefore fail here:
   - `tests/integration/upstreamFetch.integration.test.ts` → "rejects when the upstream is unreachable"
   - `tests/integration/protocolExecution.integration.test.ts` → "returns { ok:false, error } when the WSDL cannot be fetched"

   Both pass on CI/desktop. Verified mechanism: a raw `http.request` to `127.0.0.1:1` from this
   shell receives the synthetic 404.
2. **E2E cannot run** — Electron needs a desktop session (`xvfb-run` on Linux). Type-checked only.
3. **`vitest --coverage` + a failing test = no report.** Vitest's `cleanAfterRun` deletes the report
   on any test failure, and the sandbox's bulk-delete guard then aborts the run entirely. Working
   recipe: exclude the two failing files and use `--coverage.clean=false
   --coverage.reportsDirectory=<fresh dir>`.

## Measurements

| Metric | Value | How measured |
|---|---|---|
| Cold start (launch → usable UI) | *not measured — needs a desktop session* | manual stopwatch, 3 runs, median |
| Engine cold start | n/a before P2 | |
| Idle RSS (Electron main) | *not measured* | Task Manager / Activity Monitor |
| Idle RSS (renderer) | *not measured* | |
| Install size (NSIS) | `release/` last built before flattening — re-measure at P6 | `ls -la release/Bifurc.Setup.exe` |
| Install size (installed) | *not measured* | `du -sh` on the install dir |
| Install size (APPX) | *not measured* | |
| AppImage size | *not measured* | CI artifact |

## Reference counts

Re-run 2026-09-15 (raw output in [`census/handlers.txt`](census/handlers.txt) and
[`census/coupling.txt`](census/coupling.txt); classification in [`census/README.md`](census/README.md)):

| Count | Value | Note |
|---|---:|---|
| `ipcMain.handle` | **112** | matches the plan's expectation exactly |
| `ipcRenderer.invoke` in preload | **136** | |
| `ipcRenderer.on` in preload | **7** | the 7 push events |
| `window.api.*` call sites in renderer | **207** | across **42** files — see note below |
| Files importing `electron` | **22** | `src/**` — see note below |
| `ipcMain.on` | **0** | |
| `BrowserWindow.getAllWindows()` broadcast sites | **13** | |

Notes on two counts that drift from the plan documents:

- **`window.api.*` = 207 requires counting `.ts` files too.** The plan's stated command
  (`grep -rn "window\.api\." renderer --include=*.tsx | wc -l`) yields only **122 across 28 files** —
  the renderer's 14 `.ts` files hold the other 85 call sites. The 207 figure is correct; the grep in
  `13-checklist.md` is missing `--include=*.ts`.
- **22 files import `electron`, not 20.** Beyond the plan's figure, the notable one is
  `src/store/gitStore.ts` — its `import { app } from "electron"` is **entirely unused** (verified:
  zero references to `app` in the file). This answers the P2 item "Verify what `store/gitStore.ts`
  actually needs from `electron`": nothing. Delete the import.

## Baseline artifact list

Record the exact artifacts produced, so a later packaging regression is detectable:

- [ ] `release/Bifurc.Setup.exe` — size, builds, installs, runs *(re-verify at P6; `release/` predates the repo flattening)*
- [ ] `release/Bifurc.Setup.appx` — size, sideloads, runs
- [ ] `release/*.AppImage` — size, runs on a clean distro
- [ ] Tray icon renders (regression from commit `2477194` — see project memory)
- [ ] Window icon renders
