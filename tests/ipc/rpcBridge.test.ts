/**
 * `src/ipc/rpcBridge.ts` — the main-process half of the P6 seam.
 *
 * What this file is for: the bridge's *job* is to make a failure survive Electron's IPC hop. That is
 * a claim about a boundary, so the assertions here are about the boundary's shape rather than about
 * any command — which command is used matters only in that `invoke()` validates against the
 * protocol's frozen schema, so a real `CommandAction` name is required.
 *
 * The two cases that carry the most weight:
 *
 *  - **"keeps the code of an EngineError the handler threw"** — the reason the result is
 *    discriminated at all. If this regresses, `@bifurc/client`'s `isRetryable(err.code)` sees an
 *    unclassified failure and the retry policy silently degrades to all-or-nothing.
 *  - **"a handler that RESOLVES `{ok:false}` is an envelope-level success"** — the two-`ok`-flags
 *    rule. `{ok:false}` inside `value` and `ok:false` at the frame level mean different things, and
 *    a "helpful" unwrap that collapsed them would break every legacy handler that reports failure by
 *    resolving, which is most of them.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

// ── Module mocks (hoisted) ────────────────────────────────────────────────────

const { mockIpcMain, registeredHandlers } = vi.hoisted(() => {
  const handlers = new Map<string, (...args: any[]) => any>();
  const ipcMain = {
    handle: (channel: string, handler: (...args: any[]) => any) => {
      handlers.set(channel, handler);
    },
  };
  return { mockIpcMain: ipcMain, registeredHandlers: handlers };
});

vi.mock("electron", () => ({ ipcMain: mockIpcMain }));

import type { CommandAction } from "@bifurc/protocol";
import type { CommandRegistry } from "@bifurc/engine/commands/registry";
import { RPC_CHANNEL, type RpcResult } from "@/ipc/rpcContract";

const EVENT = {} as never;

/**
 * Build a bridge over a **fresh** module graph.
 *
 * `vi.resetModules()` is load-bearing rather than tidiness: `commandRegistry` is a process-wide
 * singleton whose `register()` **throws on a duplicate**, and `registerRpcBridge()` sets a
 * module-level `registered` flag. Without a reset, the second test in this file would fail on both —
 * and the failure would look like a bridge bug rather than test pollution.
 *
 * ## The trap `resetModules()` sets, and why `EngineError` is returned rather than imported
 *
 * Resetting the module registry gives the bridge a **second copy of `@bifurc/protocol`**, and
 * therefore a second `EngineError` class. `toRpcError()` decides what to do with a thrown value via
 * `err instanceof EngineError` — so an `EngineError` constructed from the *test's* copy fails that
 * check against the *bridge's* copy, falls through to the generic branch, and the specific code is
 * **silently degraded to `ENGINE_ERROR`**.
 *
 * That is not a hypothetical: it is exactly what this file did on its first run, and the symptom was
 * a wrong error code rather than a crash. So the constructor is handed back from here, and tests that
 * throw must use the one the bridge will actually recognise.
 *
 * The same hazard exists in production if two copies of the protocol ever load — a CJS `index.js`
 * and an ESM `index.mjs`, say — which is worth remembering when `plan/07` step 3 moves more traffic
 * onto this path.
 */
async function loadBridge(
  register: (r: CommandRegistry, protocol: typeof import("@bifurc/protocol")) => void,
) {
  vi.resetModules();
  const registryModule = await import("@bifurc/engine/commands/registry");
  const protocolModule = await import("@bifurc/protocol");
  const bridgeModule = await import("@/ipc/rpcBridge");

  register(registryModule.commandRegistry, protocolModule);
  bridgeModule.registerRpcBridge();

  const handler = registeredHandlers.get(RPC_CHANNEL);
  if (!handler) throw new Error(`the bridge did not register a handler on "${RPC_CHANNEL}"`);

  return {
    registry: registryModule.commandRegistry,
    registerRpcBridge: bridgeModule.registerRpcBridge,
    /** The class *this* module graph recognises — see the trap above. */
    EngineError: protocolModule.EngineError,
    /** Call the registered handler exactly as Electron would. */
    call: (frame: unknown): Promise<RpcResult> => handler(EVENT, frame),
  };
}

/** The frame the preload half actually sends. */
const frame = (cmd: string, payload: unknown = {}) => ({ cmd, payload });

beforeEach(() => {
  registeredHandlers.clear();
});

describe("src/ipc/rpcBridge.ts", () => {
  it("registers its handler on the shared RPC channel", async () => {
    await loadBridge((r) => r.register("config.get" as CommandAction, () => ({}) ));
    expect(registeredHandlers.has(RPC_CHANNEL)).toBe(true);
  });

  it("resolves {ok:true, value} for a registered command", async () => {
    const bridge = await loadBridge((r) =>
      r.register("config.get" as CommandAction, () => ({ activeWorkspaceId: "ws-1" })),
    );

    expect(await bridge.call(frame("config.get"))).toEqual({
      ok: true,
      value: { activeWorkspaceId: "ws-1" },
    });
  });

  it("keeps the code of an EngineError the handler threw", async () => {
    const bridge = await loadBridge((r, { EngineError }) =>
      r.register("config.get" as CommandAction, () => {
        throw new EngineError("CONFLICT", "an unnamed entity would be affected");
      }),
    );

    const result = await bridge.call(frame("config.get"));

    expect(result.ok).toBe(false);
    // The whole point: not flattened to ENGINE_ERROR.
    expect(result.ok === false && result.error.code).toBe("CONFLICT");
    expect(result.ok === false && result.error.message).toBe("an unnamed entity would be affected");
  });

  it("classifies a plain Error as ENGINE_ERROR", async () => {
    const bridge = await loadBridge((r) =>
      r.register("config.get" as CommandAction, () => {
        throw new Error("the disk went away");
      }),
    );

    const result = await bridge.call(frame("config.get"));

    expect(result.ok).toBe(false);
    expect(result.ok === false && result.error.code).toBe("ENGINE_ERROR");
    expect(result.ok === false && result.error.message).toBe("the disk went away");
  });

  it("reports an unregistered command as UNKNOWN_COMMAND", async () => {
    const bridge = await loadBridge(() => {
      /* nothing registered */
    });

    const result = await bridge.call(frame("config.get"));

    expect(result.ok).toBe(false);
    expect(result.ok === false && result.error.code).toBe("UNKNOWN_COMMAND");
  });

  it("reports a payload the protocol schema rejects as BAD_REQUEST", async () => {
    const bridge = await loadBridge((r) =>
      r.register("config.get" as CommandAction, () => ({})),
    );

    // `invoke()` validates against the command's frozen Zod schema, so a payload of the wrong
    // *type* is a client error rather than an engine fault — and it must be reported as one.
    const result = await bridge.call({ cmd: "config.get", payload: "not-an-object" });

    expect(result.ok).toBe(false);
    expect(result.ok === false && result.error.code).toBe("BAD_REQUEST");
  });

  it("treats a handler that RESOLVES {ok:false} as an envelope-level success", async () => {
    const bridge = await loadBridge((r) =>
      r.register("config.get" as CommandAction, () => ({ ok: false, error: "no active workspace" })),
    );

    const result = await bridge.call(frame("config.get"));

    // The two `ok` flags answer different questions. The outer one describes the IPC hop; the inner
    // one is the handler's own report and rides through untouched, because the renderer branches on
    // it. Collapsing them would break every legacy handler that reports failure this way.
    expect(result).toEqual({ ok: true, value: { ok: false, error: "no active workspace" } });
  });

  it("resolves a classified failure for a malformed frame instead of throwing", async () => {
    const bridge = await loadBridge((r) => r.register("config.get" as CommandAction, () => ({})));

    // A frame of `undefined` would make `frame.cmd` a TypeError. Destructuring inside the `try` is
    // what turns that into a classified result; letting it escape would hand Electron a rejection,
    // which it would flatten — the exact failure the discriminated result exists to prevent.
    const result = await bridge.call(undefined);

    expect(result.ok).toBe(false);
    expect(result.ok === false && result.error.code).toBe("UNKNOWN_COMMAND");
  });

  it("is idempotent — a second registration does not throw or replace the handler", async () => {
    const bridge = await loadBridge((r) =>
      r.register("config.get" as CommandAction, () => ({ activeWorkspaceId: "first" })),
    );

    const before = registeredHandlers.get(RPC_CHANNEL);
    // `ipcMain.handle()` throws on a duplicate channel, so this must be a no-op rather than a
    // second `handle()` call.
    expect(() => bridge.registerRpcBridge()).not.toThrow();
    expect(registeredHandlers.get(RPC_CHANNEL)).toBe(before);
  });
});
