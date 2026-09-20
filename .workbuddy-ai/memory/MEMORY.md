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
- **§5b-bis — the step-3 audit (2026-09-20): the legacy channels are not uniformly dead.** **132** legacy
  channels (84 literal + **48 generated** by `entityCrudFactory` — a literal `grep` misses all 48, the same
  undercount shape as §5c's "five, actually 51"), split **DEAD PASSTHROUGH ~120 / SUPERSEDED SHELL HALF 9 /
  LOCAL ALREADY ELSEWHERE 15** (`clientHandlers.ts`'s 15 are untouched). The 9 twins are **verified per
  key**: 8 equivalent-or-better, 1 **regression** — `exportAudit` had taken `artifactToFile`'s
  artifact-first default, undoing P3's dialog-first ordering for the one egress artifact with no size
  ceiling (`audit.export` is `queryLog({ limit: 0 })`, the whole log). Fixed, **and the `local.ts`
  docblock that sanctioned it corrected** — a wrong doc regenerates the bug, so fix both (`TRAPS.md`).
  **The `tls` pair was the second find, and worse:** `tlsCertStatus` lost 3 drift fields (latent — P7
  renders them), but `tlsRemoveCert` was **live** — `TlsSettingsSection.tsx:57` calls it, so "remove
  certificate" deleted the engine's keypair and left the OS store trusting the CA. **Owner's decision
  (2026-09-20): both keys are `local`** on the `tlsInstallCA` precedent. `surface.ts` is now
  **70 transport + 48 entity + 7 subscribe + 3 shim + 16 local = 144**; `ClientLocal` gained the two
  members; `certLifecycle.readCertStatus()` was extracted so the 8-field composition has one
  implementation instead of living in a channel body a routed twin could half-fill. Class 2 is **9 → 11**.
  `src/ipc/fileOpsClient.ts` **survives** the reduction (live shared library, two live paths).
- **§5b-ter — the pass's shape, found by starting it (2026-09-20).** Two corrections, both of which the
  audit's own "Not verified" paragraph predicted. (1) **The 48 generated channels need a factory *split*
  first**: `registerEntityCrudHandlers(opts)` also calls `registerCrudKind(opts)`, which populates the map
  the four `entity.*` commands dispatch through — delete it and 16 kinds throw the misleading *"is not
  routed through the CommandRegistry yet"*. (2) **The pass is a test migration, not a deletion**: eight
  integration suites register handler groups by hand and drive the channels (`foldersAndWorkspaces` 45
  invocations, `protocolExecution` 62), so each slice must re-point them at `commandRegistry.invoke()`.
  **The unit is the handler group**, not the file — and `soap`/`graphql`/`grpcHandlers` are each the
  factory *plus* two pure passthroughs, so they belong with the factory slice.
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
