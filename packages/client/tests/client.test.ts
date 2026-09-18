/**
 * `createClient` — the acceptance criteria that need a live client.
 *
 * The headline case is the key diff: `Object.keys(client)` must equal `Object.keys(window.api)`
 * exactly. That is the plan's "a script asserting `Object.keys(oldApi)` vs `Object.keys(newApi)`
 * reports zero differences", written as a test rather than a script so it runs inside the gate
 * instead of being remembered.
 *
 * The preload is imported **for real** (with `electron` mocked) rather than read from the type,
 * because `renderer/types/window.ts` under-declares the runtime by four keys — a type-driven diff
 * would pass while the client was missing them. See `src/surface.ts`'s header.
 */

import { describe, expect, it, vi } from "vitest";
import { EngineError, ErrorCode } from "@bifurc/protocol";
import type { Transport, EngineEvent } from "@bifurc/engine/transport/types";
import { createClient } from "../src/index";
import type { ClientLocal } from "../src/local";

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

  it("injects the entity kind for the CRUD collapse", async () => {
    const t = fakeTransport();
    const { local } = fakeLocal();
    const client = createClient(t, { local });

    await client.addMock({ name: "m" } as never);
    await client.deleteRule("r-1");

    expect(t.calls[0]).toEqual({ cmd: "entity.create", payload: { kind: "mocks", entity: { name: "m" } } });
    expect(t.calls[1]).toEqual({ cmd: "entity.delete", payload: { kind: "proxyRules", id: "r-1" } });
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

/** A `ClientLocal` whose `writeArtifact` records the base64 it was handed. */
function writingLocal() {
  const writes: { contentBase64: string; suggestedName: string; mimeType: string }[] = [];
  const { local } = fakeLocal();
  return {
    local: {
      ...local,
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
