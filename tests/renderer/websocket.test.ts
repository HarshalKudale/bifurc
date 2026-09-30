// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { useWebSocket } from "@/hooks/useWebSocket";
import type { WsMessage, WsMessageEvent, WsStatusEvent, WsStatusResult } from "@/types";

/**
 * `useWebSocket`'s cases, rewritten for `plan/19` WI 6.
 *
 * ## What this file used to be, and why every case in it is gone
 *
 * It tested a `MockWebSocket` class and the renderer's registry helpers (`MAX_WS_CONNECTIONS`,
 * `getActiveConnectionCount`, `canOpenNewConnection`) — because the hook *was* the client: it held the
 * sockets, constructed them and resolved their payloads. WI 6 moved all of that to
 * `packages/engine/src/websocket/`, so those cases did not move; they lost their subject. The engine's
 * registry now has its own suite (`tests/websocket/wsRegistry.test.ts`), and the *transports* are the
 * ratchets' business (`tests/gui/protocolModule.test.ts` is at `{}` because this file's subject no longer
 * constructs one).
 *
 * ## What is left to test, and it is not nothing
 *
 * The hook is a **mirror over `window.api`**, and the two things a mirror can get wrong are the two this
 * file is about:
 *
 * 1. **The hydration race.** `wsStatus` is a round trip and the events are not, so a frame can arrive
 *    *before* the snapshot it belongs to. The naive order — subscribe, hydrate, then replace state wholesale
 *    — silently drops it. The engine minting an `id` per message is what makes the merge exact.
 * 2. **D29 in the UI: no disconnect on unmount.** An unmount cannot tell "the user closed this tab" from "the
 *    window is going away", so the hook must not infer a close from either. `WebSocketsPanel` issues the
 *    explicit `wsDisconnect` from the tab-close handler, which *is* a gesture.
 *
 * Both are asserted directly rather than through a rendered component, because both are properties of the
 * hook's contract with the engine.
 */

type StatusListener = (p: WsStatusEvent) => void;
type MessageListener = (p: WsMessageEvent) => void;

function snapshot(over: Partial<WsStatusResult> = {}): WsStatusResult {
  return {
    workspaceId: "ws-1",
    socketId: "tab-1",
    url: "wss://example.test/socket",
    status: "connected",
    error: null,
    openedAt: 1,
    messages: [],
    ...over,
  };
}

const msg = (id: string, data = "hello"): WsMessage => ({ id, ts: 1, direction: "received", data });

/**
 * A `window.api` stand-in covering the six `ws.*` commands and the two events.
 *
 * The listeners are real sets rather than `vi.fn()`s so a case can *emit* — which is the only way to drive
 * the ordering the hydration case is about. `wsStatus` is left as a controllable promise by default, since
 * "the response has not arrived yet" is the state under test.
 */
function installApi(over: Record<string, unknown> = {}) {
  const statusListeners = new Set<StatusListener>();
  const messageListeners = new Set<MessageListener>();

  let resolveStatus: (v: WsStatusResult | null) => void = () => {};
  const pendingStatus = () =>
    new Promise<WsStatusResult | null>((resolve) => { resolveStatus = resolve; });

  const api = {
    wsStatus: vi.fn(() => pendingStatus()),
    wsList: vi.fn(async () => ({ connections: [], max: 5 })),
    wsConnect: vi.fn(async () => ({ ok: true })),
    wsDisconnect: vi.fn(async () => ({ ok: true })),
    wsSend: vi.fn(async () => ({ ok: true })),
    wsClearMessages: vi.fn(async () => ({ ok: true })),
    onWsStatus: vi.fn((cb: StatusListener) => {
      statusListeners.add(cb);
      return () => { statusListeners.delete(cb); };
    }),
    onWsMessage: vi.fn((cb: MessageListener) => {
      messageListeners.add(cb);
      return () => { messageListeners.delete(cb); };
    }),
    ...over,
  };

  (window as unknown as { api: unknown }).api = api;

  return {
    api,
    settleStatus: (v: WsStatusResult | null) => { resolveStatus(v); },
    emitStatus: (p: WsStatusEvent) => { statusListeners.forEach((cb) => cb(p)); },
    emitMessage: (p: WsMessageEvent) => { messageListeners.forEach((cb) => cb(p)); },
    statusListenerCount: () => statusListeners.size,
    messageListenerCount: () => messageListeners.size,
  };
}

let harness: ReturnType<typeof installApi>;

beforeEach(() => { harness = installApi(); });
afterEach(() => { vi.clearAllMocks(); });

const opts = { workspaceId: "ws-1", tabId: "tab-1" };

describe("useWebSocket — the mirror over window.api", () => {
  it("subscribes to both events and hydrates from wsStatus on mount", async () => {
    const { result } = renderHook(() => useWebSocket(opts));

    expect(harness.api.wsStatus).toHaveBeenCalledWith("ws-1", "tab-1");
    expect(harness.statusListenerCount()).toBe(1);
    expect(harness.messageListenerCount()).toBe(1);

    await act(async () => { harness.settleStatus(snapshot({ messages: [msg("m1")] })); });

    expect(result.current.status).toBe("connected");
    expect(result.current.messages.map((m) => m.id)).toEqual(["m1"]);
  });

  it("⚠ does not drop a message that arrives BEFORE wsStatus answers (the hydration race)", async () => {
    // The whole reason the hook buffers. Without the buffer the `settleStatus` below would replace state
    // wholesale and `m-live` would vanish — which is invisible in every test that hydrates first.
    const { result } = renderHook(() => useWebSocket(opts));

    act(() => { harness.emitMessage({ workspaceId: "ws-1", socketId: "tab-1", message: msg("m-live") }); });

    await act(async () => { harness.settleStatus(snapshot({ messages: [msg("m-retained")] })); });

    expect(result.current.messages.map((m) => m.id)).toEqual(["m-retained", "m-live"]);
  });

  it("does not double-count a frame that is both retained and live", async () => {
    // The overlap case, and it is the normal one: a frame that lands in the engine's log *and* on the wire
    // while the status round trip is in flight. The engine-minted `id` is what makes this exact.
    const { result } = renderHook(() => useWebSocket(opts));

    act(() => { harness.emitMessage({ workspaceId: "ws-1", socketId: "tab-1", message: msg("m1") }); });
    await act(async () => { harness.settleStatus(snapshot({ messages: [msg("m1")] })); });

    expect(result.current.messages.map((m) => m.id)).toEqual(["m1"]);
  });

  it("appends live frames after hydration", async () => {
    const { result } = renderHook(() => useWebSocket(opts));
    await act(async () => { harness.settleStatus(snapshot()); });

    act(() => { harness.emitMessage({ workspaceId: "ws-1", socketId: "tab-1", message: msg("m1", "a") }); });
    act(() => { harness.emitMessage({ workspaceId: "ws-1", socketId: "tab-1", message: msg("m2", "b") }); });

    expect(result.current.messages.map((m) => m.data)).toEqual(["a", "b"]);
  });

  it("routes events by socketId — another tab's socket does not touch this one", async () => {
    const { result } = renderHook(() => useWebSocket(opts));
    await act(async () => { harness.settleStatus(snapshot()); });

    // `await act(async …)` rather than the sync form throughout: a status event also triggers `wsList`, whose
    // resolution is a second state update — so a sync `act` leaves it to land after the test and React warns.
    await act(async () => {
      harness.emitStatus({ workspaceId: "ws-1", socketId: "tab-2", status: "error", error: "not mine" });
      harness.emitMessage({ workspaceId: "ws-1", socketId: "tab-2", message: msg("other") });
    });

    expect(result.current.status).toBe("connected");
    expect(result.current.error).toBeNull();
    expect(result.current.messages).toEqual([]);
  });

  it("⚠ establishes state from a status event when it has no snapshot", async () => {
    // `Protocol_Module_Contract.md` §4's requirement in its code form. A handler that only *amended* a
    // snapshot it already had would drop every event for exactly the client the engine-owned lifetime exists
    // for — the one that attached after the fact. `useCollectionRunner` shipped that bug first.
    const { result } = renderHook(() => useWebSocket(opts));
    // `wsStatus` never settles, so there is no snapshot when the event arrives.
    await act(async () => {
      harness.emitStatus({ workspaceId: "ws-1", socketId: "tab-1", status: "error", error: "refused", code: 1006 });
    });

    expect(result.current.status).toBe("error");
    expect(result.current.error).toBe("refused");
  });

  it("⚠ does NOT disconnect on unmount — D29 in the UI", async () => {
    const { unmount } = renderHook(() => useWebSocket(opts));
    await act(async () => { harness.settleStatus(snapshot()); });

    unmount();

    // The one assertion this file exists for. An unmount cannot tell a closed tab from a closed window, and
    // inferring a close from it would make the socket die with the window — the defect WI 6 removes.
    expect(harness.api.wsDisconnect).not.toHaveBeenCalled();
    // …and it does tear its subscriptions down, which is the opposite direction and equally required.
    expect(harness.statusListenerCount()).toBe(0);
    expect(harness.messageListenerCount()).toBe(0);
  });

  it("derives the connection limit from wsList, including the engine's own max", async () => {
    harness = installApi({
      wsList: vi.fn(async () => ({ connections: [snapshot(), snapshot({ socketId: "tab-2" })], max: 2 })),
    });

    const { result } = renderHook(() => useWebSocket(opts));

    await waitFor(() => expect(result.current.maxConnections).toBe(2));
    expect(result.current.isAtConnectionLimit).toBe(true);
  });

  it("surfaces a refused connect as the hook's error, rather than throwing", async () => {
    harness = installApi({ wsConnect: vi.fn(async () => ({ ok: false, error: "Max 5 active connections reached." })) });

    const { result } = renderHook(() => useWebSocket(opts));

    await act(async () => { await result.current.connect("wss://example.test", {}); });

    expect(result.current.error).toBe("Max 5 active connections reached.");
  });

  it("sends the payload through the engine and does NOT append it locally", async () => {
    // The engine wrote the frame, so the engine's `event.ws.message` is what puts it in the log. Appending
    // here would be a second, competing account of the same log — and it would double up once the event
    // arrived, since the hook has no way to know the two are the same frame.
    const { result } = renderHook(() => useWebSocket(opts));
    await act(async () => { harness.settleStatus(snapshot()); });

    await act(async () => { await result.current.send("ping"); });

    expect(harness.api.wsSend).toHaveBeenCalledWith("ws-1", "tab-1", "ping");
    expect(result.current.messages).toEqual([]);

    act(() => { harness.emitMessage({ workspaceId: "ws-1", socketId: "tab-1", message: { id: "s1", ts: 2, direction: "sent", data: "ping" } }); });
    expect(result.current.messages.map((m) => m.direction)).toEqual(["sent"]);
  });

  it("mirrors a clear only once the engine confirms it", async () => {
    const { result } = renderHook(() => useWebSocket(opts));
    await act(async () => { harness.settleStatus(snapshot({ messages: [msg("m1")] })); });
    expect(result.current.messages).toHaveLength(1);

    await act(async () => { await result.current.clearMessages(); });

    expect(harness.api.wsClearMessages).toHaveBeenCalledWith("ws-1", "tab-1");
    expect(result.current.messages).toEqual([]);
  });

  it("leaves the log alone when the clear is refused", async () => {
    // `ws.clearMessages` answers `{ok:false}` for an id the engine has never been asked to open, and a mirror
    // that cleared anyway would disagree with the engine about what it holds.
    harness = installApi({ wsClearMessages: vi.fn(async () => ({ ok: false, error: "No socket." })) });

    const { result } = renderHook(() => useWebSocket(opts));
    await act(async () => { harness.settleStatus(snapshot({ messages: [msg("m1")] })); });

    await act(async () => { await result.current.clearMessages(); });

    expect(result.current.messages.map((m) => m.id)).toEqual(["m1"]);
  });
});
