# 01 — Phase 0: De-risk spikes

**Goal:** resolve the unknowns that can invalidate the architecture, before any production code is
written. Each spike is a throwaway prototype with a yes/no answer.

**Effort:** 1 week (spike 4 only). **Depends on:** D1–D4 (resolved). **Blocks:** P1 and P2.

> **Do this phase first. It is the cheapest place to discover a problem and the most expensive place to
> skip one.** Every spike below is time-boxed — if a spike blows its box, that *is* the answer.

---

## Scope change — 2026-09-14, following D1/D4/D5

D4 and D5 both defer the shell migration, so **spikes 1 and 3 are no longer required**. They existed to
de-risk a Tauri shell that is now parked.

| Spike | Status | Why |
|---|---|---|
| 1 — MSIX loopback / child process | ⏸ **skipped** | Electron stays; electron-builder already produces a working APPX. Parked, not deleted. |
| 2 — Bun compile | 🔸 **optional** | Nothing on the critical path needs a standalone binary. Worth running only if you want a single-binary CLI or intend to resume P10. |
| 3 — WebKitGTK CSS | ⏸ **skipped** | Electron ships Chromium. Parked, not deleted. |
| 4 — Protocol proof of concept | ✅ **required** | The only spike that gates P1. |

**Run spike 4. Then proceed to P1.** Spikes 1–3 are retained below with their methods intact — if D4/D5
are ever revisited, they are the first things to run and the thinking is already done.

If you want a cheap early answer on D2, spike 2 is a reasonable 3-day investment before P10 — but it
blocks nothing, so it should not delay P1.

---

## Preconditions

- D1–D4 answered in `00-decisions.md` — ✅ done.
- Spike 4's throwaway branch ready.

---

## Spike 1 — MSIX: loopback and child-process spawning ⏸ SKIPPED

*Skipped under D5. Retained for the day the shell migration resumes.*

**Time box: 3 days.** Answers D3 and D5. This was the highest-risk item in the programme.

### Hypothesis

A full-trust desktop-bridge Electron app packaged as MSIX can (a) bind and connect over loopback, and
(b) spawn and supervise a child process that also uses the network.

### Why it might fail

MSIX runs apps in a container with **virtualised filesystem and registry writes**. UWP/WinRT apps are
subject to a loopback restriction; **full-trust desktop-bridge apps are not** — but the child process
inherits the package identity and container, and its writable paths get redirected. Neither behaviour is
documented clearly enough to bet the architecture on.

### Method — exploit what already exists

You already have a loopback WebSocket server (`src/companion/companionServer.ts`, port 9271) and a
released client for it — the companion extension, now in its own repository. Do not build a new harness.

1. **Loopback test.** Build and package the app as APPX as it stands today:
   ```bash
   npm run package -- --win appx --publish never
   ```
   Install `release/Bifurc.Setup.appx`. Add a temporary button in the renderer that opens
   `ws://127.0.0.1:9271`, sends `{id:"1",action:"config:get",payload:null}`, and displays the reply.
   **Pass:** a valid `{ok:true,...}` response. **Fail:** connection refused / timeout.

2. **Child-process test.** Add a temporary IPC handler that spawns a trivial child and reports back:
   ```ts
   // temporary — src/ipc/handlers/systemHandlers.ts
   ipcMain.handle("spike:spawnChild", async () => {
     const cp = require("child_process");
     const child = cp.spawn(process.execPath, ["-e", "console.log('child-alive'); setTimeout(()=>{}, 3000)"], {
       env: { ...process.env, ELECTRON_RUN_AS_NODE: "1" },
       stdio: ["ignore", "pipe", "pipe"],
     });
     return await new Promise((resolve) => {
       let out = "";
       child.stdout.on("data", (d) => { out += d.toString(); });
       child.on("error", (e) => resolve({ ok: false, error: String(e) }));
       child.on("exit", (code) => resolve({ ok: out.includes("child-alive"), code, out }));
     });
   });
   ```
   Rebuild, repackage, install, invoke. **Pass:** `{ok:true}`.

3. **Path-virtualisation test.** From inside the packaged app, write to a path outside the package
   (e.g. `%LOCALAPPDATA%\Bifurc\spike.txt` via the engine's data dir) and then verify from *outside* the
   app that the file exists at that literal path. **Pass:** the file is where you expect. **Fail:** it
   landed in `%LOCALAPPDATA%\Packages\<identity>\LocalCache\...` — which means the engine's data dir
   is virtualised and a remote client cannot point at it.

4. **Spawning a sibling binary.** Copy a small standalone executable (any — even a copy of
   `node.exe`) into the package via `extraResources`, spawn it, and confirm it runs.

### Outcomes and what each means

| Result | Consequence |
|---|---|
| All four pass | MSIX uses an in-package spawned engine. Proceed. |
| Loopback fails | Use a **named pipe** for the MSIX build (this is why D3 prefers sockets). |
| Child spawn fails | MSIX build uses the **embedded in-process engine**; the transport abstraction must support it. |
| Path virtualisation surprises | Engine data dir must be resolved at runtime and reported to clients, never assumed. |

### Deliverable

A short note in `plan/spike-results.md` with raw evidence (screenshots, stdout) and the resulting D3/D5
answer. Delete the temporary handler afterwards.

---

## Spike 2 — Bun compile against the real dependency set 🔸 OPTIONAL

**Time box: 3 days.** Answers D2.

### Hypothesis

`bun build --compile` produces a single binary that runs the engine's dependency set with no
`node_modules` present.

### Method

Write a throwaway entry point that exercises **every risky dependency**, not a hello-world:

```ts
// spike/bun-entry.ts
import { WebSocketServer } from "ws";
import simpleGit from "simple-git";
import archiver from "archiver";
import unzipper from "unzipper";
import { createCA, createCert } from "mkcert";
import * as fs from "fs";
import * as os from "os";

const dir = fs.mkdtempSync(os.tmpdir() + "/bunspike-");

// 1. ws — server + client round trip
// 2. mkcert — createCA + createCert (pure JS, must not touch node_modules)
// 3. archiver → unzipper round trip through a real file
// 4. simple-git — requires `git` on PATH; assert it is found, not bundled
// 5. fs/path/os/crypto/child_process — baseline Node compat
```

```bash
bun build spike/bun-entry.ts --compile --outfile spike/bun-engine
```

Then run it in a directory with **no `node_modules`** and on a machine without Bun installed.

### Success criteria

- Binary runs with `node_modules` absent.
- `createCA`/`createCert` produce a valid PEM pair (this is the one that would break if any dep resolved
  assets relative to `node_modules`).
- `archiver` → `unzipper` round-trips a real file.
- `ws` server accepts a connection and echoes.
- Startup time and binary size recorded.

### Fallback ladder

1. Bun compile passes → **D2 = (a)**.
2. Fails on one dep → try shimming that dep; if it still fails, **D2 = (b)** Node SEA.
3. SEA fails → **D2 = (c)**, ship `node` + bundled JS. Costs ~45–60 MB, zero compat risk. **This is a
   legitimate outcome, not a failure.**

### Deliverable

`plan/spike-results.md` entry: binary size, cold-start time, which deps passed/failed.

---

## Spike 3 — WebKitGTK CSS compatibility ⏸ SKIPPED

**Time box: 2 days.** Answers D4. This is the cheapest spike and the most likely to change your mind.

### Hypothesis

The existing design system renders correctly in WebKitGTK on the oldest distro you intend to support.

### Why it matters

`oklch()` appears in **7 renderer files** and `color-mix()` in **2** — including
`renderer/tokens.css`, `renderer/styles.css` and `renderer/lib/codemirrorTheme.ts`. If WebKitGTK
doesn't resolve them, **the entire visual design breaks on Linux** — silently, as wrong colours, not as
an error.

### Method

1. Build a minimal test page that imports the **real** token and style files plus a real CodeMirror
   editor instance:
   ```html
   <link rel="stylesheet" href="renderer/tokens.css">
   <link rel="stylesheet" href="renderer/styles.css">
   <div id="probe" style="background: var(--card); border: 1px solid var(--border)">probe</div>
   <div id="mix" style="background: color-mix(in oklab, var(--signal) 22%, transparent)">mix</div>
   ```
2. Load it in WebKitGTK on each target distro. The quickest harness is a minimal Tauri v2 app, or
   `webkit2gtk`'s `MiniBrowser` if available.
3. Read back the **computed** values:
   ```js
   getComputedStyle(document.getElementById('probe')).backgroundColor   // expect a real rgb/oklch
   getComputedStyle(document.getElementById('mix')).backgroundColor     // expect a resolved colour, not transparent
   ```
4. Also check the titlebar: `renderer/components/layout/TitleBar.tsx` uses `oklch`/`color-mix` and is a
   visible, high-traffic surface.

### Test matrix

| Distro | WebKitGTK | Expected |
|---|---|---|
| Ubuntu 22.04 LTS | 2.36 | likely **fails** `color-mix()` |
| Ubuntu 24.04 LTS | 2.44 | likely passes |
| Debian stable (current) | varies | check |
| Fedora current | recent | likely passes |

Record the exact `webkit2gtk` version per distro.

### Outcomes

| Result | Consequence |
|---|---|
| Passes everywhere you care about | D4 = (a) Tauri everywhere |
| Fails on older LTS only | Either drop those distros, or ship CSS fallbacks (`@supports` + hex fallbacks for the ~9 files), or D4 = (b) |
| Fails broadly | D4 = (b) — Tauri on Windows/macOS, Electron on Linux. Legitimate and affordable, because the shell is interchangeable. |

**Note:** if you choose fallbacks, the cost lands in P11 (1–2 weeks) — budget it there, not here.

### Deliverable

`plan/spike-results.md` entry with the version matrix and screenshots.

---

## Spike 4 — Protocol proof of concept

**Time box: 1 week.** Validates the P1 approach before committing to it.

### Hypothesis

The existing `{id, action, payload}` → `{id, ok, data, error}` shape generalises from 4 allowed actions
to the full command surface, with schemas and a typed client.

### Method — generalise what already ships

`src/companion/companionServer.ts` is already a working loopback JSON-RPC server with an allowlist
(`src/companion/allowedActions.ts`) and a **released external consumer** — the companion extension, now in its own repository. Extend
it rather than writing a new server.

1. Pick **10 representative commands** spanning the real complexity:
   `config:get`, `mock:add`, `mock:delete`, `folder:add`, `folder:move`,
   `graphql:introspect`, `soap:execute`, `tls:generate`, `server:status`, `entity:setEnabled`.
   Include at least one **mutating**, one **network-calling**, one **long-running**, and one
   **error-producing** command.
2. Define them with schemas (Zod or JSON Schema — pick now, it informs P1).
3. Generate TypeScript types from the schemas.
4. Write a typed client that consumes them.
5. Wire the client into the renderer behind the existing `window.api` names for those 10 methods only.
   **The renderer must not need editing** — that is the whole thesis, and this is where you find out.

### What this spike is really testing

- Does the action namespace scale, or does it need restructuring?
- Do the error shapes work for real failures (`graphql:introspect` against a dead endpoint)?
- Do long-running commands need progress events? (`soap:execute`, `grpc:execute`, `runner:*`)
- Does a schema-first approach survive contact with `any`-typed payloads like `runner:saveReport`?
- **Does the renderer really need zero changes?** Confirm it with 10 real methods before promising it for
  85.

### Deliverable

Throwaway branch. Write up findings in `plan/spike-results.md`, especially any place the renderer
*did* need editing — that is the most valuable output of this spike.

---

## Phase 0 exit gate

All of the following must be true before P1 begins:

- [x] **Spike 4 complete** — schemas and codegen approach chosen; **renderer-unchanged thesis confirmed
      or refuted**. Confirmed — see `plan/spike-results.md`.
- [x] `plan/spike-results.md` written, with spikes 1–3 marked skipped/optional and the reason recorded.
- [x] Temporary spike code removed from `main`. *(Scope note: this session targets `standalone-engine`,
      not `main`/`master`; `spike/protocol/` is retained there, clearly marked throwaway, as the record
      of this de-risking work. It is superseded by `packages/protocol` in P1 and is not imported by any
      production code.)*
- [x] `00-decisions.md` decision log updated — ✅ done 2026-09-14.

Optional, blocking nothing:

- [ ] Spike 2 (Bun compile) — only if you want a single-binary CLI or intend to resume P10. Records D2.
- [ ] Spikes 1 and 3 — parked under D5. Run only if the shell migration is revisited.

**If spike 4 refutes the renderer-unchanged thesis, stop and revisit `00-decisions.md` before
continuing.** That is the correct outcome of a de-risk phase — it is far cheaper to discover now than
inside P5.

---

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Spike 1's path-virtualisation behaviour is undocumented and surprising | High | Test item 3 explicitly; never assume the engine's data dir is at a literal path in MSIX |
| Spike 2 succeeds on one dep and fails on another, wasting days | Medium | Test all deps in one entry point, in one run |
| Spike 3 gives different answers per distro, no clean call | Medium | The distro matrix is the deliverable — decide on it, don't chase a universal yes |
| Spike 4 tempts you to write production code | **High** | Hard rule: throwaway branch, deleted at phase end |
| Spikes run past their time boxes | Medium | A blown time box is itself a finding. Escalate the decision; do not extend |
