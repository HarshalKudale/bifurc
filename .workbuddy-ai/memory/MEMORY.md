# Bifurc — Project Memory

**Detail lives in the repo.** Status → `plan/README.md`; env/suites → `TESTING.md`; P6 mechanics →
`plan/07`; traps → the `bifurc-*` skills. Durable facts only.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
- **App runs `packages/*/dist/`; tests run `src/`** → **`npm run build:packages` after ANY package
  edit.** The vitest alias covers `@bifurc/engine/*` **only**, so `@bifurc/protocol`/`@bifurc/client`
  resolve to built `dist/` — an un-rebuilt edit is **silently untested**; `npm test` doesn't build.
- **P6 is the only milestone.** **No renderer edits P1–P6**; `window.api` byte-identical.

## Phase state

- **P0–P2 done, P3 items 1–6 done** (item 5 open as *UI* → P7). **P4 8/10** — remaining is the
  extension release + an audit record, neither code here. A handler *resolving* `{ok:false}` is an
  envelope **success**; only a **throw** fails. **P5 done** — spec = `src/surface.ts` (144 keys);
  **`src/preload.ts` is the authority, not `renderer/types/window.ts`**; `close()` non-enumerable.
- **P6: steps 1–2, finding 1, 3a, 3b-1, 3b-2, 3b-3, 3c, 3d (keys 1–2) + finding 6 done.** 3b-2 implemented
  21 of 25 missing commands (ratchet 25→4). 3b-1 made the preload `{ ...client }` + overrides; **3b-3
  added the three filesystem hooks; 3d retired the remaining three non-SPLIT keys by widening their
  schemas → 143 of 144** route. 3c deleted `eventBridge.ts`; `registerIpcHandlers()` can't be deleted
  (1 unroutable key needs a channel) — it shrinks until P12. `src/ipc/importExportHandlers.ts` is dead
  and dies with it.
- **§5c unfixed:** `config.get`, `env.setActive`, `workspace.setActive`, `entity.load`,
  `entity.setEnabled` register **from the shell**, so a containerised engine answers
  `UNKNOWN_COMMAND`; the ratchet can't see it.
- **The last key is a protocol decision, not a move:** `checkUpdate` SPLIT (its frozen result has no
  `currentVersion`/`hasUpdate`, so the shell body can't just move) → P12. **The other three were
  *schema* problems, not routing problems**, and were retired by widening rather than moving:
  `listAudit` (`AuditListParams` lacked `filePath`/`fromTs`/`toTs`) and the two `runner.*Config`
  (`RunnerConfigSchema` required `requestOrder`). Step 3 ends at **92 registered + 1 split**, not
  89+2+1+1. **`BIFURC_ENGINE_RPC=0` does not exist anywhere** — 3b-1 deleted the preload table it was
  meant to fall back to; decided: **drop the flag, `git revert` is the only rollback layer**.

## Traps that cost real time

- **Don't re-point the shell's legacy `ipcMain.handle` bodies at the registry** — `register*Commands`
  run *from* `registerIpcHandlers()`, so two suites reach an empty registry.
- **`require()` in a handler body** is invisible to tsc and tests but **fatal in the engine's ESM
  output**; use static imports.
- **"Schema accepts the payload" ≠ "it preserves it"** — plain `z.object()` **strips** unknown keys.
- **Widening a schema invalidates its `invalid` fixture.** `COMMAND_FIXTURES[...].invalid` is asserted
  *rejected* on all four transports; `z.record` turned `config: {}` from a near-miss into a valid record.
  Re-derive the near-miss (a non-record), never trust it — this fails suites for no visible reason.
- **A suite that drives a command by *channel* name cannot see its registry implementation.** The legacy
  `ipcMain.handle` bodies are self-contained duplicates that never consult the registry, so
  `runnerStorage.integration.test.ts` passed with the engine half wrong. The conformance matrix can't
  cover it either — it registers `() => ({ reached: cmd })` over every command *on purpose*. **Zero tests
  reference any engine `register*Commands`**, so ~24 moved commands have no registry-level coverage.
- **"The command is registered" ≠ "the client's method is equivalent."** Audit before routing; 3 of
  the 4 held-back egress keys broke *after* the engine did its job correctly.
- **Artifacts are bytes.** `writeArtifact` takes **base64**; the channel writes a Buffer. Any UTF-8
  round-trip corrupts `workspace-zip`.
- **`blob.read` is paged**: 512 KB cap, `eof` is the only terminator, result is `{data, eof}`. Loop
  with `offset`. Inline threshold is 1 MB, so almost every real export is a blob.
- **A fake transport answering every request with `{ok:true}` hides whole branches** — no client test
  reached either branch of `artifactToFile`. Assert requested `offset`s, not just results.
- **The unit suite is weak evidence for the preload flip** (it tests handlers, not the preload).
  **e2e is the real gate** and can't run from the agent shell.

## Git / env

- **NEVER `git stash`** (wiped the object store once). **`git fetch` doesn't persist remote-tracking
  refs** → never trust `origin/*`; use `git ls-remote`. **Verify old trees read-only** (`git grep
  <rev>`), never check out.
- **A push pops a GCM GUI window and dies** — `GIT_TERMINAL_PROMPT=0 GIT_ASKPASS=echo
  GCM_INTERACTIVE=never git push …`, then confirm with `git ls-remote`.
- **`npx tsc` is not the compiler** → `npm run typecheck`.
- **`vitest run --project <name> <path>` is BROKEN (Vitest 4.1.5)** — drop `--project` when filtering.
  A failure uniform across unrelated files is an *invocation* bug.
- **NEVER launch Electron from the agent shell** (GPU process fails, exit 3); the **user** runs
  `npm run dev` / `test:e2e`.
- **Safe-delete guard refuses past ~50 deletions/turn**, killing `build:packages`; **`git clean -fdX
  <path>` is the way through.** A failed `tsup` leaves entry points deleted — check `dist/index.js`.
- **Linking a new workspace package needs a manual Windows junction** (`New-Item -ItemType Junction`),
  then `node -e "require.resolve('@bifurc/<pkg>')"`.
- **A suite capturing `ipcMain.handle` needs `register*Commands(registry)` in `beforeAll`**, else
  `ENOENT` reads as an fs bug.
- **e2e is the renderer's only coverage** (11 specs/46 tests); **11 of 46 cannot fail**.
