/**
 * `src/ipcTransport.ts` — the preload half of the P6 seam.
 *
 * This is the mirror of `tests/ipc/rpcBridge.test.ts`, and the pair is what makes the boundary
 * testable at all: one side is asserted to *produce* a survivable failure shape, the other to
 * *consume* it. Neither test can prove the round trip on its own — that is what the e2e suite is
 * for — but together they pin both ends of the contract that the round trip has to honour.
 *
 * The case that matters most is **"rebuilds an EngineError, preserving the code"**. It is the reason
 * the frame is discriminated rather than a rejection: Electron flattens a rejection crossing
 * `ipcMain.handle` → `ipcRenderer.invoke` and drops `EngineError.code`, and
 * `@bifurc/client`'s retry policy branches on exactly that code.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

// ── Module mocks (hoisted) ────────────────────────────────────────────────────

const { transportCalls, responder } = vi.hoisted(() => {
  const calls: { channel: string; frame: any }[] = [];
  const r: { impl: (channel: string, frame: any) => Promise<any> } = {
    impl: async () => ({ ok: true, value: undefined }),
  };
  return { transportCalls: calls, responder: r };
});

vi.mock("electron", () => ({
  ipcRenderer: {
    invoke: (channel: string, frame: any) => {
      transportCalls.push({ channel, frame });
      return responder.impl(channel, frame);
    },
  },
}));

import { EngineError } from "@bifurc/protocol";
import { createIpcTransport } from "@/ipcTransport";
import { RPC_CHANNEL } from "@/ipc/rpcContract";

beforeEach(() => {
  transportCalls.length = 0;
  responder.impl = async () => ({ ok: true, value: undefined });
});

describe("src/ipcTransport.ts", () => {
  it("reports itself as the ipc kind, not in-process", () => {
    // Dispatch is identical to `in-process`; the trust model is not. The renderer is a separate
    // process behind `contextIsolation: true`, so anything keying off the kind must not be told
    // this is a direct function call.
    expect(createIpcTransport().kind).toBe("ipc");
  });

  it("sends {cmd, payload} on the shared RPC channel", async () => {
    const transport = createIpcTransport();
    await transport.request("config.get", { workspaceId: "ws-1" });

    expect(transportCalls).toHaveLength(1);
    expect(transportCalls[0].channel).toBe(RPC_CHANNEL);
    expect(transportCalls[0].frame).toEqual({ cmd: "config.get", payload: { workspaceId: "ws-1" } });
  });

  it("unwraps a successful frame and returns the value", async () => {
    responder.impl = async () => ({ ok: true, value: { activeWorkspaceId: "ws-1" } });

    await expect(createIpcTransport().request("config.get", {})).resolves.toEqual({
      activeWorkspaceId: "ws-1",
    });
  });

  it("rebuilds an EngineError, preserving the code", async () => {
    responder.impl = async () => ({
      ok: false,
      error: { code: "CONFLICT", message: "an unnamed entity would be affected" },
    });

    const err = await createIpcTransport()
      .request("entity.setEnabled", {})
      .then(() => null)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(EngineError);
    // Not ENGINE_ERROR. This single assertion is what keeps `isRetryable(err.code)` meaningful on
    // the far side of the hop.
    expect((err as EngineError).code).toBe("CONFLICT");
    expect((err as EngineError).message).toBe("an unnamed entity would be affected");
  });

  it("passes a resolved {ok:false} value through untouched", async () => {
    // The two `ok` flags are different questions: the outer one is the IPC hop, the inner one is the
    // handler's own report. A handler that resolves `{ok:false}` is an envelope-level SUCCESS, so its
    // value must arrive as data rather than as a rejection.
    responder.impl = async () => ({ ok: true, value: { ok: false, error: "no active workspace" } });

    await expect(createIpcTransport().request("config.get", {})).resolves.toEqual({
      ok: false,
      error: "no active workspace",
    });
  });

  it("classifies an unrecognisable failure payload as ENGINE_ERROR", async () => {
    responder.impl = async () => ({ ok: false, error: "something went wrong" });

    const err = await createIpcTransport()
      .request("config.get", {})
      .then(() => null)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(EngineError);
    expect((err as EngineError).code).toBe("ENGINE_ERROR");
  });

  it("classifies a failed hop as ENGINE_ERROR, naming the bridge", async () => {
    // What actually happens when the channel has no handler — the bridge was never started, or this
    // preload is talking to an older main process. Electron's own rejection is untyped, so it has to
    // be reclassified: reaching the retry policy with no code at all is the failure this whole
    // result shape exists to prevent.
    responder.impl = async () => {
      throw new Error("No handler registered for 'engine:rpc'");
    };

    const err = await createIpcTransport()
      .request("config.get", {})
      .then(() => null)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(EngineError);
    expect((err as EngineError).code).toBe("ENGINE_ERROR");
    expect((err as EngineError).message).toContain("RPC bridge is not reachable");
    expect((err as EngineError).message).toContain("No handler registered");
  });

  it("refuses to subscribe rather than accepting an event it can never deliver", async () => {
    // The transport contract requires UNSUPPORTED here. A silent no-op would be indistinguishable
    // from "nothing has happened yet" and would be debugged as a broken engine.
    //
    // Refusing is safe in step 2 only because the client's hub is lazy — `createSubscriptionHub`
    // does not call `subscribe()` until the first listener registers, and the shell's `on*` methods
    // still use the legacy channels. This test is the tripwire for that assumption: if the hub ever
    // becomes eager, this throws at client-construction time instead of at 3am.
    const transport = createIpcTransport();

    expect(() => transport.subscribe(["event.log.entry"], () => {})).toThrowError(EngineError);
    try {
      transport.subscribe(["event.log.entry"], () => {});
    } catch (e) {
      expect((e as EngineError).code).toBe("UNSUPPORTED");
    }
  });

  it("rejects requests after close(), and close() is idempotent", async () => {
    const transport = createIpcTransport();
    await transport.close();
    await transport.close(); // idempotent

    const err = await transport
      .request("config.get", {})
      .then(() => null)
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(EngineError);
    // `ENGINE_ERROR` is the *retryable* code, which is exactly why `retry.ts` also checks the
    // transport is open — a closed transport would otherwise be retried three times.
    expect((err as EngineError).code).toBe("ENGINE_ERROR");
    expect(transportCalls).toHaveLength(0);
  });
});
