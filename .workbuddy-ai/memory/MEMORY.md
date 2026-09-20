# Bifurc — Project Memory

**Read `TRAPS.md` here first** — the failure modes that cost real time and are not
discoverable from the code. Then: status → `plan/README.md`; suites → `TESTING.md`; P6 and the §5c
ledger → `plan/07`; procedures → the `bifurc-*` skills.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
- **App runs `packages/*/dist/`; tests run `src/`** → **`npm run build:packages` after ANY package
  edit.** The vitest alias covers `@bifurc/engine/*` **only**, so `@bifurc/protocol`/`@bifurc/client`
  resolve to built `dist/` — an un-rebuilt edit is **silently untested**; `npm test` doesn't build.
- **P6 closed in scope 2026-09-20** — the ship milestone; everything after it is additive. **No renderer
  edits P1–P6**; `window.api` byte-identical.

## Phase state

- **P0–P2 done, P3 items 1–6 done** (item 5 open as *UI* → P7). **P4 8/10** — the extension release and
  an audit record, neither code here. **P5 done** — spec = `src/surface.ts` (144 keys);
  **`src/preload.ts` is the authority, not `renderer/types/window.ts`**; `close()` non-enumerable.
- **P6 CLOSED IN SCOPE 2026-09-20** (steps 1–2, findings 1/6, 3a, 3b-1/2/3, 3c, 3d, 3e slices 1–7, 3f,
  work items 4–5). **144 of 144 keys classified, zero `SPLIT`.** `registerIpcHandlers()` is **reduced,
  not deleted** — the local entry point already exists *beside* it (`registerClientHandlers()`,
  `main.ts:294`). The e2e fixture asserts its own data-dir isolation; on win32 `dataRoot()` ≠ `dataDir()`,
  both in temp (`TRAPS.md`). **Gate re-scoped by the owner's decision** (`plan/07` Acceptance criteria):
  criteria 5–8 **moved verbatim to work item 2**; 10 reworded ("contains the engine binary" is unmeetable
  in-process → "the packaged bundle resolves the engine from the asar"); 11 (`BIFURC_ENGINE_RPC=0`)
  **decided: dropped** — after step 3 there was one path, not two, so it would be dead code reading as a
  safety net. **One item CARRIED, not waived: criterion 2, "11 e2e specs pass, unmodified"** — owner's
  terminal only; re-measured 2026-09-20, reproduced the documented failure exactly (46 failed / 0 passed).
- **P6 work item 4 (packaging), measured 2026-09-20 — the plan's premise was stale.** "A packaged build
  cannot resolve `@bifurc/*`" came from an asar predating the deps; electron-builder copies the workspaces
  in with their `package.json` and **44 of 44** resolve, so **`build.files` is unchanged**. The real defect
  was **stale `dist/`** (`tsc` never deletes): 92 of 116 files, 447 KB, one requiring a module that no
  longer exists — the only unresolvable specifier. Fixed by `clean:main`; guarded by
  `scripts/assert-packaged.mjs` in `release.yml`. `tsconfig.json`'s claim that `tsc-alias` rewrites
  `@bifurc/engine/*` was **false**. `extraResources` waits on the process split. `plan/07` item 4.
- **§5c pass COMPLETE (2026-09-19), guarded (`plan/07` §5c-viii).** All **51** commands (not the 5 the
  section records) are registered by an explicit `register*Commands()` call from `registerIpcHandlers()`;
  a static test fails on any module-scope registration. **Count by grep, never by test** — the ratchet is
  blind to the defect but sharp on the *move* (`TRAPS.md`). Step 3's remaining work is the reduction of
  `registerIpcHandlers()` plus the dead `src/ipc/importExportHandlers.ts` — both unblocked, but see
  §5b-bis before starting.
- **All four pinned keys are resolved — nothing is outstanding.** `listAudit` + the two `runner.*Config`
  were **schema** problems retired by widening (step 3d). `checkUpdate` was resolved 2026-09-20 (step 3f)
  by **deleting `app.checkUpdate` from the protocol** and making the key client-local — so it was never
  P12's, and its frozen result's missing `currentVersion`/`hasUpdate` is moot. Recorded in
  `plan/protocol-changes.md`. Dead on disk until the `src/` wipe: `src/ipc/importExportHandlers.ts`,
  `handlers/utils.ts`.
- **§5b-bis (2026-09-20) — the step-3 reduction is an audit, not a deletion pass.** The 12 legacy files
  hold **132** channels: 84 literal + **48 generated** by `entityCrudFactory`, invisible to
  `grep ipcMain.handle("` and exactly the surface's 48 `entity` keys. **9 are superseded shell halves**
  owning a `dialog.*` / `app.getVersion()` step the client now does via `pickSavePath` /
  `readArtifactFile` — each needs its client-side twin confirmed before deletion, and deleting
  `tls:certStatus`'s channel today would delete the only producer of `trustState`. **`fileOpsClient.ts`
  survives** (live via `mimeFor` + `certLifecycle`). Found a **second lying cast**: `tlsCertStatus` loses
  `trustState`/`firefoxDetected` on the routed path — latent, P7 renders it (`TRAPS.md`).

## The baseline you measure against

**101 files / 2,507 tests — 2,466 passed, 40 skipped, 1 failed.** *(Was 2,509 / 2,468 until 2026-09-20:
step 3f deleted `app.checkUpdate` from the protocol, and `packages/protocol/src/commands/index.test.ts`
generates **two** tests per command — `accepts a valid payload` / `rejects an invalid payload` — so the
delta is exactly −2. **Always account for a delta by naming its mechanism**, not by waving at it.)* The 1
is `soap.execute`, an environmental failure (`TRAPS.md`). A *delta* against this number is the signal;
the 1 is not.
