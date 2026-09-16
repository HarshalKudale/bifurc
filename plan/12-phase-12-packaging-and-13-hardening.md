# 13 — Phase 12: Packaging and distribution

**Goal:** ship all four artifacts — NSIS, APPX, AppImage and Docker — from CI, with the engine bundled
correctly in each.

**Effort:** 2–3 weeks. **Depends on:** P6 (first change), P9 (Docker). **Touches:** the shipping phases.

> Packaging is not a phase you do once at the end — it changes at **two** points now (was three). This
> document tracks both so nothing is discovered late.
>
> **Change point 3 (Tauri) is not applicable** — P11 is parked under D5, so electron-builder keeps
> producing NSIS, APPX and AppImage exactly as it does today.

---

## Current state

| Target | Configured where | Artifact |
|---|---|---|
| Windows NSIS | `package.json` → `build.win.target` | `Bifurc.Setup.exe` |
| Windows APPX (MS Store) | `build.appx` with Store identity | `Bifurc.Setup.appx` |
| Linux AppImage | **CI only** — `.github/workflows/release.yml:117` (`--linux AppImage`); no `linux` block in `package.json` | `*.AppImage` |
| Docker | does not exist | — |

Store identity is inline in `package.json`: `identityName: HarshalKudale.Bifurc`,
`publisher: CN=C594A390-264F-4DE0-ABF9-524B641E479D`, `applicationId: Bifurc`, with logos in `build/appx/`.

Release flow (`.github/workflows/release.yml`): `prepare` → `build-windows` (windows-latest) +
`build-linux` (ubuntu-latest) → `publish-release` via `softprops/action-gh-release`.

---

## Change point 1 — at P6 (engine bundled into the shell)

**Smallest change, biggest risk of a packaging regression.**

```json
// package.json
"extraResources": [
  { "from": "build/tray-icon.png", "to": "tray-icon.png" },
  { "from": "build/icon.png",      "to": "icon.png" },
  { "from": "../packages/engine/dist/engine", "to": "engine" }
]
```

| Item | Action |
|---|---|
| Engine binary in `extraResources` | As above |
| Dev-vs-packaged path resolution | **Reuse `iconPath()`** (`main.ts:36–39`) — it already handles `app.isPackaged` with a dev fallback. Do not add a second convention. |
| `asar` | The engine binary must be **outside** the asar (`extraResources` is). A binary inside an asar cannot be executed. |
| Workspace restructure | `files` / `directories` may need updating now that deps live in `packages/*`. **See the measured note below — this row is now concrete, and the in-process case is already broken.** |

### Measured 2026-09-15 — the **in-process** case has no packaging support today

The section above describes the P6 end state, where the engine is a **separate process** launched from
`extraResources`. Until then the engine is an ordinary npm package that `dist/main.js` `require()`s
**in-process** — and that path has no packaging support at all right now.

Evidence, from the last real build (`release/win-unpacked/resources/app.asar`, built 2026-09-10) plus
the current `dist/`:

| Fact | Value |
|---|---|
| `build.files` | `["dist/**/*", "package.json"]` |
| asar top level | `node_modules/`, `dist/`, `package.json` — electron-builder *does* add production deps |
| `node_modules/@bifurc` **in the asar** | **absent** |
| app files requiring `@bifurc/protocol` | `dist/commands/registry.js` |
| app files requiring `@bifurc/engine` | 20+, including `dist/main.js` |

`@bifurc/protocol` (since P2 item 7) and `@bifurc/engine` (since item 8) are declared as root
dependencies, but they resolve through **workspace symlinks** into `packages/`. electron-builder does
not dereference those into the asar, so a packaged build has no way to resolve either specifier.

**Stated precisely:** the 2026-09-10 asar predates both dependencies, so it cannot demonstrate the
failure directly — it confirms only that nothing outside `dist/`, `package.json` and production
`node_modules` is copied. Confirming the failure end-to-end needs one `electron-builder --dir` run
against the current tree.

This is **not** a P2 blocker — dev mode and the whole test suite resolve fine through the symlink, and
P6 replaces the mechanism anyway. But it must not be discovered at a release build. When it lands,
two options: add `packages/{engine,protocol}/dist/**/*` to `files` **and** make the bare specifier
resolvable, or have `tsc-alias` rewrite `@bifurc/*` to a relative path into a vendored directory.
The CI assertion at the end of this section should be extended to cover whichever is chosen.

### There is a precedent for this exact failure

`src/main.ts` previously broke because icons moved to `build/` and were dropped from `build.files`, while
the code still loaded them from `__dirname/..`. `nativeImage.createFromPath()` on a missing path returns
an **empty image without throwing**, so the app shipped with a blank tray icon and no error.

**The engine binary has the same failure mode but worse:** a missing binary means the app cannot start at
all. Add a CI assertion that the packaged output contains the engine binary **before** the artifact is
published. Do not rely on a runtime error to catch it.

```bash
# after packaging, before upload
ls -la release/win-unpacked/resources/engine* || exit 1
```

---

## Change point 2 — at P9 (Docker)

Covered in `10-phase-9-docker.md`. Packaging-relevant items:

- Publish the image as part of the release workflow, tagged with the same version (D9 lockstep).
- Multi-arch manifest if the P0 decision chose it.
- Do not publish `latest` from a non-tag build.

---

## Change point 3 — at P11 (Tauri) ⏸ NOT APPLICABLE

*Removed under D5. Retained for the day the shell migration resumes.*

**This change point no longer exists.** With P11 parked, Electron remains the shell and
electron-builder continues to produce **NSIS, APPX and AppImage** exactly as it does today. **There is no
MSIX problem**, because there is no Tauri bundler involved.

This is the single largest simplification the decisions bought. Packaging now has **two** change points
instead of three, and the one that remains on the critical path (P6) is the smallest of the three.

The full Tauri packaging problem, preserved for later:

> Tauri v2's Windows bundler emits **NSIS + MSI (WiX)** only. There is **no `appx`/`msix` target**, so
> the Store artifact would have to be hand-authored from a Tauri layout — `makeappx`, a manifest, Store
> logos and a signing pipeline. Options would be: (a) keep electron-builder for the Store build only,
> (b) hand-author MSIX, or (c) drop the Store. Under (b), the sidecar binary must be inside the MSIX
> layout, and the P0 spike 1 findings determine whether it can be spawned or must run in-process. The
> AppImage would also need verification that Tauri bundles or correctly depends on `webkit2gtk-4.1`.

---

## Work item — the `app:checkUpdate` split

`systemHandlers.ts:85` fetches GitHub Releases and selects an asset by `process.platform`. With split
artifacts this becomes ambiguous — is it updating the shell or the engine?

Under D9 (lockstep), keep them one version and let the shell own the update:
- Shell update → shell's own mechanism (Electron updater / Tauri updater).
- Engine → updated as part of the shell payload, or via the container tag.
- **`process.platform` will not exist in a browser**, so this handler must be client-side, not engine-side.

---

## Work item — CI rework

```yaml
jobs:
  test:                # unit + integration
  build-desktop-win:   # NSIS + APPX (engine bundled)
  build-desktop-linux: # AppImage
  build-docker:        # image + multi-arch manifest
  assert-artifacts:    # verify engine binary present in each bundle
  publish:             # gh-release + registry push
  # build-engine:      # P10 matrix — only if P10 is resumed (D2/D4). Not on the critical path.
```

**Add the `assert-artifacts` job.** It is the guard against the icon-class of regression and it costs
about ten lines.

**Note the CI is already flattened.** `.github/workflows/release.yml` now uses `working-directory: .`
and `release/*.exe` (was `bifurc/release/`), and `require('./package.json')` (was
`require('./bifurc/package.json')`). Verify this survived the flattening before adding jobs.

---

## Acceptance criteria

- [ ] NSIS installer contains the engine binary and the app runs from a clean install.
- [ ] APPX build contains the engine binary and runs from a Store-style install.
- [ ] AppImage contains the engine binary and runs on a clean distro.
- [ ] Docker image publishes with the version tag; runs headless.
- [ ] `assert-artifacts` fails the build if the engine binary is missing from any bundle.
- [ ] `app:checkUpdate` moved client-side; no `process.platform` in the engine.
- [ ] All artifacts carry the same version (D9).
- [ ] Release notes are generated and accurate.
- [ ] Store identity (`build.appx` config) unchanged by the workspace restructure.
- [ ] macOS artifacts exist if macOS is a supported target (currently not in the release workflow).

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Engine binary missing from a bundle, discovered at runtime | **High** | `assert-artifacts` CI job; the icon regression already happened once |
| Engine placed inside the asar and cannot execute | Medium | `extraResources` only; assert the path |
| Version skew between shell, engine and Docker image | Medium | D9 lockstep; single version source in CI |
| Store identity details lost during the workspace restructure | Medium | Copy `build.appx` config verbatim; diff against the current file |
| CI paths left pointing at the old `bifurc/` prefix | Medium | Grep the workflows for `bifurc/` before the first release after P6 |

*MSIX authoring and Tauri AppImage/WebKitGTK risks removed — see change point 3.*

---

# 14 — Phase 13: Hardening

**Goal:** close the security, observability and documentation debt created by the split.

**Effort:** 2–3 weeks. **Depends on:** P6. **Runs continuously, plus a dedicated pass here.**

> Some of this is already done in P4 (auth, scopes, RPC audit). This phase verifies it, fills the gaps,
> and makes the system operable by someone who did not build it.

---

## Work item 1 — Security review

| Item | Verify |
|---|---|
| Auth cannot be bypassed | No unauthenticated code path; no "local requests skip auth" shortcut |
| Scope enforcement | Every command class checked; `read` cannot mutate; `execute` cannot admin |
| Bind defaults | Loopback or socket; `0.0.0.0` requires explicit opt-in |
| Docker fails closed | No token + non-loopback bind → refuse to start |
| Token handling | `0600` on disk; never logged; rotation works |
| Blob path traversal | A crafted `blobId` cannot escape the blob root. **Test this explicitly.** |
| Blob size cap | Enforced before allocation, not after |
| Command injection | `processSpawner` and `certManager` shell out. Audit every `exec`/`spawn` for unsanitised input. |
| RPC audit coverage | Every mutating command recorded |
| Dependency audit | `npm audit` clean, or every finding triaged |

**The blob path traversal check is the one most likely to be missed.** A blob id arrives over the
network and is used to build a filesystem path.

---

## Work item 2 — Observability

- **Structured logs** from the engine (JSON), with a correlation id per RPC request.
- **A single diagnostic command**: `bifurc doctor` — engine version, protocol version, data dir,
  writability, git presence, port/socket state, connected clients, blob store size. One command that
  answers "why is this broken".
- **Client-side error surfacing.** Protocol errors carry `code`; the UI should show an actionable message
  per code, not `Error: something went wrong`.
- **Connection state** visible in the UI at all times (the P7 footer).

---

## Work item 3 — Documentation

| Document | Contents |
|---|---|
| `plan/protocol-changes.md` | Every post-freeze protocol change with justification |
| `docs/protocol.md` | The command surface, generated from schemas |
| `docs/architecture.md` | The diagram from `Decoupling_Plan_v2.md` §5, updated to reality |
| `docs/docker.md` | The full CA workflow end to end, including Firefox |
| `docs/troubleshooting.md` | Version mismatch, auth failure, orphaned engine, stale CA, Firefox trust |
| `CONTRIBUTING.md` | Update for the workspace layout (it currently describes a single-package repo) |

---

## Work item 4 — Performance budgets

Set and enforce numbers, or regressions will accumulate invisibly.

| Metric | Budget | Where measured |
|---|---|---|
| Cold start (shell → usable UI) | ≤ baseline from P6 | e2e |
| Engine cold start | ≤ 500 ms | CI |
| Idle RSS (engine) | ≤ 120 MB | CI |
| Idle RSS (shell) | Electron: unchanged; Tauri: ≤ 80 MB | CI |
| RPC round-trip (local) | ≤ 5 ms p95 | conformance |
| `log.entry` throughput before dropping | ≥ 2,000/s | conformance |
| Install size | Electron unchanged; Tauri ≤ 20 MB | CI |

The **install-size budget is the whole justification for P11** — if it is not met, the Tauri migration
did not deliver its reason to exist.

---

## Work item 5 — Close the pre-existing debt

From the project memory and `Cleanup_plan.md`:

- **Renderer is not type-checked by default.** `npm run typecheck` covers `src/**` only.
  `npm run typecheck:renderer` reports ~154 pre-existing errors. Fix, then gate it in CI. With the
  renderer now shared across three clients, this matters much more than it did.
- **Coverage thresholds are a ratchet.** Raise them as gaps close; never lower them to make CI pass.
- **`mkId()` collision risk** (`src/proxy/serverUtils.ts`) — a 4-char random suffix, collidable in a
  same-millisecond burst. Widen it before it is relied on for durable ids.
- **Guarded e2e specs**: `protocols`, `environments`, `capture-ws-webhooks`, `settings`, `screenshots`.
  Close them or delete them — a guarded test is not a test.

---

## Acceptance criteria

- [ ] Security review complete; every item verified with evidence, not assertion.
- [ ] Blob path traversal tested and blocked.
- [ ] Every `exec`/`spawn` call site audited for injection.
- [ ] `bifurc doctor` implemented.
- [ ] Protocol errors map to actionable UI messages.
- [ ] All six docs written.
- [ ] Performance budgets measured in CI and enforced.
- [ ] `typecheck:renderer` clean and gated.
- [ ] Coverage thresholds raised to the post-refactor level.
- [ ] `mkId()` widened.
- [ ] No guarded e2e specs remain.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Security review becomes a rubber stamp | Medium | Require evidence per item, not a checkbox |
| Docs written last and skipped | **High** | Write `protocol.md` from generated schemas — it costs nothing |
| Performance budgets set but not enforced | Medium | Put them in CI as failing thresholds |
| Blob path traversal overlooked | Medium | Explicit test in the acceptance criteria |
| Pre-existing debt never closes | **High** | It is in the acceptance criteria, not a "nice to have" |
