# Bifurc — Project Memory

**Detail lives in the repo.** Status → `plan/README.md`; env/suites → `TESTING.md`; phase mechanics →
`plan/07`; traps → the `bifurc-*` skills. Keep this file to durable facts only.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
- **App runs `packages/engine/dist/`; tests run `src/`** (vitest alias) → **`npm run build:packages` after
  engine edits.** `vitest.config.ts`'s alias covers `@bifurc/engine/*` **only** → `@bifurc/protocol` and
  `@bifurc/client` resolve to their built `dist/`, so an un-rebuilt edit to either is **silently
  untested**; **`npm test` does not build first.** Root `tsc` is `include: ["src/**/*"]` → root
  `tests/**` unchecked.
- **P6 is the only milestone.** **No renderer edits P1–P6**; `window.api` byte-identical through P5–P6;
  `../bifurc-extension`'s 4 commands are **frozen**. **Only 3 shell files may change in P6**: `main.ts`,
  `preload.ts`, the shell-only handler module.

## Phase state

- **P0–P2 done. P3 items 1–6 done** (item 5 open as *UI*, deferred to P7 — renderer edit).
- **P4 — 8/10.** Open = the **extension release** + item 4's audit **record** (neither is code here).
  A handler that *resolves* `{ok:false,error}` is an envelope-level **success** (only a **throw** fails).
- **P5 done 2026-09-18.** `packages/client`, 5 suites / 54 tests. Spec = **`src/surface.ts`** (the
  **144-key** surface). **`src/preload.ts` is the authority, not `renderer/types/window.ts`** (declares
  140, wrong both ways). Regressions are caught by `packages/client/tests/surface.test.ts` — it imports
  the **real** preload under a mocked Electron and diffs `Object.keys` against `SURFACE_KEYS`, so
  `close()` must stay **non-enumerable**. Mechanics → `bifurc-client-surface` skill.
- **P6 — step 2, finding 1, and step 3a done.** The seam carries `config:get` over
  `client → ipcTransport → ipcRenderer.invoke("engine:rpc") → rpcBridge → registry.invoke`; events ride
  a **second** channel (`EVENT_CHANNEL`, `webContents.send` → `ipcRenderer.on`) because `invoke` is
  request/response. `TransportKind` gained `"ipc"`. Legacy `registerIpcHandlers()` still runs — deleting
  it is the **last** act of step 3.
- **P6 — step 3b-2 DONE (2026-09-18): 21 of 25 moved**, ratchet **25 → 19 → 15 → 10 → 5 → 4**, across
  `proxy/serverCommands.ts` (6), `store/configCommands.ts` (4), `proxy/webhookCommands.ts` (5),
  `miscCommands.ts` (5), `runner/runnerCommands.ts` (1). `MOVABLE` is **deleted, not emptied** — an empty
  array invites the next unimplemented command to be filed there by default. What remains is not a
  backlog: **NARROWED 1** (`audit.list` — schema omits `filePath`; latent), **BLOCKED 2**
  (`runner.saveConfig`/`loadConfig` — a *passing* test saves a config `RunnerConfigSchema` rejects, and
  the ratchet asserts the block itself), **SPLIT 1** (`app.checkUpdate` — P12). Step 3 ends at
  **89 + 2 + 1 + 1**, not 93. **Three traps:** (1) do **NOT** re-point the shell's legacy `ipcMain.handle`
  bodies at the registry (`register*Commands` are called *from* `registerIpcHandlers()`, so two suites
  reach an empty registry → `UNKNOWN_COMMAND` at invocation time); (2) **`require()` inside a handler
  body** is invisible to `tsc` and to tests but **fatal in the engine's ESM output** — `tsup` emits
  `.mjs` too, so it throws at call time; use static imports; (3) **"the schema accepts the payload" ≠
  "it preserves it"** — a plain `z.object()` **strips** unknown keys, so check for truncation before
  moving anything carrying a big object. **5c, unfixed:** `config.get`, `env.setActive`,
  `workspace.setActive`, `entity.load`, `entity.setEnabled` are registered **from the shell**, so a
  containerised engine (P9) answers `UNKNOWN_COMMAND`; the ratchet cannot see it.
- **P6 — step 3b-1 DONE (2026-09-18): the preload flip.** `src/preload.ts` is no longer a 136-call
  channel table; it is `{ ...client }` plus **13 overrides**, so **131 of 144 keys** route through the
  bridge. The table could be deleted outright because **`src/preload.ts` was always the contract, not
  `renderer/types/window.ts`**, and `@bifurc/client` satisfies it **positional args included** — so the
  flip is mechanical, not a rewrite. **13 held back, two groups:** 4 with no registry implementation
  (`checkUpdate`, `listAudit`, `saveRunnerConfig`, `loadRunnerConfig`) and 9 artifact-egress needing two
  `ClientLocal` hooks the shell lacks (`writeArtifact` / `readArtifactFile`) — routing those would
  resolve `{ok:false,"cannot write files"}`, worse than the working channel. **Two preconditions that
  were checked, not assumed:** `config.save` no longer calls `updateTrayMenu()` — the registry emits
  `settings.changed` and **`src/main.ts:299` subscribes**; and `event.log.entry` arrives as a **batch**
  while the renderer wants one entry, so `onLogEntry` is the one subscription the client does **not**
  pass through. **The unit suite is weak evidence for the flip** — it tests handlers, not the preload,
  whose only coverage is the key count — so **e2e is the real gate**.
- **P6 — next: the two `ClientLocal` hooks (routes the last 9), then 3c** — delete
  `registerIpcHandlers()` + `eventBridge.ts`. **3c cannot delete everything**: the 4 unroutable commands
  must keep a channel, so `registerIpcHandlers()` shrinks rather than disappears until P12 and a
  protocol change resolve them.

## Git

- **P3–P6 committed** (`68f9cb7`, `4e4f84c`, `2fb78b5`, `8dcaadb`, … through `8471b7b`). `origin/
  standalone-engine` tracks HEAD. **Verify old trees read-only (`git grep <rev>`) — never check out;
  the object store has been lost once.**
- **A push pops a GCM GUI window and dies without it.** Push with
  `GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=echo GCM_INTERACTIVE=never git push …`, then confirm via
  `git ls-remote origin standalone-engine`.

## Env

- **NEVER `git stash`** (wiped `.git/objects/pack`). **`git fetch` here does NOT persist
  remote-tracking refs** → never trust `origin/*`; ask with `git ls-remote`.
- **`npx tsc` is not the compiler** → `npm run typecheck`.
- **`vitest run --project <name> <path>` is BROKEN (Vitest 4.1.5)** — every file dies at the first
  `describe`. Drop `--project` when filtering by path. **`npm run test:unit` / `test:integration` are
  fine.** Lesson: a failure uniform across files that share nothing is an *invocation* bug.
- **NEVER launch Electron from the agent shell** — GPU process can't start, Electron hard-exits (exit 3);
  a 5-line app fails identically. The **user** runs `npm run dev` / `test:e2e`.
- **Safe-delete guard refuses past ~50 deletions per turn**, killing `build:packages` and Playwright
  cleanup. The `env -u …` prefix is unreliable. **`git clean -fdX <path>` is the way through** (git does
  its own unlink). **A failed `tsup` run leaves the package entry points deleted** — check
  `dist/index.js`/`.mjs`/`.d.ts` exist afterwards (`dist/` is gitignored, git won't tell you).
- **Linking a new workspace package needs a manual Windows junction** — `npm install` can't (it runs
  `build:packages`, which trips the guard) and Git Bash's `ln -s` yields an **empty directory**.
  `New-Item -ItemType Junction -Path node_modules\@bifurc\<pkg> -Target packages\<pkg>`, then
  `node -e "require.resolve('@bifurc/<pkg>')"`.
- **A suite capturing `ipcMain.handle` needs `register*Commands(registry)` in `beforeAll`**, else
  `No handler registered` + `ENOENT` reads as an fs bug.
- **e2e is the renderer's only coverage** (11 specs / 46 tests) and **11 of those 46 cannot fail**.
  Detail → `bifurc-test-coverage` skill.
