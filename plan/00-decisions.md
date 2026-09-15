# 00 — Decisions

**Goal:** close every open decision that gates the programme. These are cheap now and expensive later.

**Effort:** 1–2 days. **Depends on:** nothing. **Blocks:** P0 and everything after it.

Answer each one in the "Decision" column and date it. Do not begin P0 with any of D1–D5 unresolved.

---

## D1 — Ordering: is P6 (seam) the ship milestone, with Tauri deferred?

|                    |                                                                                                                                                                                                                                                                                                      |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Options**        | (a) Seam first, keep Electron, defer Tauri · (b) Tauri first · (c) both concurrently                                                                                                                                                                                                                 |
| **Recommendation** | **(a)**                                                                                                                                                                                                                                                                                              |
| **Why**            | After decoupling the shell is disposable, so the Tauri swap becomes isolated, deferrable and reversible. The RPC client is identical for Electron and Tauri, so waiting costs nothing. (b)/(c) entangle engine extraction + webview compatibility + hand-authored MSIX — three independent unknowns. |
| **Blocks**         | P0 scope, P11 scheduling                                                                                                                                                                                                                                                                             |
| **Decision**       | (a)                                                                                                                                                                                                                                                                                                  |

---

## D2 — Engine runtime: Bun, Node SEA, or ship `node` + JS?

|                    |                                                                                                                                                                                                                                                                                                                              |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Options**        | (a) `bun build --compile` · (b) Node 20+ SEA · (c) ship `node` binary + bundled JS                                                                                                                                                                                                                                           |
| **Recommendation** | **(a), pending the P0 spike.** Fall back to (c) if Bun fights the dependency set.                                                                                                                                                                                                                                            |
| **Why**            | The dep surface is compile-friendly (`mkcert`, `node-forge`, `ws`, `archiver`, `unzipper` are all pure JS; `simple-git` and `certManager` spawn external tools that must exist at runtime anyway). (a) gives the smallest binary and fastest startup; (c) is the most boring and most reliable, costing only installer size. |
| **Verified**       | `mkcert@3.2.0` is pure JS — no bundled native binary. `__dirname`/`process.resourcesPath` appear **only in `main.ts`**, so no engine module resolves assets from disk.                                                                                                                                                       |
| **Blocks**         | P0 spike 2, P10, P9 base image, P8 CLI runtime                                                                                                                                                                                                                                                                               |
| **Decision**       | **(a), pending the P0 spike.** Fall back to (c) if Bun fights the dependency set.                                                                                                                                                                                                                                            |



---

## D3 — Transport for spawned engines: TCP port or socket?

|                    |                                                                                                                                                                                                                |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Options**        | (a) ephemeral TCP port · (b) unix domain socket (macOS/Linux) + named pipe (Windows)                                                                                                                           |
| **Recommendation** | **(b)**, with (a) as a fallback for environments where sockets are unavailable                                                                                                                                 |
| **Why**            | Avoids port collisions entirely, is not reachable from other machines, and **sidesteps the MSIX loopback question**. `cfg.companionPort ?? 9271` is a fixed default today and cannot survive multiple engines. |
| **Blocks**         | P0 spike 1, P4                                                                                                                                                                                                 |
| **Decision**       | **(b)**, with (a) as a fallback for environments where sockets are unavailable                                                                                                                                 |

---

## D4 — Linux shell strategy if WebKitGTK fails the CSS check

|                    |                                                                                                                                                              |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Options**        | (a) Tauri everywhere · (b) Tauri on Windows/macOS, keep Electron on Linux · (c) drop Tauri entirely                                                          |
| **Recommendation** | Decide after P0 spike 3. **(b) is a legitimate split** — the shell is interchangeable, so per-platform divergence is affordable.                             |
| **Why**            | `oklch()` appears in 7 renderer files and `color-mix()` in 2. WebView2 (Chromium) and WKWebView are fine; WebKitGTK support depends on the distro's version. |
| **Blocks**         | P11 scope, P12                                                                                                                                               |
| **Decision**       | keep it electron right now we will see about the shell migration later. Decoupling is the focus right now.                                                   |

---

## D5 — MSIX ownership once Tauri's bundler stops producing it

|                    |                                                                                                                                                                                                                                                         |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Options**        | (a) keep electron-builder for the Store build only · (b) hand-author MSIX from a Tauri layout · (c) drop the Store                                                                                                                                      |
| **Recommendation** | **(a) during the transition**, then (b) if/when P11 lands                                                                                                                                                                                               |
| **Why**            | Tauri v2's Windows bundler emits **NSIS + MSI (WiX)** only — there is no `appx`/`msix` target. Today electron-builder produces your Store artifact for free. Keeping Electron as the Store-signed shell until P11 is a valid, low-cost holding pattern. |
| **Blocks**         | P12                                                                                                                                                                                                                                                     |
| **Decision**       | Tauri is deffered we keep electron for now until decoupled engine matures                                                                                                                                                                               |

---

## D6 — Where does `Cleanup_plan.md` fit?

|                    |                                                                                                                                                                                                                                                                                                                             |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Options**        | (a) cleanup first, then decoupling · (b) decoupling first · (c) interleave                                                                                                                                                                                                                                                  |
| **Recommendation** | **(a)**, and treat it as a hard prerequisite to P2                                                                                                                                                                                                                                                                          |
| **Why**            | `Cleanup_plan.md` already touches ~110 files across 5 phases. Running it concurrently with engine extraction makes regressions untraceable. There is genuine overlap — splitting `handlers.ts` (1,909 lines) and extracting `entityCrudFactory` both help P2 — so cleanup → decoupling is strictly better than the reverse. |
| **Caveat**         | If cleanup is large, consider doing only its Phase 1–2 (dead code + shared utils) before P2 and deferring Phase 4–5.                                                                                                                                                                                                        |
| **Blocks**         | P2 start date                                                                                                                                                                                                                                                                                                               |
| **Decision**       | **(a)**, and treat it as a hard prerequisite to P2                                                                                                                                                                                                                                                                          |

---

## D7 — CLI scope

|                    |                                                                                                                                                                                                                                                                 |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Options**        | (a) engine lifecycle + read-only inspection (~3 w) · (b) full CRUD parity across all protocols (~5 w+)                                                                                                                                                          |
| **Recommendation** | **(a) for v1**, then expand based on real usage                                                                                                                                                                                                                 |
| **Why**            | This is the single largest swing in the estimate. A CLI that starts/stops the engine, inspects config, and tails logs is immediately useful and validates the protocol. Full parity across REST/GraphQL/gRPC/SOAP is a much larger surface with unknown demand. |
| **Blocks**         | P8 scope                                                                                                                                                                                                                                                        |
| **Decision**       | **(a) for v1**, then expand based on real usage                                                                                                                                                                                                                 |

---

## D8 — Web UI parity target

|                    |                                                                                                                                                                                                                                                                                 |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Options**        | (a) full parity with Electron · (b) inspection + light editing                                                                                                                                                                                                                  |
| **Recommendation** | **(b) initially.** Ship it, measure what people actually do in a browser, then close gaps.                                                                                                                                                                                      |
| **Why**            | The renderer is reusable, so parity is mostly free — but the *dialogs, auth and multi-engine* UX is new work whose scope depends entirely on how far you push. The CodeMirror editor and capture tooling behave differently at browser scale (large HAR files, streaming logs). |
| **Blocks**         | P7 scope                                                                                                                                                                                                                                                                        |
| **Decision**       | **(b) initially.** Ship it, measure what people actually do in a browser, then close gaps.                                                                                                                                                                                      |

---

## D9 — Versioning policy

|                    |                                                                                                                                                                                                                                                                                                                       |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Options**        | (a) lockstep — shell, engine, protocol, web UI share one version · (b) independent versions with a compatibility matrix                                                                                                                                                                                               |
| **Recommendation** | **(a) until a remote-engine user base actually exists**                                                                                                                                                                                                                                                               |
| **Why**            | Independent versioning requires a protocol compatibility matrix, a deprecation policy, and cross-version testing — ongoing cost with no benefit while the engine ships inside the shell. Note `app:checkUpdate` currently uses `app.getVersion()` + `process.platform`; under lockstep it has an unambiguous meaning. |
| **Blocks**         | P1 version handshake design, P12                                                                                                                                                                                                                                                                                      |
| **Decision**       | **(a) until a remote-engine user base actually exists**                                                                                                                                                                                                                                                               |

---

## Decision log

Append a row each time a decision is made or revisited.

| Date | ID | Decision | Notes |
| ---- | -- | -------- | ----- |
| 2026-09-14 | D1 | (a) Seam first; Tauri deferred | P6 is the ship milestone |
| 2026-09-14 | D2 | (a) Bun compile, pending P0 spike 2; fall back to (c) `node` + JS | Revisit at the P0 gate |
| 2026-09-14 | D3 | (b) Unix socket / named pipe, TCP fallback | Simplifies P0 spike 1; sets the P4 target |
| 2026-09-14 | D4 | **Keep Electron.** Shell migration revisited later; decoupling is the focus | Parks P11 |
| 2026-09-14 | D5 | **Tauri deferred.** Electron stays until the decoupled engine matures | Parks P11; MSIX stays with electron-builder |
| 2026-09-14 | D6 | (a) `Cleanup_plan.md` first — hard prerequisite to P2 | P2 cannot start until cleanup lands |
| 2026-09-14 | D7 | (a) CLI v1 = engine lifecycle + read-only inspection | Narrows P8 to ~3 w |
| 2026-09-14 | D8 | (b) Web UI v1 = inspection + light editing | Sets P7 scope |
| 2026-09-14 | D9 | (a) Lockstep versioning | See the extension exception below |

---

## Consequences of the resolved decisions

### D1 + D4 + D5 park Track C

P10 (standalone binary) and P11 (Tauri shell) are **out of scope** until the shell migration is
revisited. This removes roughly **6–8 engineer-weeks** from the programme and, more importantly, takes
**two of the three highest-risk items off the critical path**:

| Risk | Status now |
|---|---|
| MSIX cannot spawn a supervised engine | **Removed** — Electron stays, and electron-builder already produces a working APPX |
| WebKitGTK `oklch()` / `color-mix()` failures | **Removed** — Electron ships Chromium |
| Bun compile against the real dependency set | **Downgraded** — still worth spiking, but nothing depends on it |

**P0's scope changes accordingly.** Spike 1 (MSIX) and spike 3 (WebKitGTK) are **no longer required** —
they existed to de-risk a Tauri shell that is now parked. Spike 2 (Bun compile) matters only if you want
a single-binary CLI or intend to resume P10. **Spike 4 (protocol proof of concept) is the only one that
is still mandatory**, because it validates the approach P1 is built on.

Keep spikes 1 and 3 documented but skipped. If D4/D5 are ever revisited, they are the first things to
run — and their notes are already written.

### D6 makes P2 gated on cleanup

P2 does not start until `Cleanup_plan.md` lands. If cleanup is large, doing only its Phase 1–2 (dead code
+ shared utils) is enough to unblock P2 — Phase 4–5 (file splitting, anti-patterns) can follow later.
Both `handlers.ts` splitting and `entityCrudFactory` extraction come out of cleanup and directly reduce
P2's work, which is why the order is cleanup → decoupling rather than the reverse.

### D7 narrows P8

CLI v1 is **engine lifecycle + read-only inspection** only:

```
In scope:  engine start|stop|status|logs, remote add|list|use|test,
           config get, workspace/env list, mocks|requests|rules|mappings list,
           tls generate|status|export, logs tail, audit list
Deferred:  every mutating command — add/rm/update, import, tls trust, requests send
```

Roughly **3 weeks** instead of 5+. The deferred set is not wasted work — it is the same client calls,
added later against a proven protocol.

### D8 sets P7's target

Web UI v1 is **inspection + light editing**. Full parity is explicitly deferred until real usage shows
where the gaps actually are.

### D9 has one exception now

Engine, shell and web UI are lockstep. **The companion extension is not** — it lives in its own
repository with its own release cycle. Its four commands (`mock:add`, `request:add`, `folder:add`,
`config:get`) must stay backward-compatible across at least one minor protocol version. See
`README.md` → "Protocol compatibility".

---

## Repo changes (2026-09-14)

The repository was flattened and the extension extracted. This affects every path in the plan:

| Was | Now |
|---|---|
| `bifurc/` (app subdirectory) | repository root |
| `bifurc/package.json` (`bifurc-monorepo`, npm workspaces) | `package.json` (`bifurc`, single package) |
| `bifurc-extension/` | separate repository (`../bifurc-extension`) |
| `.github/workflows/*.yml` with `working-directory: bifurc` | `working-directory: .` |

`npm workspaces` is gone from the root manifest. P2 item 8 still introduces `packages/*` workspaces — it
now adds them to a single-package repo rather than restructuring an existing monorepo, which is slightly
less work than the phase document originally assumed.
