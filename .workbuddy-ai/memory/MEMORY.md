# Bifurc — Project Memory

**Read `TRAPS.md` in this directory before touching anything.** It holds the failure modes that have
cost real time and are not discoverable from the code. Then: status → `plan/README.md`; env and suites →
`TESTING.md`; P6 mechanics and the §5c ledger → `plan/07`; procedures → the `bifurc-*` skills.

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
- **P6: steps 1–2, finding 1, 3a, 3b-1/2/3, 3c, 3d, 3e slices 1–6 done.** 143 of 144 keys route;
  `checkUpdate` is SPLIT → P12. `registerIpcHandlers()` cannot be deleted yet.
- **§5c pass: 4 module-scope `commandRegistry.register(` calls remain — all four in
  `src/ipc/handlers/entityCrudFactory.ts`** (`entity.create`/`update`/`delete`/`list`). Moved so far:
  `store/coreCommands`, `runner/runnerCommands`, `store/folderCommands`,
  `protocols/{graphql,soap,grpc}Commands`, `applications/applicationCommands`, `sync/syncCommands`.
  **Count by grep, never by test** — the ratchet is blind to the defect (though sharp on the *move*:
  forget the explicit call and it fails, `TRAPS.md`). The four remaining are the awkward ones: the
  file's *exported* purpose is the CRUD collapse, six shell handler files import it, so importing any
  of them registers `entity.*` as a side effect — **seven suites** are affected, the widest set of the
  pass. Next slice = `plan/07` §5c-vii.
- **The last key is a protocol decision, not a move:** `checkUpdate` SPLIT (its frozen result has no
  `currentVersion`/`hasUpdate`) → P12. `listAudit` and the two `runner.*Config` were **schema** problems,
  retired by widening rather than moving. **`BIFURC_ENGINE_RPC=0` does not exist anywhere** — decided:
  drop the flag, `git revert` is the only rollback layer.
- Dead but left on disk (`git rm` correlated with the `src/` wipe): `src/ipc/importExportHandlers.ts`,
  `handlers/utils.ts`.

## The baseline you measure against

**101 files / 2,508 tests — 2,467 passed, 40 skipped, 1 failed.** The 1 is `soap.execute`, an
environmental failure (`TRAPS.md`). A *delta* against this number is the signal; the 1 is not.
