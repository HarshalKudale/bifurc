/**
 * `createClient` — the acceptance criteria that need a live client.
 *
 * The headline case is the key diff: `Object.keys(client)` must equal `Object.keys(window.api)`
 * exactly. That is the plan's "a script asserting `Object.keys(oldApi)` vs `Object.keys(newApi)`
 * reports zero differences", written as a test rather than a script so it runs inside the gate
 * instead of being remembered.
 *
 * The preload is imported **for real** (with `electron` mocked) rather than read from the type,
 * because `packages/gui/src/types/window.ts` under-declares the runtime by four keys — a type-driven diff
 * would pass while the client was missing them. See `src/surface.ts`'s header.
 */

import { describe, expect, it, vi } from "vitest";
import { EngineError, ErrorCode } from "@bifurc/protocol";
import type { Transport, EngineEvent } from "@bifurc/engine/transport/types";
import { createClient } from "../src/index";
import type { ClientLocal, LocalFileContent, ReadFileOptions, SavePathOptions } from "../src/local";

const captured = vi.hoisted(() => ({ api: undefined as Record<string, unknown> | undefined }));

vi.mock("electron", () => ({
  contextBridge: {
    exposeInMainWorld: (_key: string, value: Record<string, unknown>) => {
      captured.api = value;
    },
  },
  ipcRenderer: { invoke: () => Promise.resolve(undefined), on: () => {}, off: () => {} },
}));

// ── a counting fake transport ───────────────────────────────────────────────

interface FakeTransport extends Transport {
  readonly calls: { cmd: string; payload: unknown }[];
  readonly subscriptions: string[][];
  readonly closed: boolean[];
  emit(event: string, payload: unknown): void;
  listenerCount(): number;
}

function fakeTransport(): FakeTransport {
  const calls: { cmd: string; payload: unknown }[] = [];
  const subscriptions: string[][] = [];
  const closed: boolean[] = [];
  const listeners = new Set<(e: EngineEvent) => void>();

  return {
    kind: "in-process",
    calls,
    subscriptions,
    closed,
    request: (cmd, payload) => {
      calls.push({ cmd, payload });
      return Promise.resolve({ ok: true, echoed: cmd });
    },
    subscribe: (events, cb) => {
      subscriptions.push(events);
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    close: () => {
      closed.push(true);
      return Promise.resolve();
    },
    emit: (event, payload) => {
      for (const l of [...listeners]) l({ event: event as EngineEvent["event"], seq: 1, payload });
    },
    listenerCount: () => listeners.size,
  };
}

/** A client-local stub that records what it was asked to do. */
function fakeLocal() {
  const seen: string[] = [];
  const local: ClientLocal = {
    // `checkUpdate` is `local` as of 2026-09-20 — the surface's only `SPLIT` resolved to the client
    // rather than to an engine command, so it is a member here like the other thirteen.
    checkUpdate: async () => {
      seen.push("checkUpdate");
      return {
        ok: true,
        hasUpdate: false,
        currentVersion: "0.0.0",
        latestVersion: "0.0.0",
        downloadUrl: "https://example.invalid/releases",
        releaseUrl: "https://example.invalid/releases",
      };
    },
    openExternal: async (url) => void seen.push(`openExternal:${url}`),
    setTitleBarOverlay: async () => {
      seen.push("setTitleBarOverlay");
      return { ok: true };
    },
    getTheme: async () => {
      seen.push("getTheme");
      return "dark";
    },
    setTheme: async (t) => {
      seen.push(`setTheme:${t}`);
      return { ok: true };
    },
    tlsInstallCA: async () => {
      seen.push("tlsInstallCA");
      return { ok: true };
    },
    // `local` as of 2026-09-20, alongside `tlsInstallCA` — both compose the engine's half with the
    // host's OS trust store, so neither can be answered by routing (`plan/07` §5b-bis). Returning the
    // full eight-field shape here rather than a stub is deliberate: the routed form's whole defect was
    // that three of these arrived `undefined`, and a fixture that omitted them would reproduce the bug
    // while looking satisfied.
    tlsRemoveCert: async () => {
      seen.push("tlsRemoveCert");
      return { ok: true, engineRemoved: true, clientUntrusted: true };
    },
    tlsCertStatus: async () => {
      seen.push("tlsCertStatus");
      return {
        generated: false,
        certPath: null,
        keyPath: null,
        fingerprint: null,
        trustedFingerprint: null,
        trustState: "none",
        firefoxDetected: false,
        firefoxProfiles: 0,
      };
    },
    openFileDialog: async () => {
      seen.push("openFileDialog");
      return null;
    },
    pickFilePath: async () => {
      seen.push("pickFilePath");
      return null;
    },
    pickFolderPath: async () => {
      seen.push("pickFolderPath");
      return null;
    },
    platform: "win32",
    isFirstLaunch: async () => {
      seen.push("isFirstLaunch");
      return true;
    },
    completeFirstLaunch: async () => {
      seen.push("completeFirstLaunch");
      return { ok: true };
    },
    getZoomLevel: async () => {
      seen.push("getZoomLevel");
      return 0;
    },
    setZoomLevel: async () => {
      seen.push("setZoomLevel");
      return { ok: true };
    },
  };
  return { local, seen };
}

async function realPreloadKeys(): Promise<string[]> {
  if (!captured.api) await import("../../../src/preload");
  if (!captured.api) throw new Error("preload did not call exposeInMainWorld");
  return Object.keys(captured.api);
}

// ── the key diff ────────────────────────────────────────────────────────────

describe("createClient — the surface is byte-identical", () => {
  it("exposes exactly the keys src/preload.ts exposes, and no others", async () => {
    const { local } = fakeLocal();
    const client = createClient(fakeTransport(), { local });

    const mine = Object.keys(client).sort();
    const theirs = (await realPreloadKeys()).sort();

    // Set equality in both directions: a swap (one added, one dropped) must fail.
    expect(mine).toEqual(theirs);
    expect(mine).toHaveLength(144);
  });

  it("keeps close() off the enumerated surface while still being callable", async () => {
    /**
     * The mechanism behind the test above, asserted directly — so a refactor that moves `close` back
     * into the object literal fails *here*, with the reason spelled out, instead of only showing up
     * as an opaque 145-vs-144 length mismatch.
     *
     * `window.api` has no `close`; the shell needs one. Enumerability is the discriminator that lets
     * both be true at once, and it is also what `contextBridge` follows when it copies the object
     * into the renderer — so a non-enumerable `close` cannot leak onto `window.api` in P6.
     *
     * `writable: true` is asserted on purpose: it proves the descriptor passed to `defineProperty`
     * carried *only* `enumerable`, leaving the other attributes as the literal gave them. A
     * hand-rolled full descriptor would show `writable: false` here.
     */
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    expect(Object.keys(client)).not.toContain("close");
    expect(Object.getOwnPropertyDescriptor(client, "close")).toMatchObject({
      enumerable: false,
      writable: true,
    });
    expect(typeof client.close).toBe("function");
  });

  it("keeps the four keys BifurcApi does not declare", async () => {
    /**
     * These are the reason the return type is `BifurcApiFull` rather than `BifurcApi`. Dropping
     * them would still satisfy the declared contract and the type-level shape test — which is
     * precisely the failure mode P5 exists to prevent.
     */
    const { local } = fakeLocal();
    const client = createClient(fakeTransport(), { local });

    const keys = Object.keys(client);
    for (const k of ["isFirstLaunch", "completeFirstLaunch", "getZoomLevel", "setZoomLevel"]) {
      expect(keys).toContain(k);
      expect(typeof (client as unknown as Record<string, unknown>)[k]).toBe("function");
    }
  });
});

// ── routing ─────────────────────────────────────────────────────────────────

describe("createClient — command routing and payloads", () => {
  it("maps a positional window.api call onto the command's object payload", async () => {
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    await client.loadEntity("ws-1", "mocks", "m-9");

    expect(t.calls).toEqual([{ cmd: "entity.load", payload: { workspaceId: "ws-1", kind: "mocks", id: "m-9" } }]);
  });

  it("loadEntity translates the two engine-internal kinds its callers pass — the live bug, 2026-09-21", async () => {
    /**
     * The **`window.api`-fidelity half** of a two-sided translation, and the reason `loadEntity` is a
     * `shim` rather than a `transport` entry.
     *
     * `EntityKind` renames two of the twelve CRUD kinds: the engine stores a proxy rule under `"rules"`
     * and a websocket connection under `"sockets"`, the protocol calls them `"proxyRules"` and
     * `"wsConnections"`. Two panels pass the engine's names —
     * `ProxyRulesPanel.tsx:74,132,165` and `WebSocketsPanel.tsx:146,159,250` — and the deleted
     * `entity:load` channel translated them with `toProtocolKind()` before invoking the command. The
     * client forwarded them verbatim, so the frozen schema rejected them, and both panels'
     * `.catch(() => {})` turned a `BAD_REQUEST` into "the entity has no fields".
     *
     * **This test would have failed before the fix**, which is what makes it worth writing: the
     * pre-existing case above passes `"mocks"`, for which the translation is the identity, so it pinned
     * the mapping's *existence* and not its *content*. Verified against the schema directly —
     * `EntityLoadParams.safeParse({kind: "rules"})` is rejected.
     */
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    await client.loadEntity("ws-1", "rules", "r-1");
    await client.loadEntity("ws-1", "sockets", "w-1");
    await client.loadEntity("ws-1", "mappings", "m-1");

    expect(t.calls).toEqual([
      { cmd: "entity.load", payload: { workspaceId: "ws-1", kind: "proxyRules", id: "r-1" } },
      { cmd: "entity.load", payload: { workspaceId: "ws-1", kind: "wsConnections", id: "w-1" } },
      // The control: the other fourteen kinds must round-trip **unchanged**, so a table that widened
      // the translation would fail here rather than silently renaming a kind the engine stores as-is.
      { cmd: "entity.load", payload: { workspaceId: "ws-1", kind: "mappings", id: "m-1" } },
    ]);
  });

  it("setEntityEnabled resolves `invalid_kind` for an out-of-enum kind, and does not call the engine", async () => {
    /**
     * The other half of the same channel pair, and **latent rather than live** — every caller passes a
     * kind the enum accepts (the mocks panel's `resolveEntityKind` yields only `mocks` / `graphqlMocks` /
     * `grpcMocks` / `soapMocks` in mock mode, and the two toggling panels pass `"mappings"` and
     * `"rules"`). It is carried over because it is a *documented* client-facing behaviour of the surface
     * being reconstructed: `packages/gui/src/types/window.ts:41` declares
     * `Promise<{ ok: boolean; error?: string }>`, and the engine's own header says the guard
     * "is client-facing behaviour and stays where the channel is" (`store/coreCommands.ts:24-28`).
     *
     * The `calls` assertion is the load-bearing half. A version that let the payload through and
     * *also* returned the error would pass a result-only assertion while still sending a payload the
     * schema rejects — the shape of mistake `respondingTransport`'s own note records.
     */
    const { t, calls } = respondingTransport(() => ({ ok: true }));
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    // `"requests"` is a real `EntityKind` but not an enabled-state kind — the boundary the guard exists
    // for. `"sockets"` is rejected by *both* vocabularies, which is why it is not the control here.
    await expect(client.setEntityEnabled("ws-1", "requests", "r-1", true))
      .resolves.toEqual({ ok: false, error: "invalid_kind" });
    expect(calls).toEqual([]);

    // And a valid kind still goes through, so the guard cannot swallow everything.
    await client.setEntityEnabled("ws-1", "rules", "r-1", true);
    expect(calls).toEqual([
      { cmd: "entity.setEnabled", payload: { workspaceId: "ws-1", kind: "rules", id: "r-1", enabled: true } },
    ]);
  });

  it("injects the entity kind for the CRUD collapse", async () => {
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    await client.addMock({ name: "m" } as never);
    await client.deleteRule("r-1");

    expect(t.calls[0]).toEqual({ cmd: "entity.create", payload: { kind: "mocks", entity: { name: "m" } } });
    expect(t.calls[1]).toEqual({ cmd: "entity.delete", payload: { kind: "proxyRules", id: "r-1" } });
  });

  it("unwraps entity.create's {id, entity} envelope, which the surface does not declare", async () => {
    /**
     * `entity.create` answers `{ id, entity }` (`entityCrudCommands.ts:413`); the surface declares the
     * *raw* entity, and the shell channels used to unwrap it. On the routed path nothing did, so all 16
     * `add*` keys handed the panel the envelope — an object with an `id` and no other field, pushed
     * straight into `config.mappings` / `config.proxyRules` / … by the panels. The
     * `as ReturnType<BifurcApi[…] >` cast made the compiler agree with the **declaration** rather than
     * the runtime, so nothing ever failed. The default transport in this file echoes `{ok:true}`, which
     * has no `.entity` to unwrap — which is exactly why the tests above never caught it. This one drives
     * the real shape.
     */
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    t.request = (cmd, payload) => {
      t.calls.push({ cmd, payload });
      return Promise.resolve({ id: "m-1", entity: { id: "m-1", name: "m" } });
    };

    await expect(client.addMock({ name: "m" } as never)).resolves.toEqual({ id: "m-1", name: "m" });
  });

  it("passes the gate-blocked create through unwrapped, because it is not an envelope", async () => {
    /**
     * The one `entity.create` result that is **not** `{id, entity}`: `environments` is the sole kind
     * with a `gateKind`, and a blocked create returns `{error: "limit_reached", ...gate}` with no
     * `.entity` (`entityCrudCommands.ts:410`). The shell passed that through unchanged, and the surface
     * declares it — so the unwrap must leave it alone rather than reduce it to `undefined`.
     */
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    const gate = { error: "limit_reached", allowed: false, limit: 3, current: 3 };
    t.request = (cmd, payload) => {
      t.calls.push({ cmd, payload });
      return Promise.resolve(gate);
    };

    await expect(client.addEnvironment({ name: "prod" } as never)).resolves.toEqual(gate);
  });

  it("renames wsId to workspaceId and passes no workspaceId where the command omits it", async () => {
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    await client.syncPush("ws-7");
    await client.addWorkspace("new");

    expect(t.calls[0]).toEqual({ cmd: "sync.push", payload: { workspaceId: "ws-7" } });
    // `workspace.add` takes only a name — adding a workspaceId would fail the frozen schema.
    expect(t.calls[1]).toEqual({ cmd: "workspace.add", payload: { name: "new" } });
  });

  it("resolves the active workspace for the three listers, which carry no wsId", async () => {
    /**
     * `entity.list`'s `workspaceId` is required, unlike `entity.create`'s. The legacy handlers read
     * `loadConfig().activeWorkspaceId` in the main process; a client has to ask.
     */
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    // The fake transport echoes `{ok:true}`; give config.get a real activeWorkspaceId.
    const original = t.request;
    t.request = (cmd, payload) => {
      if (cmd === "config.get") {
        t.calls.push({ cmd, payload });
        return Promise.resolve({ activeWorkspaceId: "ws-active" });
      }
      if (cmd === "entity.list") {
        t.calls.push({ cmd, payload });
        return Promise.resolve({ entities: [{ id: "a" }] });
      }
      return original(cmd, payload);
    };

    const rows = await client.listWsdls();

    expect(rows).toEqual([{ id: "a" }]);
    expect(t.calls.map((c) => c.cmd)).toEqual(["config.get", "entity.list"]);
    expect(t.calls[1].payload).toEqual({ kind: "wsdls", workspaceId: "ws-active" });
  });

  it("refuses to invent a workspace when the engine reports none", async () => {
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    t.request = () => Promise.resolve({ activeWorkspaceId: undefined });

    await expect(client.listProtoFiles()).rejects.toThrow(/no active workspace/);
  });
});

// ── client-local is never routed ─────────────────────────────────────────────

describe("createClient — client-local methods never touch the transport", () => {
  it("delegates every local method and issues zero transport calls", async () => {
    const t = fakeTransport();
    const { local, seen } = fakeLocal();
    const client = createClient(t, { local });

    await client.openExternal("https://example.com");
    await client.setTitleBarOverlay("#000", "#fff");
    await client.getTheme();
    await client.setTheme("dark");
    await client.tlsInstallCA();
    await client.openFileDialog();
    await client.pickFilePath("t");
    await client.pickFolderPath("t");
    await client.isFirstLaunch();
    await client.completeFirstLaunch();
    await client.getZoomLevel();
    await client.setZoomLevel(2);

    expect(seen).toEqual([
      "openExternal:https://example.com",
      "setTitleBarOverlay",
      "getTheme",
      "setTheme:dark",
      "tlsInstallCA",
      "openFileDialog",
      "pickFilePath",
      "pickFolderPath",
      "isFirstLaunch",
      "completeFirstLaunch",
      "getZoomLevel",
      "setZoomLevel",
    ]);
    // The acceptance criterion, asserted directly.
    expect(t.calls).toEqual([]);
    expect(client.platform).toBe("win32");
  });
});

// ── subscriptions ───────────────────────────────────────────────────────────

describe("createClient — subscriptions", () => {
  it("shares one transport subscription between two listeners", () => {
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    const a = vi.fn();
    const b = vi.fn();
    const offA = client.onSyncStatus(a as never);
    const offB = client.onSyncStatus(b as never);

    expect(t.subscriptions).toEqual([["event.sync.status"]]);

    t.emit("event.sync.status", { wsId: "w" });
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);

    // Removing one must not tear the shared subscription down under the survivor.
    offA();
    t.emit("event.sync.status", { wsId: "w" });
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(2);

    offB();
    expect(t.listenerCount()).toBe(0);
  });

  it("unwraps a coalesced log.entry batch, because the pump reuses the same wire name", () => {
    /**
     * `eventPump.ts`'s `COALESCED_EVENT` is `"event.log.entry"`, so a batch arrives as
     * `{entries:[…]}` on the *same* event. Without the unwrap the renderer would receive one
     * `{entries}` object where it expects an entry, and blank exactly when traffic is heaviest.
     */
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    const cb = vi.fn();
    client.onLogEntry(cb as never);

    t.emit("event.log.entry", { entries: [{ id: 1 }, { id: 2 }] });
    expect(cb).toHaveBeenCalledTimes(2);
    expect(cb).toHaveBeenNthCalledWith(1, { id: 1 });

    // A single (unbatched) entry still arrives as itself.
    t.emit("event.log.entry", { id: 3 });
    expect(cb).toHaveBeenCalledTimes(3);
    expect(cb).toHaveBeenNthCalledWith(3, { id: 3 });
  });

  it("drops the entity.changed payload for onCompanionRefresh, which only wants the signal", () => {
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    const cb = vi.fn();
    client.onCompanionRefresh(cb);

    t.emit("event.entity.changed", { entity: "mocks", id: "m-1" });
    expect(cb).toHaveBeenCalledWith();
    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("closes the hub and the transport together", async () => {
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    client.onLogChunk(vi.fn() as never);
    expect(t.listenerCount()).toBe(1);

    await client.close();
    expect(t.listenerCount()).toBe(0);
    expect(t.closed).toEqual([true]);
  });
});

// ── the retry policy is applied, not merely available ───────────────────────

/**
 * `retry.test.ts` proves the *policy* is correct. These prove it is **wired in** — a policy module
 * that nothing imports passes every test in isolation and retries nothing in production, which is
 * exactly the state this file caught (`index.ts` did not import `retry.ts` at all).
 *
 * The `sleep` is stubbed in every case so the suite does not wait on real backoff.
 */
describe("createClient — every request goes through the retry policy", () => {
  /** A transport whose first `failures` requests reject with `code`, then succeed. */
  function flaky(failures: number, code: string) {
    const t = fakeTransport();
    let attempts = 0;
    t.request = () => {
      attempts += 1;
      if (attempts <= failures) return Promise.reject(new EngineError(code as never, "boom"));
      return Promise.resolve({ ok: true, attempts });
    };
    return { t, attempts: () => attempts };
  }

  it("retries a read and succeeds on the second attempt", async () => {
    const { t, attempts } = flaky(1, ErrorCode.TIMEOUT);
    const { local } = fakeLocal();
    const client = createClient(t, { local, retry: { sleep: async () => {} } });

    await expect(client.serverStatus()).resolves.toEqual({ ok: true, attempts: 2 });
    expect(attempts()).toBe(2);
  });

  it("does not retry a mutation, because a TIMEOUT means the create may have landed", async () => {
    // The dangerous direction, asserted through the client rather than the predicate.
    const { t, attempts } = flaky(1, ErrorCode.TIMEOUT);
    const { local } = fakeLocal();
    const client = createClient(t, { local, retry: { sleep: async () => {} } });

    await expect(client.addMock({ name: "m" } as never)).rejects.toThrow("boom");
    expect(attempts()).toBe(1);
  });

  it("retries the internal config.get that listWsdls needs, not just surface methods", async () => {
    /**
     * `listWsdls` resolves the active workspace through `config.get`, and that call is *not* a
     * surface method. Routing it around the policy would have left a read unprotected — and one
     * that every `list*` call depends on, so a transient blip there would fail all three listers.
     */
    const t = fakeTransport();
    let configAttempts = 0;
    t.request = (cmd) => {
      if (cmd === "config.get") {
        configAttempts += 1;
        if (configAttempts === 1) return Promise.reject(new EngineError(ErrorCode.TIMEOUT, "boom"));
        return Promise.resolve({ activeWorkspaceId: "ws-1" });
      }
      // `listWsdls` unwraps `.entities` from `entity.list`, so the stub has to carry that shape.
      return Promise.resolve({ entities: [{ id: "wsdl-1" }] });
    };
    const { local } = fakeLocal();
    const client = createClient(t, { local, retry: { sleep: async () => {} } });

    await expect(client.listWsdls()).resolves.toEqual([{ id: "wsdl-1" }]);
    expect(configAttempts).toBe(2);
  });

  it("stops retrying once the client has been closed", async () => {
    /**
     * A closed transport rejects with `ENGINE_ERROR`, which the protocol marks retryable — so
     * without the closure flag this would make three attempts and wait 150ms against a transport
     * the client itself just shut down.
     */
    const t = fakeTransport();
    const { local } = fakeLocal();
    const slept: number[] = [];
    const client = createClient(t, {
      local,
      retry: { attempts: 5, sleep: async (ms) => void slept.push(ms) },
    });

    await client.close();
    let attempts = 0;
    t.request = () => {
      attempts += 1;
      return Promise.reject(new EngineError(ErrorCode.ENGINE_ERROR, "this transport has been closed."));
    };

    await expect(client.serverStatus()).rejects.toThrow("has been closed");
    expect(attempts).toBe(1);
    expect(slept).toEqual([]);
  });

  it("honours attempts: 1, which disables retrying", async () => {
    const { t, attempts } = flaky(1, ErrorCode.TIMEOUT);
    const { local } = fakeLocal();
    const client = createClient(t, { local, retry: { attempts: 1, sleep: async () => {} } });

    await expect(client.serverStatus()).rejects.toThrow("boom");
    expect(attempts()).toBe(1);
  });
});

// ── artifact egress ────────────────────────────────────────────────────────

/**
 * A transport that answers `blob.read` with **real slice semantics** — honouring `offset`, capping
 * at `chunkBytes`, and setting `eof` only once the last slice has been handed over.
 *
 * The stub matters more than the assertions. `blob.read` is a *paged* primitive, so a transport
 * that returns one fixed object makes a single-read implementation look correct: the bug this
 * guards against was exactly that, and it produced an empty file rather than an error.
 */
function slicingTransport(whole: Uint8Array, chunkBytes: number) {
  const calls: { cmd: string; payload: unknown }[] = [];
  const t = fakeTransport();
  t.request = (cmd, payload) => {
    calls.push({ cmd, payload });
    if (cmd === "blob.read") {
      const offset = (payload as { offset?: number }).offset ?? 0;
      const slice = whole.subarray(offset, offset + chunkBytes);
      return Promise.resolve({
        data: Buffer.from(slice).toString("base64"),
        eof: offset + slice.length >= whole.length,
      });
    }
    return Promise.resolve({ ok: true, blobId: "blob-1" });
  };
  return { t, calls };
}

/**
 * A `ClientLocal` whose `writeArtifact` records the base64 it was handed.
 *
 * `pickSavePath` is here because `exportAudit` is **dialog-first** as of 2026-09-20: the key used to
 * take `artifactToFile`'s artifact-first default, which meant these four tests reached the artifact
 * machinery without a dialog ever existing in the fixture. With the ordering corrected the dialog runs
 * first, so a fixture without `pickSavePath` short-circuits to `{ok:false}` and tests nothing — which
 * is exactly how the four below announced the fix. The ordering itself is asserted in
 * "createClient — dialog-first egress" below, not here.
 */
function writingLocal() {
  const writes: { contentBase64: string; suggestedName: string; mimeType: string }[] = [];
  const { local } = fakeLocal();
  return {
    local: {
      ...local,
      pickSavePath: async () => "/tmp/audit-log.json",
      writeArtifact: async (contentBase64: string, suggestedName: string, mimeType: string) => {
        writes.push({ contentBase64, suggestedName, mimeType });
        return { ok: true, filePath: "/tmp/out" };
      },
    } as ClientLocal,
    writes,
  };
}

describe("createClient — artifact egress", () => {
  it("hands an inline artifact to writeArtifact as base64, untouched", async () => {
    const { t } = slicingTransport(new Uint8Array(0), 5);
    // `audit.export`, not `exportAudit` — `call()` maps the surface key onto the command name.
    t.request = (cmd) =>
      cmd === "audit.export"
        ? Promise.resolve({ ok: true, inline: "aGVsbG8gd29ybGQ=", suggestedName: "audit.json" })
        : Promise.resolve({ ok: true });
    const { local, writes } = writingLocal();
    const client = createClient(t, { local });

    await client.exportAudit("json");

    // Not decoded and re-encoded: the same characters the engine sent.
    expect(writes[0]).toEqual({
      contentBase64: "aGVsbG8gd29ybGQ=",
      suggestedName: "audit.json",
      mimeType: "application/octet-stream",
    });
  });

  it("pulls every slice of a blob and hands over the whole thing", async () => {
    /**
     * The regression. `blob.read` caps at `BLOB_READ_CHUNK_BYTES` and `eof` is the only terminator,
     * so a single un-offset read returns the **first** slice and truncates silently. The old
     * implementation did that *and* read the wrong field names (`content`/`base64` instead of
     * `data`), which made the file empty rather than short — no error either way.
     *
     * 20 bytes in 5-byte slices is four reads; asserting the offsets is what proves the loop
     * advances by *decoded bytes* and not by base64 characters.
     */
    const whole = Buffer.from("0123456789abcdefghij");
    const { t, calls } = slicingTransport(whole, 5);
    const { local, writes } = writingLocal();
    const client = createClient(t, { local });

    await client.exportAudit("json");

    expect(calls.filter((c) => c.cmd === "blob.read").map((c) => (c.payload as { offset: number }).offset))
      .toEqual([0, 5, 10, 15]);
    expect(Buffer.from(writes[0].contentBase64, "base64")).toEqual(whole);
  });

  it("keeps a binary artifact byte-exact through the blob path", async () => {
    /**
     * `50 4b 03 04` is a ZIP's magic bytes, and `00 ff fe 80` is not valid UTF-8. Decoding these as
     * a UTF-8 string and re-encoding corrupts them, which is how `workspace-zip` exports broke.
     */
    const zip = Buffer.from([0x50, 0x4b, 0x03, 0x04, 0x00, 0xff, 0xfe, 0x80]);
    const { t } = slicingTransport(zip, 3);
    const { local, writes } = writingLocal();
    const client = createClient(t, { local });

    await client.exportAudit("json");

    expect(Buffer.from(writes[0].contentBase64, "base64")).toEqual(zip);
  });

  it("releases the blob even when a slice read fails", async () => {
    const released: string[] = [];
    const t = fakeTransport();
    t.request = (cmd, payload) => {
      if (cmd === "blob.read") return Promise.reject(new EngineError(ErrorCode.BAD_REQUEST, "no such blob"));
      if (cmd === "blob.release") {
        released.push((payload as { blobId: string }).blobId);
        return Promise.resolve({ ok: true });
      }
      return Promise.resolve({ ok: true, blobId: "blob-1" });
    };
    const { local } = writingLocal();
    const client = createClient(t, { local, retry: { attempts: 1, sleep: async () => {} } });

    await expect(client.exportAudit("json")).rejects.toThrow("no such blob");
    // The lease survives one pull; a failed pull that skips the release pins bytes until the sweep.
    expect(released).toEqual(["blob-1"]);
  });

  it("reports failure rather than succeeding when the client cannot write files", async () => {
    /**
     * A browser client (P7) has no `writeArtifact`. It must not resolve `{ok:true}` and claim an
     * export happened.
     *
     * Asserted as a bare `{ok:false}` and not as the message: `BifurcApi["exportAudit"]` is declared
     * `Promise<{ok: boolean}>`, so `exportAudit` **intentionally** drops the reason — the renderer
     * cannot read a field the type does not declare. `tlsExportCert` and `shareCaptureJson` keep
     * theirs because their declared types carry `error`/`canceled`. The distinction is deliberate,
     * and this test pins the half that discards.
     */
    const { t } = slicingTransport(new Uint8Array(0), 5);
    t.request = (cmd) =>
      cmd === "audit.export" ? Promise.resolve({ ok: true, inline: "eA==" }) : Promise.resolve({ ok: true });
    const { local } = fakeLocal(); // no writeArtifact — a browser client

    const client = createClient(t, { local });

    await expect(client.exportAudit("json")).resolves.toEqual({ ok: false });
  });
});

/**
 * Step 3b-3 — the four methods that needed a **third** primitive.
 *
 * `writeArtifact` bundles "ask where" with "write there", which forces the dialog *after* the render.
 * `File_Ops_Protocol.md` §3.2 requires the opposite, so `pickSavePath` splits the ask out; and the
 * import pair needed `readArtifactFile` to accept a path and hand one back, because the renderer
 * threads `res.filePath` from preflight into `importData` and the old hook returned no path at all.
 *
 * The assertions below are mostly about **ordering and count** rather than about values — "was the
 * dialog opened before the command", "was it opened twice" — because that is what the old shape got
 * wrong, and a value-only assertion passes either way.
 */
function fsLocal(
  opts: {
    savePath?: string | null;
    openPath?: string | null;
    file?: LocalFileContent;
    /**
     * What the `tlsCertStatus` hook reports for the two CA paths.
     *
     * The three TLS shims ask this hook for the path the engine deliberately will not report, so the
     * fixture has to be able to answer with a real one. A stub that always returned `null` would make
     * the shims look broken — and, worse, a shim that *ignored* the answer would look correct.
     */
    certPaths?: { certPath: string | null; keyPath: string | null };
  } = {},
) {
  const seen: string[] = [];
  const { local } = fakeLocal();
  const resolved: ClientLocal = {
    ...local,
    tlsCertStatus: async () => {
      const base = await local.tlsCertStatus();
      return { ...base, certPath: opts.certPaths?.certPath ?? null, keyPath: opts.certPaths?.keyPath ?? null };
    },
    pickSavePath: async (o: SavePathOptions) => {
      seen.push(`pickSavePath:${o.defaultName ?? ""}`);
      return opts.savePath === undefined ? "/tmp/chosen.json" : opts.savePath;
    },
    writeArtifact: async (contentBase64: string, suggestedName: string, mimeType: string, destPath?: string) => {
      seen.push(`writeArtifact:${destPath ?? suggestedName}`);
      return { ok: true, filePath: destPath ?? "/tmp/written" };
    },
    readArtifactFile: async (o?: ReadFileOptions) => {
      seen.push(`readArtifactFile:${o?.path ?? "ask"}`);
      if (opts.openPath === null) return null;
      const file = opts.file ?? { name: "collection.json", size: 2, base64: "eA==", mimeType: "application/json" };
      return { ...file, filePath: o?.path ?? opts.openPath ?? "/tmp/picked.json" };
    },
  };
  return { local: resolved, seen };
}

const FORMATS = { workspace: [{ id: "workspace-zip", label: "Workspace ZIP", extensions: ["zip"] }] };

/**
 * A transport that answers by command and **keeps recording**.
 *
 * `fakeTransport()` records into `t.calls` inside its own `request`, so assigning `t.request`
 * replaces the recorder along with the behaviour — which is how the first version of these tests
 * passed the ordering assertion and then found nothing in `calls`.
 */
function respondingTransport(respond: (cmd: string, payload: unknown) => unknown) {
  const calls: { cmd: string; payload: unknown }[] = [];
  const t = fakeTransport();
  t.request = (cmd, payload) => {
    calls.push({ cmd, payload });
    return Promise.resolve(respond(cmd, payload));
  };
  return { t, calls };
}

describe("createClient — dialog-first egress (step 3b-3)", () => {
  it("exportData asks where BEFORE it asks the engine to render", async () => {
    const order: string[] = [];
    const { local } = fsLocal();
    local.pickSavePath = async () => {
      order.push("dialog");
      return "/tmp/ws.zip";
    };
    const { t, calls } = respondingTransport((cmd) => {
      order.push(cmd);
      if (cmd === "export.formats") return FORMATS;
      if (cmd === "export.create") return { ok: true, inline: "UEsDBA==", mimeType: "application/zip" };
      return { ok: true };
    });
    const client = createClient(t, { local });

    const res = await client.exportData({ kind: "workspace", format: "workspace-zip", wsId: "ws-1" });

    /**
     * `export.formats` is the exception and has to come first: the dialog's default name and its
     * filter come from the engine's format table, so the chrome cannot be built without it. What
     * matters is that `export.create` — the 200 MB render — is last, after the user has committed.
     */
    expect(order).toEqual(["export.formats", "dialog", "export.create"]);
    /**
     * Two things in one assertion, and both were live bugs waiting:
     *
     * - only the *name* crosses — the engine never learns a path it could not act on;
     * - the renderer's `wsId` becomes the schema's `workspaceId`. The client's shim used to read
     *   `req.workspaceId`, a field `ExportRequest` does not have, and nothing noticed because the
     *   preload overrode the method. Routed as written, every export would have been a `BAD_REQUEST`.
     */
    expect(calls.find((c) => c.cmd === "export.create")?.payload).toMatchObject({
      filename: "ws.zip",
      workspaceId: "ws-1",
    });
    expect(res).toEqual({ ok: true, filePath: "/tmp/ws.zip" });
  });

  it("exportData writes to the path the user chose and does not open a second dialog", async () => {
    const { local, seen } = fsLocal();
    const { t } = respondingTransport((cmd) => (cmd === "export.formats" ? FORMATS : { ok: true, inline: "eA==" }));
    const client = createClient(t, { local });

    await client.exportData({ kind: "workspace", format: "workspace-zip", wsId: "ws-1" });

    expect(seen).toEqual(["pickSavePath:workspace-export.zip", "writeArtifact:/tmp/chosen.json"]);
  });

  it("exportData reports a cancel and never runs the command", async () => {
    const { local } = fsLocal({ savePath: null });
    const { t, calls } = respondingTransport((cmd) => (cmd === "export.formats" ? FORMATS : { ok: true }));
    const client = createClient(t, { local });

    await expect(client.exportData({ kind: "workspace", format: "workspace-zip", wsId: "ws-1" }))
      .resolves.toEqual({ ok: false, canceled: true });
    // Fail fast: the whole point of dialog-first is that nothing was rendered.
    expect(calls.some((c) => c.cmd === "export.create")).toBe(false);
  });

  it("exportAudit asks where BEFORE it asks the engine to render", async () => {
    /**
     * The regression this pins, found 2026-09-20 by the §5b-bis twin audit in `plan/07`. `exportAudit`
     * called `artifactToFile(await call("exportAudit", …))` with no `destPath`, so the engine ran
     * first and the dialog second — and `audit.export` is `queryLog({ limit: 0 })`, the workspace's
     * **entire** `git log`. Cancelling therefore still paid for the full query, which is precisely the
     * cost the shell's `syncHandlers.ts` (since deleted) recorded P3 removing when it made this channel
     * dialog-first.
     *
     * Ordering is asserted rather than values because that is the whole defect, and because the
     * default that produced it is invisible in the call: `artifactToFile` is artifact-first unless its
     * third argument is passed. `exportAudit`'s own tests all passed while the order was wrong.
     */
    const order: string[] = [];
    const { local } = fsLocal();
    local.pickSavePath = async () => {
      order.push("dialog");
      return "/tmp/audit-log.json";
    };
    const { t } = respondingTransport((cmd) => {
      order.push(cmd);
      return { ok: true, inline: "eA==" };
    });
    const client = createClient(t, { local });

    await client.exportAudit("json");

    // No format-table call here, unlike `exportData`: `audit.export` takes `{format}` alone, so the
    // dialog's chrome needs nothing from the engine and the command has no reason to precede it.
    expect(order).toEqual(["dialog", "audit.export"]);
  });

  it("exportAudit reports a cancel and never runs the command", async () => {
    const { local } = fsLocal({ savePath: null });
    const { t, calls } = respondingTransport(() => ({ ok: true, inline: "eA==" }));
    const client = createClient(t, { local });

    await expect(client.exportAudit("json")).resolves.toEqual({ ok: false });
    // Fail fast: nothing was rendered, so the whole-log query never ran.
    expect(calls.some((c) => c.cmd === "audit.export")).toBe(false);
  });

  it("shareCaptureJson renders BEFORE it asks — the deliberate exception to dialog-first", async () => {
    /**
     * The **counterpart** to the two `exportAudit` cases above, and the confirmation that made
     * deleting the shell's `capture:shareJson` channel safe (P6 reduction, fifth group, 2026-09-21).
     *
     * The shell channel was dialog-first — save dialog, then `capture.shareJson`, then write. The
     * client is deliberately the other way round, and `local.ts`'s egress note gives the reason: this
     * artifact is a `JSON.stringify` of entries the renderer already holds, so rendering it before the
     * user commits costs nothing, whereas asking first would throw away the engine's own
     * `suggestedName` as the dialog's default. `audit.export` is the opposite case — `queryLog({
     * limit: 0 })`, the whole `git log` — which is why that one *had* to become dialog-first and this
     * one must not.
     *
     * Two facts are pinned, and neither is about values: that no `pickSavePath` happens (the hook is
     * called with no `destPath`, so it shows the dialog itself, after the render), and that the hook is
     * handed the **artifact's** name rather than the caller's fallback. Without this the difference
     * lives only in a docblock — and `exportAudit` is the worked example of what happens when a
     * docblock and a call disagree: the docblock was edited to sanction the bug.
     */
    const order: string[] = [];
    const { local } = fsLocal();
    local.writeArtifact = async (_b: string, suggestedName: string, _m: string, destPath?: string) => {
      order.push(`writeArtifact:${destPath ?? suggestedName}`);
      return { ok: true, filePath: destPath ?? "/tmp/written" };
    };
    const { t } = respondingTransport((cmd) => {
      order.push(cmd);
      return cmd === "capture.shareJson"
        ? { ok: true, inline: "eA==", suggestedName: "captured-requests.json", mimeType: "application/json" }
        : { ok: true };
    });
    const client = createClient(t, { local });

    const res = await client.shareCaptureJson([{ id: "x" }], "capture-GET-1.json");

    // The command first, then the hook — and no `pickSavePath` anywhere in between.
    expect(order).toEqual(["capture.shareJson", "writeArtifact:captured-requests.json"]);
    expect(res).toEqual({ ok: true, filePath: "/tmp/written" });
  });

  /**
   * The regression this replaced: the client hardcoded `format: "json"`, so routing
   * `exportRunnerReport` silently deleted HTML export — which is the dialog's *default*. The chosen
   * extension **is** the format, which is the starkest reason the dialog cannot run after the
   * render.
   */
  it.each([
    ["/tmp/report.json", "json"],
    ["/tmp/report.html", "html"],
    ["/tmp/report", "html"],
  ])("exportRunnerReport derives the format from the chosen extension (%s → %s)", async (dest, format) => {
    const { local } = fsLocal({ savePath: dest });
    const { t, calls } = respondingTransport(() => ({ ok: true, inline: "eA==" }));
    const client = createClient(t, { local });

    await client.exportRunnerReport({ folderName: "Smoke", startedAt: 0 });

    expect(calls.find((c) => c.cmd === "runner.exportReport")?.payload).toMatchObject({ format });
  });

  it("exportRunnerReport resolves a bare {ok:false} on cancel, as the declared type requires", async () => {
    const { local } = fsLocal({ savePath: null });
    const { t } = respondingTransport(() => ({ ok: true }));
    const client = createClient(t, { local });

    // No `canceled` key: `BifurcApi["exportRunnerReport"]` does not declare one, so the renderer
    // cannot branch on it. `toEqual` would accept an undefined-valued key, hence the explicit check.
    const res = await client.exportRunnerReport({});
    expect(res).toEqual({ ok: false });
    expect(Object.keys(res)).toEqual(["ok"]);
  });
});

describe("createClient — single-dialog ingress (step 3b-3)", () => {
  const IMPORT_FORMATS = { requests: [{ id: "json", label: "JSON", extensions: ["json"] }] };

  it("preflightImport returns the path it chose, so the next step can reuse it", async () => {
    const { local, seen } = fsLocal({ openPath: "/tmp/picked.json" });
    const { t, calls } = respondingTransport((cmd) => {
      if (cmd === "export.formats") return IMPORT_FORMATS;
      if (cmd === "blob.put") return { blobId: "blob-7" };
      if (cmd === "import.preflight") return { ok: true, itemCount: 3, collisionIds: [] };
      return { ok: true };
    });
    const client = createClient(t, { local });

    const res = await client.preflightImport({ kind: "requests", format: "json", wsId: "ws-1" });

    expect(res).toEqual({ ok: true, filePath: "/tmp/picked.json", itemCount: 3, collisionIds: [] });
    // Asked once, and only once — the blob is staged here and reused by `importData`.
    expect(seen).toEqual(["readArtifactFile:ask"]);
    expect(calls.find((c) => c.cmd === "import.preflight")?.payload).toMatchObject({
      blobId: "blob-7",
      workspaceId: "ws-1",
    });
  });

  it("preflightImport reports a cancel without uploading", async () => {
    const { local } = fsLocal({ openPath: null });
    const { t, calls } = respondingTransport((cmd) => (cmd === "export.formats" ? IMPORT_FORMATS : { ok: true }));
    const client = createClient(t, { local });

    await expect(client.preflightImport({ kind: "requests", format: "json", wsId: "ws-1" }))
      .resolves.toEqual({ ok: false, canceled: true });
    expect(calls.some((c) => c.cmd === "blob.put")).toBe(false);
  });

  /**
   * The bug this step exists to fix, asserted end to end through both halves: `importData` is given
   * the path `preflightImport` returned, and it must (a) not ask again and (b) not re-upload bytes
   * the engine already has. `File_Ops_Protocol.md` §5 calls the second one "uploaded ONCE".
   *
   * The strongest form of (a): the file is not even **read** a second time, because the remembered
   * blob short-circuits before the read.
   */
  it("importData reuses the path and the blob — no second dialog, no second upload", async () => {
    const { local, seen } = fsLocal({ openPath: "/tmp/picked.json" });
    const { t, calls } = respondingTransport((cmd) => {
      if (cmd === "export.formats") return IMPORT_FORMATS;
      if (cmd === "blob.put") return { blobId: "blob-7" };
      if (cmd === "import.preflight") return { ok: true, itemCount: 3, collisionIds: ["c1"] };
      if (cmd === "import.commit") return { ok: true, imported: 3 };
      return { ok: true };
    });
    const client = createClient(t, { local });

    const pre = await client.preflightImport({ kind: "requests", format: "json", wsId: "ws-1" });
    const res = await client.importData({
      kind: "requests",
      format: "json",
      wsId: "ws-1",
      filePath: pre.filePath!,
      collisionStrategy: "keep",
    });

    expect(res).toEqual({ ok: true, imported: 3 });
    expect(seen).toEqual(["readArtifactFile:ask"]);
    expect(calls.filter((c) => c.cmd === "blob.put")).toHaveLength(1);
    expect(calls.find((c) => c.cmd === "import.commit")?.payload).toMatchObject({
      collisionStrategy: "keep",
      workspaceId: "ws-1",
    });
    // Transform-and-discard: the source file never persists on the engine.
    expect(calls.filter((c) => c.cmd === "blob.release")).toHaveLength(1);
  });

  /**
   * The recovery path: the renderer sends a path but this process has no remembered blob for it —
   * an app restart between the two steps, which is exactly why the shell had `uploadFor`'s fallback.
   * The path is still reused, so the user is not asked again; only the read is repeated.
   */
  it("importData reads a path it was given when no blob is remembered", async () => {
    const { local, seen } = fsLocal();
    const { t, calls } = respondingTransport((cmd) => {
      if (cmd === "blob.put") return { blobId: "blob-9" };
      if (cmd === "import.commit") return { ok: true, imported: 1 };
      return { ok: true };
    });
    const client = createClient(t, { local });

    const res = await client.importData({
      kind: "requests",
      format: "json",
      wsId: "ws-1",
      filePath: "/tmp/from-preflight.json",
      collisionStrategy: "keep",
    });

    expect(res).toEqual({ ok: true, imported: 1 });
    expect(seen).toEqual(["readArtifactFile:/tmp/from-preflight.json"]);
    expect(calls.filter((c) => c.cmd === "blob.put")).toHaveLength(1);
  });

  it("importData asks when the renderer sends no path, rather than reading undefined", async () => {
    const { local, seen } = fsLocal({ openPath: "/tmp/picked.json" });
    const { t } = respondingTransport((cmd) => {
      if (cmd === "export.formats") return IMPORT_FORMATS;
      if (cmd === "blob.put") return { blobId: "blob-7" };
      if (cmd === "import.commit") return { ok: true, imported: 1 };
      return { ok: true };
    });
    const client = createClient(t, { local });

    // The renderer's `ImportRequest` declares `filePath` required, so this is the recovery path for a
    // caller that skipped preflight. The old code read `undefined` and reported a confusing fs error.
    const res = await client.importData({
      kind: "requests",
      format: "json",
      wsId: "ws-1",
      filePath: "",
      collisionStrategy: "keep",
    });

    expect(res).toEqual({ ok: true, imported: 1 });
    expect(seen).toEqual(["readArtifactFile:ask"]);
  });
});

/**
 * The three methods whose only reason for being shims is a path the engine will not report.
 *
 * `File_Ops_Protocol.md` §8 keeps the engine's directory layout out of the protocol, so `tls.generate`
 * answers `{ok, fingerprint}` and the two imports answer `{ok}`. `TlsSettingsSection.tsx` records
 * `tlsCaCertPath` / `tlsCaKeyPath` from the result and reads a missing field as "not configured" — so
 * routing these three as 1:1 transport calls, which is what P6 step 3b-1 did, made the Generate and
 * Import buttons no-ops on the settings: the CA landed on disk and the UI never learned it.
 *
 * The assertions below are what fails if anyone files them as `transport` again. That is the whole
 * point of pinning them here rather than only in `surface.test.ts`'s counts: the count would still be
 * 144 if the classification regressed, and the counts test cannot see a dropped *field*.
 */
describe("createClient — the TLS path shims (step 3b-1 regression)", () => {
  const CERT = "C:/data/ca-cert.pem";
  const KEY = "C:/data/ca-key.pem";

  it("tlsGenerate appends the shell's certPath and keyPath to the engine's answer", async () => {
    const { local } = fsLocal({ certPaths: { certPath: CERT, keyPath: KEY } });
    const { t, calls } = respondingTransport(() => ({ ok: true, fingerprint: "sha256:abc" }));
    const client = createClient(t, { local });

    const res = await client.tlsGenerate();

    expect(res).toEqual({ ok: true, fingerprint: "sha256:abc", certPath: CERT, keyPath: KEY });
    // The engine is asked once and only for the command: it has no path to give, so the extra step
    // must be a client-local hook rather than another round-trip.
    expect(calls.map((c) => c.cmd)).toEqual(["tls.generate"]);
  });

  it("tlsGenerate reports a failure without inventing a path", async () => {
    const { local } = fsLocal({ certPaths: { certPath: CERT, keyPath: KEY } });
    const { t } = respondingTransport(() => ({ ok: false, error: "mkcert not found" }));
    const client = createClient(t, { local });

    await expect(client.tlsGenerate()).resolves.toEqual({ ok: false, error: "mkcert not found" });
  });

  it("tlsImportCert and tlsImportKey each append their own path, and not the other's", async () => {
    const { local } = fsLocal({
      openPath: "C:/picked/ca.pem",
      certPaths: { certPath: CERT, keyPath: KEY },
    });
    const { t } = respondingTransport((cmd) => (cmd === "blob.put" ? { blobId: "blob-9" } : { ok: true }));
    const client = createClient(t, { local });

    await expect(client.tlsImportCert()).resolves.toEqual({ ok: true, path: CERT });
    await expect(client.tlsImportKey()).resolves.toEqual({ ok: true, path: KEY });
  });

  it("a cancelled TLS import stays a bare {ok:false} and never reaches the engine", async () => {
    const { local } = fsLocal({ openPath: null });
    const { t, calls } = respondingTransport(() => ({ ok: true }));
    const client = createClient(t, { local });

    await expect(client.tlsImportCert()).resolves.toEqual({ ok: false });
    expect(calls.some((c) => c.cmd.startsWith("tls."))).toBe(false);
  });
});
