import { useState, useEffect, useRef, useCallback } from "react";
import type { WsMessage, WsListResult, WsStatusResult, WsStatusEvent, WsMessageEvent } from "@/types";

/**
 * The socket tab's view of the engine's connection registry — `plan/19` WI 6.
 *
 * ## What this file used to be, and why that mattered
 *
 * Until 2026-09-30 this hook *was* the WebSocket client. It held a module-level `Map<string, WebSocket>`,
 * constructed the socket itself — the single transport construction `tests/gui/protocolModule.test.ts`
 * existed to find, and the reason its allowlist is now empty — resolved the url and the outgoing payload
 * with `@/lib/resolveVars`, and owned the message log in React state.
 *
 * Two consequences followed, and they are the whole reason for the move:
 *
 * 1. **The renderer *was* the connection's lifetime.** Closing the window, reloading it, or anything that
 *    discarded the browser context dropped every socket — because the object holding them was a browser
 *    object. `plan/19` WI 6's one-liner says it: *"a socket dies with the window and a CLI cannot open one."*
 * 2. **A CLI could not open one.** `Protocol_Module_Contract.md` §1: every byte reaching the wire is the
 *    engine's. A socket the engine cannot open is a protocol no second client can speak.
 *
 * ## What it is now: a **mirror**, not the authority
 *
 * It holds React state, and the distinction that matters is what the state is *used* for. Nothing here
 * decides anything: whether a socket is open comes from `wsStatus` and `event.ws.status`, the connection
 * limit comes from `wsList`, and "what is in the log" comes from the engine's retained tail. That is §2's
 * test applied to this file — *would this line differ if the UI were a terminal?* — and the answer is no:
 * a terminal would keep exactly the same view.
 *
 * This is `useCollectionRunner`'s shape one namespace over, and it is deliberate rather than convergent: the
 * engine owns the state, the hook subscribes to it and renders it.
 *
 * ## ⚠ Two things this hook deliberately does **not** do
 *
 * 1. **It does not disconnect on unmount.** That is D29 stated in the UI. A client going away — a closed
 *    window, a reload, a panel switch — must not cancel engine-owned work, and an unmount effect cannot tell
 *    those apart from a user closing a tab. So the *only* close is an explicit `disconnect()` call, and
 *    `WebSocketsPanel` issues it from the tab-close handler, which **is** a user gesture.
 *
 *    The visible consequence, recorded rather than hidden: a tab that disappears without being closed leaves
 *    its socket running, which is now the point — reopening that tab re-attaches and shows the retained
 *    transcript. The connection-limit banner reports the engine's live count, so those sockets are visible
 *    rather than mysterious.
 * 2. **It does not resolve anything.** `{{host}}` in the url, `{{token}}` in a header value and
 *    `{{random.*}}` in an outgoing frame are all resolved engine-side (D23) — and that is not merely a move:
 *    the browser `WebSocket` constructor **cannot send headers at all**, so a resolved `Authorization` header
 *    used to be discarded. Engine-side it reaches the wire.
 *
 * ## ⚠ The hydration race, and the buffer that exists for it
 *
 * `wsStatus` is a round trip; the events are not. A frame can therefore arrive **before** the status response
 * that would have given us a snapshot to append it to, and the naive order — subscribe, then hydrate, then
 * replace state wholesale — silently drops it. So message events are buffered until hydration lands and
 * merged in by `id` afterwards.
 *
 * That is `HealthBarPanel`'s stale-closure lesson in its other form: **the async value and the consumer of it
 * do not arrive together.** The engine minting an `id` per message is what makes the merge exact rather than
 * approximate — without it, "the same frame twice" and "two identical frames" are indistinguishable.
 */

export type WsStatus = "disconnected" | "connecting" | "connected" | "error";

interface UseWebSocketOptions {
  workspaceId: string;
  /** The caller's name for the socket. For the UI this is the tab id it already had — the engine has no
   *  notion of a tab and does not pretend to. */
  tabId: string;
}

interface UseWebSocketReturn {
  status: WsStatus;
  error: string | null;
  messages: WsMessage[];
  /** Resolves `{ok:false}` when the registry refuses — the connection limit, or a malformed url. */
  connect(url: string, headers: Record<string, string>): Promise<{ ok: boolean; error?: string }>;
  disconnect(): Promise<{ ok: boolean; error?: string }>;
  /** `{ok:false, error:"Not connected"}` when nothing is open, rather than a throw. */
  send(data: string): Promise<{ ok: boolean; error?: string }>;
  clearMessages(): Promise<{ ok: boolean; error?: string }>;
  /** True when the workspace is at the registry's limit — **and it includes this tab's own socket.** */
  isAtConnectionLimit: boolean;
  /** The registry's limit, from the engine, or `null` until the first `wsList` answers. */
  maxConnections: number | null;
}

export function useWebSocket({ workspaceId, tabId }: UseWebSocketOptions): UseWebSocketReturn {
  const [snapshot, setSnapshot] = useState<WsStatusResult | null>(null);
  const [list, setList] = useState<WsListResult | null>(null);
  /** Failures the *commands* reported — a refused connect, a send with nothing open. Distinct from
   *  `snapshot.error`, which is the socket's own state, and the two are shown in the same line. */
  const [localError, setLocalError] = useState<string | null>(null);

  /**
   * Messages that arrived before `wsStatus` answered. See the header.
   *
   * Both are refs rather than state on purpose: they gate a *write*, and a state update here would re-render
   * for something nothing renders.
   */
  const pendingRef = useRef<WsMessage[]>([]);
  const hydratedRef = useRef(false);

  const refreshList = useCallback(() => {
    void window.api.wsList(workspaceId).then(setList).catch(() => { /* the count is advisory */ });
  }, [workspaceId]);

  useEffect(() => {
    let cancelled = false;
    hydratedRef.current = false;
    pendingRef.current = [];

    // `wsStatus` on mount, so a tab opened **while a socket is live** attaches to it — D29 visible in the UI,
    // and the reason the engine's snapshot outlives the socket.
    void window.api.wsStatus(workspaceId, tabId).then((fresh) => {
      if (cancelled) return;
      const pending = pendingRef.current;
      pendingRef.current = [];
      hydratedRef.current = true;
      setSnapshot(fresh === null ? null : {
        ...fresh,
        // Anything that arrived during the round trip, minus anything the engine already retained — the two
        // overlap whenever a frame lands in both the log and a live event.
        messages: [
          ...fresh.messages,
          ...pending.filter((m) => !fresh.messages.some((x) => x.id === m.id)),
        ],
      });
    }).catch(() => { hydratedRef.current = true; });

    refreshList();

    /**
     * **Both handlers establish state when there is none**, which is the `Protocol_Module_Contract.md` §4
     * requirement in its code form. A handler that only *amended* a snapshot it already had would silently
     * drop every event for exactly the client the engine-owned lifetime exists for — the one that attached
     * after the fact. `useCollectionRunner` shipped that bug first and its tests caught it; this is the same
     * fix, written deliberately.
     */
    const offStatus = window.api.onWsStatus((p: WsStatusEvent) => {
      if (p.socketId !== tabId) return;
      setSnapshot((prev) => ({
        ...(prev ?? blankSnapshot(workspaceId, tabId)),
        status: p.status,
        error: p.error,
        ...(p.code === undefined ? {} : { closeCode: p.code }),
        ...(p.reason === undefined ? {} : { closeReason: p.reason }),
      }));
      // The limit is the engine's answer, so any state change can move it — including another tab's.
      refreshList();
    });

    const offMessage = window.api.onWsMessage((p: WsMessageEvent) => {
      if (p.socketId !== tabId) return;
      if (!hydratedRef.current) {
        pendingRef.current.push(p.message);
        return;
      }
      setSnapshot((prev) => {
        const base = prev ?? blankSnapshot(workspaceId, tabId);
        if (base.messages.some((m) => m.id === p.message.id)) return base;
        return { ...base, messages: [...base.messages, p.message] };
      });
    });

    /**
     * Unsubscribe only. **No `wsDisconnect` here** — see the header. An unmount is not a user gesture, and
     * D29 forbids inferring a close from one.
     */
    return () => {
      cancelled = true;
      offStatus();
      offMessage();
    };
  }, [workspaceId, tabId, refreshList]);

  const connect = useCallback(async (url: string, headers: Record<string, string>) => {
    setLocalError(null);
    const res = await window.api.wsConnect(workspaceId, tabId, url.trim(), headers);
    if (!res.ok) {
      // The registry's refusal is shown inline rather than thrown, which is why it answers `{ok:false}`.
      setLocalError(res.error ?? "Connect failed");
    } else {
      refreshList();
    }
    return res;
  }, [workspaceId, tabId, refreshList]);

  const disconnect = useCallback(async () => {
    setLocalError(null);
    const res = await window.api.wsDisconnect(workspaceId, tabId);
    if (!res.ok) setLocalError(res.error ?? "Disconnect failed");
    refreshList();
    return res;
  }, [workspaceId, tabId, refreshList]);

  const send = useCallback(async (data: string) => {
    setLocalError(null);
    const res = await window.api.wsSend(workspaceId, tabId, data);
    if (!res.ok) setLocalError(res.error ?? "Send failed");
    // The sent frame reaches the log through `event.ws.message`, not from here — the engine wrote it, so the
    // engine is what says it was written. Appending locally would be a second, competing account of the log.
    return res;
  }, [workspaceId, tabId]);

  const clearMessages = useCallback(async () => {
    const res = await window.api.wsClearMessages(workspaceId, tabId);
    // Only mirror the clear once the engine has confirmed it. A `ws.message` event landing in between is
    // appended afterwards by the subscription, which is the correct order.
    if (res.ok) setSnapshot((prev) => (prev === null ? prev : { ...prev, messages: [] }));
    return res;
  }, [workspaceId, tabId]);

  const maxConnections = list?.max ?? null;

  return {
    status: snapshot?.status ?? "disconnected",
    // The socket's own error takes precedence: it is the more specific fact, and the local one is cleared on
    // the next command either way.
    error: snapshot?.error ?? localError,
    messages: snapshot?.messages ?? [],
    connect,
    disconnect,
    send,
    clearMessages,
    isAtConnectionLimit: list !== null && list.connections.length >= list.max,
    maxConnections,
  };
}

/**
 * A snapshot for a socket the engine has told us about but `wsStatus` has not yet answered for.
 *
 * `url` is empty rather than guessed: the event carries no url, and the editor renders the url from its own
 * input state anyway. Filling it in from the caller's input would make the mirror disagree with the engine
 * about what was connected.
 */
function blankSnapshot(workspaceId: string, socketId: string): WsStatusResult {
  return {
    workspaceId,
    socketId,
    url: "",
    status: "connecting",
    error: null,
    openedAt: null,
    messages: [],
  };
}
