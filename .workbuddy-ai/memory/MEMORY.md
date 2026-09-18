# Bifurc — Project Memory

**Detail lives in the repo.** Status → `plan/README.md`; env/suites → `TESTING.md`; history → `memory/`.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
- **App runs `packages/engine/dist/`; tests run `src/`** (vitest alias) → **`npm run build:packages` after
  engine edits.** **`@bifurc/protocol` has NO alias** → an un-rebuilt protocol edit is **silently untested**;
  **`npm test` does not build first.** Root `tsc` is `include: ["src/**/*"]` → **root `tests/**` unchecked**.
- **P6 is the only milestone.** **No renderer edits P1–P6**; `window.api` byte-identical through P5–P6;
  `../bifurc-extension`'s 4 commands are **frozen**.

## Phase state

- **P4 — 8/10.** Open = the **extension release** + item 4's audit **record**. Mechanics → `TESTING.md` §4.12
  and the `bifurc-transport-layer` skill. A handler that *resolves* `{ok:false, error}` is an envelope-level
  **success** (only a **throw** fails → **two `ok` flags**). **Conflict resolution is a privilege** — gate
  `onAddConflict`/`entity.setEnabled` on **`mayAffectUnnamedEntities(ctx)`** (= `admin`), since `enabled`
  lives **only in `enabled.json`**.
- **P5 — done 2026-09-18.** `packages/client`, 7/7 criteria, 5 suites / 54 tests. Spec = **`src/surface.ts`**
  (the **144-key** surface). **`src/preload.ts` is the authority, not `renderer/types/window.ts`** — the type
  declares 140 and is wrong in **two directions**: 4 omitted (`isFirstLaunch`, `completeFirstLaunch`,
  `getZoomLevel`, `setZoomLevel`) and 3 marked optional that the preload always provides
  (`setTitleBarOverlay`, `getTheme`, `setTheme`). Traps → `bifurc-client-surface` skill. Three that bite:
  **`close()` is non-enumerable** (`Object.keys` *is* the regression instrument); **retry =
  `isRetryable(code)` && `isIdempotent(command)` && transport-open**; **a `packages/*/tsconfig.json` must have
  NO comments** (oxc rejects JSONC → `[TSCONFIG_ERROR]`), and `shape.test-d.ts` is checked **only** by that
  tsconfig. **Per-command timeouts do NOT exist** — `COMMANDS[action]` is `{params, legacyChannel}`.
- **P6 — in progress.** Two blocking findings (detail → `plan/07`): **`log.entry`/`log.chunk`/`server.error`
  never reach the bus** (only `logEmitter`), so a transport-only P6 silently kills the capture + log panels;
  and **`ipcMain.handle`→`ipcRenderer.invoke` drops `err.code`**, which `withRetry` branches on.

## Env

- **NEVER `git stash`** — 2026-09-16 it wiped `.git/objects/pack`.
- **`npx tsc` is NOT the compiler** → use `npm run typecheck`.
- **NEVER launch Electron from the agent shell.** The GPU process can't start, Electron **hard-exits**
  (`FATAL: GPU process isn't usable`, exit 3), and every e2e test reads as `firstWindow: Target page…
  closed`. **A minimal 5-line Electron app fails identically**, and `dangerouslyDisableSandbox` does **not**
  help — so it is the shell's process context, **not the app**. The user runs `npm run dev` / `test:e2e`;
  **no flags are ever needed, packaged builds included.**
- **Safe-delete guard counts deletions per turn**, refusing past 50 → `build:packages` and **Playwright's
  `test-results` cleanup** die with `SAFE_DELETE_BULK_CONFIRM_REQUIRED`; prefix
  `env -u CODEBUDDY_SAFE_DELETE_BULK_STATE_DIR -u CODEBUDDY_TOOL_CALL_ID`.
- **A converted IPC channel needs `register*Commands(registry)` in the `beforeAll` of any suite capturing
  `ipcMain.handle`** — else `No handler registered`+`ENOENT` reads as an fs bug.
- **e2e is the renderer's only coverage** (11 specs / 46 tests) and **11 of those 46 cannot fail** —
  `expect(typeof isVisible).toBe("boolean")` tautologies plus guarded assertions. `PANEL_REGISTRY`'s
  **`showInSidebar: false`** means **`nav-settings` / `nav-environments` / `nav-workspace` / `nav-audit` are
  never rendered**, and there is **no `graphql`/`soap`/`grpc` panel at all** (they are `chooseProtocol()`
  choices). Detail → `bifurc-test-coverage` skill.
