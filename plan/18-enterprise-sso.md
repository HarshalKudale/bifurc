# P18 — Enterprise: optional SSO, org workspaces, permission-gated actions

**Opened 2026-09-26.** Supersedes the pre-Docker-removal version of the implementation brief. The brief's
own scope statement is the authority for *what*; this document is the authority for *how*, and for what
the code already contains that the brief did not know about.

> **Read [`00-decisions.md`](00-decisions.md) D19–D21 first.** All three were put to the owner on
> 2026-09-26 and answered; this document does not re-open them.

---

## 1. Scope, and what is already cut

| | |
| --- | --- |
| **One target** | the **Electron desktop app**. The Docker image and the browser host are gone (2026-09-26) — no Docker publishing, no browser-served UI. |
| **Packages stay intact** | `@bifurc/engine` (still runnable standalone — the test suite depends on it), `@bifurc/gui` (imported by the Electron app, and later by the Chrome extension, so it stays Electron-API-free behind an adapter), `@bifurc/protocol`, `@bifurc/client`, `@bifurc/web-client`. |
| **New, private** | `@bifurc/enterprise` — SSO, login, assignment fetch, provisioning/teardown, permission-JSON handling. **Consumed only by the Electron app** (D20). |
| **Backend** | thin, and **a mock contract until a real backend exists** — SSO + workspace-assignment API + an admin page for users/workspaces/assignments. **No device fleet, no enrollment codes, no device certs.** |

**What "never pushed to the public repo" means concretely:** the enterprise package lives in its **own
sibling repository** (`../bifurc-enterprise`), not in a gitignored directory of this one (D20). The only
enterprise-visible thing in the public repo is the **permission-context plumbing** — the generic types and
the gate — which contains no SSO, no endpoints and no credentials.

---

## 2. The finding that shapes this phase: there are already two authorization axes, and this adds a third

The brief reads as if authorization does not yet exist. **It does**, at the session level, and it is
load-bearing. `packages/engine/src/transport/auth/scopes.ts` holds a **total** command → scope table
(`SCOPE_BY_COMMAND`, 93 explicit lines) that `assertScopesAreTotal()` verifies **at import time** against
`@bifurc/protocol`'s `COMMANDS`; `requiredScopeFor()` returns `undefined` for an unknown command and
`commandIsAllowed()` therefore **denies**. The vocabulary is `read | write | execute | admin`, with **no
hierarchy** — `admin` does not imply `read` — and three presets (`SHELL_SCOPES`, `COMPANION_SCOPES`,
`READ_ONLY_SCOPES`). It is enforced by the `authenticated` transport decorator.

That axis answers **"who is calling?"**. The axis this phase adds answers **"which workspace is being
changed?"** They are different questions with different lifetimes, and **conflating them is the main risk
of this phase**:

| | Session scope (exists today) | Workspace permission (this phase) |
| --- | --- | --- |
| Question | who is calling? | which workspace is being changed? |
| Lifetime | one connection | one workspace, for as long as it is registered |
| Source | the `hello` handshake | the workspace record |
| Vocabulary | `read` / `write` / `execute` / `admin` | the six permissions in §4 |
| Standalone engine | session-less ⇒ full authority | the local actor ⇒ allow-all |
| Failure code | `FORBIDDEN` | **new** — must be distinguishable, see §4.4 |

**A single merged table would make "the org revoked this user's push right" indistinguishable from "this
session may not call `sync.push`"** — different causes, different fixes, one message. Keep them separate.

**And there is a third, older axis worth naming so it is not mistaken for either:** `setDataRootOverride()`
in `store/workspaceFs.ts` and `setDataRoot()` in `store/paths.ts` are *test/lifecycle* hooks, not
authorization. §5 is where they matter.

---

## 3. Decisions

| ID | Decision | Where the reasoning lives |
| --- | --- | --- |
| **D19** | **Push the 128 unpublished commits before starting.** The public repo was 8 days and 494 files behind; it was brought current while still clean of enterprise code | `00-decisions.md` D19 |
| **D20** | **The enterprise package is a sibling repository** (`../bifurc-enterprise`), consumed as a `file:` dependency — not an in-repo gitignored directory | `00-decisions.md` D20 |
| **D21** | **Permissions gate remote-affecting operations and visibility only.** Local CRUD on an org workspace is never gated | `00-decisions.md` D21 |

---

## 4. The permission model (D21)

### 4.1 Vocabulary

Six permissions, in two classes. **Nothing else is gated** — that is the point of D21.

| Permission | Class | Gates |
| --- | --- | --- |
| `org.remote.push` | remote-affecting | `sync.push`, `git.sync`, `sync.setAutoSync` |
| `org.remote.configure` | remote-affecting | `sync.setRemote`, `sync.disconnect` |
| `org.remote.publish` | remote-affecting | `entity.publish`, `folder.publish` |
| `org.remote.pull` | remote-affecting ⚠ | `sync.pull` |
| `org.audit.view` | visibility | `audit.list`, `audit.diff`, `audit.export` |
| `org.history.view` | visibility | `history.list`, `history.diff`, `git.history`, `git.diff` |

⚠ **Two entries are judgement calls, flagged rather than silently decided:**

- **`org.remote.pull` does not change the remote**, so it fails D21's literal test — but it rewrites the
  local tree *from* the remote, which is the same trust boundary and the same shared history. Included;
  the owner may strike it.
- **`git.discard` is deliberately NOT gated.** It destroys local work, but it is purely local, and D21
  says local is free. It is called out here because it is the one destructive command that reads like it
  should be gated and is not.

**`sync.setAutoSync` sits under `push`, not `configure`** — enabling auto-sync is an unattended push, so
granting it without `push` would be a way around `push`.

### 4.2 The context object

```ts
export type WorkspaceKind = "local" | "org";

export interface PermissionContext {
  actor: "local" | "org";
  granted: ReadonlySet<WorkspacePermission>;
}
```

**The local actor short-circuits, and that is what preserves today's behaviour.** `actor === "local"` ⇒
allow, without consulting `granted`. Standalone engine usage (the whole test suite, the headless host)
constructs the local context, so **every existing suite keeps its current result** — which is the property
that makes this phase reviewable.

**The org actor fails closed.** `actor === "org"` and the permission is absent ⇒ deny. Same reasoning as
`commandIsAllowed()`'s: unknown means denied, never allowed.

### 4.3 Where it is enforced

**In the packages, at one choke point** — `commandRegistry.invoke()` — via a gate installed by the
composition root, mirroring how `assertScopesAreTotal()` is a module-scope assertion. The registry stays
ignorant of *what* a permission is: it asks an injected predicate. This keeps the dependency direction
(`transport/` imports `commands/`, never the reverse) intact.

**The workspace is resolved by ONE named function**, `permissionSubject(params)` = `params.workspaceId ??
activeWorkspaceId`. It must be one function and not an inline expression at each site, because the failure
mode is silent and already recorded: **`TRAPS.md` — "A proxy reads one workspace's rules and then filters
them by a *different* one."** A gate that resolves a different workspace than the handler does is the same
defect with a new subject.

### 4.4 The failure must be distinguishable

`FORBIDDEN` already means "this session lacks the scope". Reusing it for a revoked org permission would
make the two indistinguishable to a user reading the error and to any test asserting on it. **A distinct
code is required** — the shape `EngineError` already supports, and the same reasoning the `authenticated`
decorator's own docblock gives for not sharing one code across its three failure modes.

---

## 5. On-disk model: two roots, and the trap that makes this dangerous

**Org repos live in a dedicated location, separate from local workspaces** — under the Electron
`userData` dir, e.g. `<userData>/org/`, against the local root `<userData>/data/`. The engine
**auto-picks them up**, so the workspace registry spans both roots and every entry is tagged with its
`kind`.

### ⚠ The hazard is not the new root — it is the resolvers that will silently ignore it

`TRAPS.md` records three separate victims of exactly this, and the third produced a **false diagnosis**:

> **"`setDataRoot()` redirects *some* roots and not others — and only on Windows."** `dataDir()` honours
> `setDataRoot()` on both platforms; `settingsPath()` → `appDataDir()` honours it **only off Windows** — on
> Windows it reads `%LOCALAPPDATA%\Bifurc\app.json`, ignoring the override entirely. The symptom is
> **silence, not an error** ("serving 0 workspace(s)", exit code 0), it is **invisible to Linux CI**, and
> **the failure is always attributed to the thing you were working on.**

**So WI 2 does not begin by adding a root. It begins by enumerating every resolver** — `grep` for every
function that builds a path from an env var or a root override — and recording, for each, whether it
consults the new org root. A redirect that covers three of four resolvers is worse than none, because the
failure is partial and looks like data loss.

**The isolation claim must be a proof, not an assertion.** "Org wipe can never touch local data" is
provable by a guard asserting the two roots are not ancestors of one another (in either direction) before
any recursive delete — and that guard is the thing to write first, not last.

---

## 6. Teardown

| Trigger | Effect on org root | Effect on local workspaces |
| --- | --- | --- |
| Logout | **wiped** | never touched |
| Login | wiped, then re-provisioned | never touched |
| Token expiry | **wiped** — expiry is treated as logout for org data | never touched |

**Wipe, not orphan.** The brief's word is "wipes"; a swept-aside directory that is never reclaimed is a
different product decision. If a soft delete is wanted later, it is a change to this table.

---

## 7. The enterprise package (D20)

`../bifurc-enterprise` — its own git repo, **private**, consumed by the Electron app as a `file:`
dependency. Package name `@bifurc/enterprise`.

| Exports | Responsibility |
| --- | --- |
| session | SSO login, logout, session state, refresh-on-restart |
| token custody | **Electron `safeStorage`** — the engine never sees a token |
| assignment client | the authenticated workspace-assignment call, returning each workspace's repo URL |
| provisioning | clone/set-up into the org root |
| teardown | the wipe in §6 |
| permission mapping | the assignment API's permission JSON → the public `PermissionContext` |

**It may use Electron APIs** (safeStorage) — unlike `@bifurc/gui`, which must not. It is consumed by the
Electron app and nothing else.

**The dependency is one-way and never reversed:** the public packages must not import it. The Electron
shell imports both and injects the provider. **A grep asserting the public packages never name
`@bifurc/enterprise` is the guard** — the same shape as the engine's existing "zero Electron imports"
check, and it is what makes the privacy claim checkable rather than promised.

⚠ **`package-lock.json` is a coupling here.** `TRAPS.md`: "`npm ci` validates the workspace set against
the lockfile — and it fails in **both** directions." Adding a `file:` dependency changes the lockfile and
adds a **fresh-clone and CI** failure mode that no local run reproduces. Verify with `npm ci --dry-run`
in a temp context, never by reasoning.

---

## 8. The mock backend

**A mock contract, explicitly — until a real backend exists.** Three surfaces:

1. **SSO** — a token endpoint. Not real SSO; the shape is the point.
2. **Assignment API** — authenticated; returns the workspaces assigned to the caller, each with its repo
   URL.
3. **Admin page** — users, workspaces, assignments.

⚠ **The trap this section exists to avoid:** `TRAPS.md` — *"An optional param that is **accepted and
ignored** reads as implemented."* A permission JSON that is fetched, parsed and then never consulted looks
exactly like a working feature from the outside. **The mock must produce a permission set that actually
differs between users**, and at least one WI 3 case must assert a *denial* driven by the mock's response —
otherwise the whole feature is unverified.

---

## 9. Work items

| # | Work item | Deliverable | Depends on |
| --- | --- | --- | --- |
| **0** | Opening measurement and this document | baseline re-measured; D19–D21 recorded | — |
| **1** | **Permission-context plumbing (public)** | `WorkspaceKind`, `PermissionContext`, the local/allow-all default, the totality assertion. **No enterprise code.** Every existing suite byte-identical | WI 0 |
| **2** | **The org root and the isolation proof** | a second workspace root; **the resolver enumeration of §5 first**; the ancestor guard | WI 1 |
| **3** | **The gate** | the six-permission table, the `invoke()` gate, `permissionSubject()`, the distinct failure code | WI 1, WI 2 |
| **4** | **The private enterprise package** | `../bifurc-enterprise` scaffold: session, safeStorage, assignment, provision, teardown, permission mapping | WI 1, D20 |
| **5** | **The Electron shell** | login/logout, provisioning on login, wipe on logout and expiry, `kind` reaching the engine | WI 2, WI 3, WI 4 |
| **6** | **The mock backend and admin page** | SSO + assignment + admin, with a permission set that actually varies | WI 4 |
| **7** | **The GUI** | the Org badge; surfaces hidden when their permission is absent | WI 3, WI 5 |

**Ordering note.** WI 1 and WI 2 are **public, reviewable and independently valuable** — they land before
any enterprise code exists, and they are the half the brief calls "the permission-context plumbing in the
public packages". WI 4 onward is private and does not appear in the public repo's history.

---

## 10. Acceptance criteria

1. Logged out, the app behaves **exactly as today** — local mode, every action available.
2. Login provisions assigned workspaces into the **org root**, and they render with an Org badge.
3. **The engine never receives a token.** Asserted, not assumed.
4. Logout and token expiry **both** wipe the org root.
5. **Local workspaces survive every teardown path.** The ancestor guard proves it; a test exercises it.
6. On an org workspace, **local CRUD is unrestricted** (D21).
7. On an org workspace, a **denied remote-affecting command is refused**, with a code distinguishable from
   `FORBIDDEN`.
8. **No audit-log permission ⇒ the audit log is not visible**, and `audit.list` is refused.
9. The **standalone engine** — the full test suite, the headless host — runs as the local actor and is
   **unchanged**.
10. **No public package imports `@bifurc/enterprise`** — checked by a grep guard.
11. `npm ci --dry-run` passes in a fresh context with the `file:` dependency present.

---

## 11. Traps this phase must respect

Carried from [`../.workbuddy-ai/memory/TRAPS.md`](../.workbuddy-ai/memory/TRAPS.md), with the section
that matters:

| Trap | Why it bites *here* |
| --- | --- |
| **`setDataRoot()` redirects some roots and not others** | §5. The new org root is a fourth resolver, and the failure is silence. Enumerate before adding. |
| **Namespacing a store surfaces every *half-threaded* id** | The org root is a second namespace. Unit suites stayed green through D17's identical change; **only the integration suite saw the 30 failures**. |
| **A proxy reads one workspace's rules and then filters them by a *different* one** | `permissionSubject()` must be one function. |
| **An optional param that is accepted and ignored reads as implemented** | §8. A permission JSON nothing consults looks like a feature. |
| **`npm ci` validates the workspace set against the lockfile** | §7. The `file:` dependency fails in both directions, and only on a fresh clone. |
| **A guard that can silently match nothing is worse than no guard** | The "public packages never import the enterprise package" grep must be mutation-checked. |
| **A failure you cannot attribute is a question, not a finding** | `npm run dev` and `test:e2e` are the owner's to run (standing rule). |
| **A count written into prose is never re-derived** | Every measurement in this document is re-measured at the gate, never carried. |

---

## 12. Status

**WI 0 — this document, and D19–D21 — DONE 2026-09-26.** D19 was executed the same evening: the 128
commits were pushed, `origin/standalone-engine` moved `614bb03..73ef3ad`, and the range was scanned for
credential patterns and for enterprise paths **before** it was published (both clean).

**Nothing else has started.** WI 1 is next.
