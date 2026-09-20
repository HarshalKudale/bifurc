# Bifurc — Project Memory

**Read `TRAPS.md` here first** — the failure modes that cost real time and are not
discoverable from the code. Then: status → `plan/README.md`; suites → `TESTING.md`; P6 and the §5c
ledger → `plan/07`; procedures → the `bifurc-*` skills.

## Layout

- **Flat repo — the app IS the root.** `workspaces: ["packages/*"]`; **never `cd bifurc`**.
- **App runs `packages/*/dist/`; tests run `src/`** → **`npm run build:packages` after ANY package
  edit.** The vitest alias covers `@bifurc/engine/*` **only**, so `@bifurc/protocol`/`@bifurc/client`
  resolve to built `dist/` — an un-rebuilt edit is **silently untested**; `npm test` doesn't build. The
  engine's `tsup` `entry` is a glob, so a new engine file needs no config change.
- **P6 is the only milestone.** **No renderer edits P1–P6**; `window.api` byte-identical.

## Phase state

- **P0–P2 done, P3 items 1–6 done** (item 5 open as *UI* → P7). **P4 8/10** — the extension release and
  an audit record, neither code here. **P5 done** — spec = `src/surface.ts` (144 keys);
  **`src/preload.ts` is the authority, not `renderer/types/window.ts`**; `close()` non-enumerable.
- **P6: steps 1–2, finding 1, 3a, 3b-1/2/3, 3c, 3d, 3e slices 1–7, work item 5 done.** 143 of 144 keys
  route; `checkUpdate` SPLIT → P12. `registerIpcHandlers()` cannot be deleted yet. **The e2e fixture
  asserts its own data-dir isolation**; on win32 `dataRoot()` ≠ `dataDir()`, both in temp (`TRAPS.md`).
- **§5c pass COMPLETE (2026-09-19).** `grep -rn '^commandRegistry.register(' src/` returns **nothing** —
  all **51** commands (not the 5 the section records) are registered by an explicit
  `register*Commands()` call from `registerIpcHandlers()`, across seven modules under `store/` `runner/`
  `protocols/` `applications/` `sync/` (`plan/07` §5c). **Count by grep, never by test** — the ratchet is
  blind to the defect but sharp on the *move*: forget the explicit call and it fails (`TRAPS.md`).
  **Now guarded** (`plan/07` §5c-viii): a static test fails on any module-scope registration, including
  the *indented* variant the pass's own grep missed. Step 3 is unblocked on the §5c side;
  it still needs `checkUpdate` (P12) and the dead `src/ipc/importExportHandlers.ts`.
- **The last key is a protocol decision, not a move:** `checkUpdate` SPLIT (its frozen result has no
  `currentVersion`/`hasUpdate`) → P12. `listAudit` and the two `runner.*Config` were **schema** problems,
  retired by widening rather than moving. **`BIFURC_ENGINE_RPC=0` does not exist anywhere** — decided:
  drop it; `git revert` is the only rollback layer.
- Dead, left on disk (`git rm` with the `src/` wipe): `src/ipc/importExportHandlers.ts`, `handlers/utils.ts`.

## The baseline you measure against

**101 files / 2,509 tests — 2,468 passed, 40 skipped, 1 failed.** The 1 is `soap.execute`, an
environmental failure (`TRAPS.md`). A *delta* against this number is the signal; the 1 is not.
