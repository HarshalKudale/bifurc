import { useState, useEffect, useRef, useCallback } from "react";
import { Environment } from "@/types";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";

// -- Connection registry (module-level, shared across all hook instances) -----

/** Max concurrent live WebSocket connections. */
export const MAX_WS_CONNECTIONS = 5;

/** Map of tabId -> WebSocket for all currently open connections. */
const activeConnections = new Map<string, WebSocket>();

export function getActiveConnectionCount(): number {
  return activeConnections.size;
}

export function canOpenNewConnection(): boolean {
  return activeConnections.size < MAX_WS_CONNECTIONS;
}

const STATUS_COLORS: Record<string, string> = {
  connected:    "var(--c-green)",
  connecting:   "var(--c-yellow)",
  disconnected: "var(--c-text-dim)",
  error:        "var(--c-red)",
};

function dispatchStatusColor(tabId: string, status: WsStatus): void {
  window.dispatchEvent(new CustomEvent("ws:statuscolor", {
    detail: { tabId, color: STATUS_COLORS[status] ?? "var(--c-text-dim)" },
  }));
}

// -- Types ---------------------------------------------------------------------

export type WsStatus = "disconnected" | "connecting" | "connected" | "error";

export interface WsMessage {
  id: string;
  ts: number;
  direction: "sent" | "received";
  data: string;
}

// -- Hook ----------------------------------------------------------------------

interface UseWebSocketOptions {
  tabId: string;
  activeEnv?: Environment | null;
}

interface UseWebSocketReturn {
  status: WsStatus;
  error: string | null;
  messages: WsMessage[];
  connect(url: string, headers: Record<string, string>): void;
  disconnect(): void;
  send(data: string): void;
  clearMessages(): void;
  isAtConnectionLimit: boolean;
}

let msgIdCounter = 0;
function nextMsgId() {
  return `msg-${Date.now()}-${++msgIdCounter}`;
}

export function useWebSocket({ tabId, activeEnv = null }: UseWebSocketOptions): UseWebSocketReturn {
  const [status,   setStatus]   = useState<WsStatus>("disconnected");
  const [error,    setError]    = useState<string | null>(null);
  const [messages, setMessages] = useState<WsMessage[]>([]);
  const [isAtConnectionLimit, setIsAtConnectionLimit] = useState(false);

  const wsRef = useRef<WebSocket | null>(null);

  // Keep limit state in sync whenever the registry changes
  const refreshLimit = useCallback(() => {
    setIsAtConnectionLimit(!canOpenNewConnection());
  }, []);

  // Clean up on unmount (tab close)
  useEffect(() => {
    return () => {
      const ws = activeConnections.get(tabId);
      if (ws) {
        ws.close(1000, "Tab closed");
        activeConnections.delete(tabId);
      }
      wsRef.current = null;
      refreshLimit();
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabId]);

  const connect = useCallback((rawUrl: string, rawHeaders: Record<string, string>) => {
    // Close any existing connection for this tab
    const existing = activeConnections.get(tabId);
    if (existing) {
      existing.close(1000, "Reconnecting");
      activeConnections.delete(tabId);
      refreshLimit();
    }

    if (!canOpenNewConnection()) {
      setError(`Max ${MAX_WS_CONNECTIONS} active connections reached. Disconnect another tab first.`);
      setStatus("error");
      return;
    }

    const resolvedUrl = resolveVars(rawUrl.trim(), activeEnv);
    // Headers can't be passed directly to the WebSocket browser API.
    // We resolve them for future reference but they are stored, not injected.
    resolveHeaders(rawHeaders, activeEnv); // side-effect: validation/logging only

    setStatus("connecting");
    setError(null);
    dispatchStatusColor(tabId, "connecting");

    let ws: WebSocket;
    try {
      ws = new WebSocket(resolvedUrl);
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Invalid WebSocket URL";
      setError(msg);
      setStatus("error");
      return;
    }

    wsRef.current = ws;
    activeConnections.set(tabId, ws);
    refreshLimit();

    ws.onopen = () => {
      setStatus("connected");
      setError(null);
      refreshLimit();
      dispatchStatusColor(tabId, "connected");
    };

    ws.onclose = (ev) => {
      activeConnections.delete(tabId);
      wsRef.current = null;
      refreshLimit();
      if (ev.wasClean) {
        setStatus("disconnected");
        dispatchStatusColor(tabId, "disconnected");
      } else {
        setError(ev.reason || `Connection closed (code ${ev.code})`);
        setStatus("error");
        dispatchStatusColor(tabId, "error");
      }
    };

    ws.onerror = () => {
      // onerror always precedes onclose; set status/error here too for immediate feedback
      setError("WebSocket error — connection failed or was refused");
      setStatus("error");
      dispatchStatusColor(tabId, "error");
    };

    ws.onmessage = (ev) => {
      const data = typeof ev.data === "string" ? ev.data : "[binary data]";
      setMessages((prev) => [...prev, { id: nextMsgId(), ts: Date.now(), direction: "received", data }]);
    };
  }, [tabId, activeEnv, refreshLimit]);

  const disconnect = useCallback(() => {
    const ws = activeConnections.get(tabId);
    if (ws) {
      ws.close(1000, "User disconnected");
      activeConnections.delete(tabId);
      wsRef.current = null;
      refreshLimit();
    }
    setStatus("disconnected");
    setError(null);
    dispatchStatusColor(tabId, "disconnected");
  }, [tabId, refreshLimit]);

  const send = useCallback((data: string) => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      setError("Not connected");
      return;
    }
    const resolved = resolveVars(data, activeEnv);
    ws.send(resolved);
    setMessages((prev) => [...prev, { id: nextMsgId(), ts: Date.now(), direction: "sent", data: resolved }]);
  }, [activeEnv]);

  const clearMessages = useCallback(() => setMessages([]), []);

  // Refresh limit state on each render so callers always see the latest value
  useEffect(() => {
    setIsAtConnectionLimit(!canOpenNewConnection());
  });

  return { status, error, messages, connect, disconnect, send, clearMessages, isAtConnectionLimit };
}
