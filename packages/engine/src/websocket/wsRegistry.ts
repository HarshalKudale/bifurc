/**
 * The engine's **live-socket registry** — `plan/19` WI 6.
 *
 * ## What moved, and what was wrong before
 *
 * `packages/gui/src/hooks/useWebSocket.ts` owned a module-level `Map<string, WebSocket>` and did the whole
 * protocol in the renderer. Two things followed, and they are the reason this file exists:
 *
 * 1. **The renderer *was* the connection's lifetime.** A socket died with the window — with a reload, with a
 *    panel switch, with anything that discarded the browser context — because the object holding it was a
 *    browser object. Nothing in that design could have been otherwise.
 * 2. **A CLI could not open one.** `Protocol_Module_Contract.md` §1: every byte reaching the wire is the
 *    engine's. A socket the engine cannot open is a protocol no second client can speak.
 *
 * ## The four decisions `Protocol_Module_Contract.md` §4 asks for
 *
 * ### Lifetime — D29, and there is deliberately **no disconnect hook in this file**
 *
 * A client disconnect and a workspace switch do **not** close a socket, and a host restart loses every one.
 * Nothing here can observe a client going away: there is no teardown registration, no "last subscriber left"
 * path, and no reference to a transport. `disconnectSocket()` is the **only** lever, and `ws.disconnect` is
 * its only caller.
 *
 * That is §8 trap 5's requirement stated as code — *"a disconnected client must not cancel the run … and
 * 'cancel on disconnect' is the default most implementations reach for"* — and a reader looking for where a
 * disconnect is handled will not find one. This paragraph is why.
 *
 * **The state outlives the socket**, on `runner.status`'s precedent: a closed socket keeps its entry so
 * `ws.status` can answer *what happened to it*, and a reconnect replaces the entry rather than adding one.
 *
 * ### Concurrency — one socket per `{workspaceId, socketId}`, and a second connect REPLACES
 *
 * Deliberately **not** D30's refusal, and the difference is the kind of thing each command makes. A *run* is a
 * unit of work, so a duplicate is effort nobody asked for twice. A *socket* is a channel, so a second connect
 * on a live key is a **reconnect** — the renderer did exactly that ("close any existing connection for this
 * tab"), and refusing would make a broken socket unrecoverable without a separate disconnect first.
 *
 * The replacement is **atomic in the sense that matters**: the old socket is marked superseded *before* it is
 * closed, so its close handler cannot emit a `disconnected` event that arrives after the new `connecting`
 * one. That is what the `generation` counter below is for, and it is the kind of ordering bug that shows up
 * as an intermittent wrong-coloured dot rather than as a failure.
 *
 * ### Event completeness — `ws.status` returns the retained log, not just the state
 *
 * The `total`-rides-every-progress-event rule in its socket form: a client that attached mid-stream never saw
 * the connect call, so "the state changed" is not enough to render. `getSocket()` therefore returns the whole
 * snapshot, the retained messages included.
 *
 * ### Scope — the split lives in `transport/auth/scopes.ts`
 *
 * `connect`/`disconnect`/`send` are `execute`; `status`/`list` are `read`; `clearMessages` is `write`. This
 * file does not enforce it (the transport decorator does) but it is what lets a reconnecting client ask what
 * happened without being granted the right to open or close anything.
 *
 * ## Two behaviour changes that are improvements rather than ports
 *
 * Both were limitations of the *browser* API that the engine does not inherit, and neither was reachable
 * before because the renderer had no way to express it:
 *
 * 1. **Headers now reach the wire.** The browser `WebSocket` constructor cannot send headers at all — the
 *    renderer's own comment said the resolved headers were "stored, not injected". The `ws` client can, so a
 *    `{{TOKEN}}` in an Authorization header went from *resolved and discarded* to *resolved and sent*.
 * 2. **Binary frames are detected properly.** The renderer decided with `typeof ev.data === "string"`, which
 *    is a proxy for the frame's opcode rather than a reading of it. The `ws` client hands over `isBinary`
 *    directly, so a text frame that happens to decode to a Buffer is no longer mislabelled.
 *
 * ## ⚠ The message log is bounded, and the bound is a decision
 *
 * `MAX_WS_MESSAGES` per socket. An unbounded per-socket log would be a leak in a process that now **outlives
 * every client** — which is the whole point of D29 — so "the engine keeps the history" has to come with a
 * ceiling. A client that needs every frame must consume `event.ws.message` as it arrives; `ws.status` returns
 * the retained tail, and the two are documented as different things rather than one being quietly lossy.
 */
import { WebSocket, type RawData } from "ws";
import type { WsConnectionState, WsMessage } from "@bifurc/protocol";
import { loadMergedEnvironment } from "../env/envState";
import { resolveHeaders, resolveVars } from "../env/resolve";

/**
 * Max concurrent **live** sockets per workspace. Carried over from the renderer's `MAX_WS_CONNECTIONS`, which
 * is now the engine's constant — and reported on the wire by `ws.list` rather than mirrored into the UI,
 * because the registry is what refuses the sixth connection.
 */
export const MAX_WS_CONNECTIONS = 5;

/** The retained message log's ceiling per socket. See the header — this is a decision, not a default. */
export const MAX_WS_MESSAGES = 500;

/** What a caller sees. Mirrors `WsStatusResult` in `@bifurc/protocol`, **which is the wire authority**. */
export interface SocketSnapshot {
  workspaceId: string;
  socketId: string;
  /** The url **as the caller sent it**, unresolved (surrounding whitespace trimmed), so it can be shown back. */
  url: string;
  status: WsConnectionState;
  error: string | null;
  closeCode?: number;
  closeReason?: string;
  /** Epoch ms of the successful open, or `null` if it never opened. */
  openedAt: number | null;
  messages: WsMessage[];
}

/**
 * The event seam, supplied at connect time and kept for the socket's life.
 *
 * `startRun` takes its callbacks per call, and that works because a run's callbacks are used within the call
 * that started it. A socket's are not: its `open`/`message`/`close` handlers fire long after `ws.connect`
 * returned, so the seam has to be stored on the entry. **The bus is still injected rather than imported** —
 * `websocket/`'s §6 row grants `kernel/` for *types*, and a headless host builds its own bus, so reaching for
 * the singleton here would deliver events to a bus nobody is listening to.
 */
export interface SocketEmitters {
  status: (state: {
    status: WsConnectionState;
    error: string | null;
    code?: number;
    reason?: string;
  }) => void;
  message: (message: WsMessage) => void;
}

interface Entry extends SocketSnapshot {
  socket: WebSocket | null;
  /**
   * Bumped on every connect. Handlers capture the value they were installed with and no-op when the entry has
   * moved on — which is what stops a superseded socket's late `close` from overwriting the new socket's state.
   */
  generation: number;
  /**
   * Set by `disconnectSocket`. A close the *caller* asked for is not an error, even though the `ws` client
   * reports an aborted handshake as one (`error` then `close(1006)`) when the socket was still CONNECTING.
   */
  closeRequested: boolean;
  emit: SocketEmitters | null;
  /** Monotonic per entry; the suffix of each message's `id`. */
  seq: number;
}

/**
 * The registry. **Module-level, and that is D29** — see the header. Keyed by `{workspaceId, socketId}`;
 * `\u0000` as the separator for the reason `runRegistry` records (an id containing a colon is not forbidden
 * anywhere, and a collision would attach one workspace's socket to another's).
 */
const SOCKETS = new Map<string, Entry>();

const keyOf = (workspaceId: string, socketId: string): string => `${workspaceId}\u0000${socketId}`;

/** Test seam: the map is module-level by design, so a suite needs a way to start from empty. */
export function resetSocketsForTests(): void {
  for (const entry of SOCKETS.values()) {
    // `terminate` rather than `close`: a test that has to wait for a graceful close is a test that is about
    // the `ws` client rather than about this module.
    try { entry.socket?.terminate(); } catch { /* already gone */ }
    entry.socket = null;
    entry.emit = null;
  }
  SOCKETS.clear();
}

const isLive = (e: Entry): boolean => e.socket !== null;

/** The snapshot for one socket — live or finished — or `null` if the engine has never opened that id. */
export function getSocket(workspaceId: string, socketId: string): SocketSnapshot | null {
  const entry = SOCKETS.get(keyOf(workspaceId, socketId));
  return entry === undefined ? null : snapshotOf(entry);
}

/**
 * The **live** sockets for a workspace.
 *
 * Live only, and the contrast with `getSocket` is the design: this answers *what is open?* (a connection-limit
 * indicator, a CLI's `ws list`), while `getSocket` answers *what happened to this one?* — which is why a
 * finished socket keeps its entry and does not appear here.
 */
export function listSockets(workspaceId: string): SocketSnapshot[] {
  return [...SOCKETS.values()]
    .filter((e) => e.workspaceId === workspaceId && isLive(e))
    .map(snapshotOf);
}

export function liveSocketCount(workspaceId: string): number {
  let n = 0;
  for (const e of SOCKETS.values()) if (e.workspaceId === workspaceId && isLive(e)) n++;
  return n;
}

export interface ConnectSocketOptions {
  workspaceId: string;
  socketId: string;
  url: string;
  headers?: Record<string, string>;
  emit: SocketEmitters;
}

/**
 * Open a socket, replacing any live one under the same key. See the header for the concurrency policy.
 *
 * **Resolution happens here and only here** (D23): the url and every header *value* are resolved against the
 * workspace's merged environment — `loadMergedEnvironment`, so `__global__` sits under the active environment
 * and a shadowed key resolves the way the request pipeline resolves it. Resolving against the active
 * environment alone is `plan/19` §8 trap 2, and it is invisible in every test that uses no shadowed key.
 */
export function connectSocket(opts: ConnectSocketOptions): { ok: boolean; error?: string } {
  const key = keyOf(opts.workspaceId, opts.socketId);
  const existing = SOCKETS.get(key);
  const env = loadMergedEnvironment(opts.workspaceId);

  // The limit is checked **before** the replacement, and only when the key has no live socket of its own —
  // reconnecting to a socket you already hold must not be refused by the socket you are replacing.
  if (!(existing && isLive(existing)) && liveSocketCount(opts.workspaceId) >= MAX_WS_CONNECTIONS) {
    return {
      ok: false,
      error:
        `Max ${MAX_WS_CONNECTIONS} active connections reached. ` +
        `Disconnect another socket first (ws.disconnect), or list them with ws.list.`,
    };
  }

  const resolvedUrl = resolveVars(opts.url.trim(), env);
  const resolvedHeaders = opts.headers === undefined ? undefined : resolveHeaders(opts.headers, env);

  // Supersede the old socket *before* closing it, so its close handler cannot emit a `disconnected` that
  // arrives after the `connecting` below. See the header.
  if (existing) {
    existing.generation += 1;
    const old = existing.socket;
    existing.socket = null;
    if (old) {
      try { old.close(1000, "Reconnecting"); } catch { /* already gone */ }
    }
  }

  const entry: Entry = existing ?? {
    workspaceId: opts.workspaceId,
    socketId: opts.socketId,
    url: opts.url,
    status: "connecting",
    error: null,
    openedAt: null,
    messages: [],
    socket: null,
    generation: 0,
    closeRequested: false,
    emit: null,
    seq: 0,
  };

  // Reset the mutable half; the retained message log deliberately survives a reconnect (the renderer's
  // `messages` state did too, and a reconnect that silently dropped the transcript would be worse).
  entry.url = opts.url.trim();
  entry.status = "connecting";
  entry.error = null;
  entry.openedAt = null;
  delete entry.closeCode;
  delete entry.closeReason;
  entry.closeRequested = false;
  entry.emit = opts.emit;
  SOCKETS.set(key, entry);

  const gen = entry.generation;
  /** Every handler no-ops once the entry has moved on — the supersede guard, in one place. */
  const stale = (): boolean => entry.generation !== gen;

  entry.emit.status({ status: "connecting", error: null });

  let socket: WebSocket;
  try {
    socket = resolvedHeaders === undefined
      ? new WebSocket(resolvedUrl)
      : new WebSocket(resolvedUrl, { headers: resolvedHeaders });
  } catch (e) {
    // A malformed url throws synchronously. The renderer caught this too, and its message is the shape the
    // UI renders — "Invalid WebSocket URL" when the throw carried nothing.
    const message = e instanceof Error ? e.message : "Invalid WebSocket URL";
    entry.status = "error";
    entry.error = message;
    entry.emit.status({ status: "error", error: message });
    return { ok: false, error: message };
  }

  entry.socket = socket;

  socket.on("open", () => {
    if (stale()) return;
    entry.status = "connected";
    entry.error = null;
    entry.openedAt = Date.now();
    entry.emit?.status({ status: "connected", error: null });
  });

  socket.on("message", (data: RawData, isBinary: boolean) => {
    if (stale()) return;
    push(entry, "received", isBinary ? "[binary data]" : data.toString());
  });

  socket.on("error", (err: Error) => {
    if (stale()) return;
    // A close the caller asked for is not a failure, even when the `ws` client reports it as one — an aborted
    // handshake emits `error` before `close(1006)`. The close handler finishes the transition.
    if (entry.closeRequested) return;
    entry.status = "error";
    entry.error = err.message;
    entry.emit?.status({ status: "error", error: err.message });
  });

  socket.on("close", (code: number, reason: Buffer) => {
    if (stale()) return;
    entry.socket = null;
    const text = reason.length > 0 ? reason.toString() : "";

    /**
     * `code === 1000` is a normal closure, **whether we asked for it or the peer did** — the `ws` client has
     * no `wasClean`, and this is the reading that reproduces the renderer's `ev.wasClean` branch. Anything
     * else is an abnormal closure and is reported as an error with the peer's reason, which is what the
     * renderer's `else` branch did with `ev.reason`.
     *
     * **⚠ `closeRequested` overrides the code, and the first version of this handler did not — a test caught
     * it.** A disconnect while the socket is still CONNECTING aborts the handshake, and `ws` reports that as
     * `close(1006)`: abnormal. Read by code alone, a user pressing Disconnect mid-connect would be shown
     * "Connection closed (code 1006)". So the flag the `error` handler already consults is consulted here
     * too — **a close the caller asked for is clean whatever code the client reports**, because the code
     * describes how the *handshake* ended and the flag describes who ended it.
     */
    const clean = code === 1000 || entry.closeRequested;
    const status: WsConnectionState = clean ? "disconnected" : "error";
    const error = clean ? null : (text || `Connection closed (code ${code})`);

    entry.status = status;
    entry.error = error;
    entry.closeCode = code;
    if (text) entry.closeReason = text;
    entry.emit?.status({ status, error, code, reason: text || undefined });
  });

  return { ok: true };
}

/**
 * Close a socket. The **only** lever that stops one, which is D29 stated as code.
 *
 * `{ok:true}` for a socket that is already closed, matching `runner.cancel`'s answer for a finished run: the
 * caller's intent ("this should not be open") is satisfied either way, and the alternative is a race every
 * client would have to defend against. `{ok:false}` only when the engine has never been asked to open that id,
 * which is a real distinction — a typo'd `socketId` must not read as success.
 */
export function disconnectSocket(
  workspaceId: string,
  socketId: string,
  code?: number,
  reason?: string,
): { ok: boolean; error?: string } {
  const entry = SOCKETS.get(keyOf(workspaceId, socketId));
  if (!entry) {
    return { ok: false, error: `No socket "${socketId}" in workspace "${workspaceId}".` };
  }

  const socket = entry.socket;
  if (!socket) {
    // Already closed. Answer as the caller's intent, not as a no-op — same rule as a finished run.
    return { ok: true };
  }

  entry.closeRequested = true;
  try {
    socket.close(code ?? 1000, reason ?? "User disconnected");
  } catch {
    // A socket that cannot be closed gracefully is still not open from here: the entry's socket is dropped and
    // the state set directly, because the close handler may never fire.
    entry.socket = null;
    entry.status = "disconnected";
    entry.error = null;
    entry.emit?.status({ status: "disconnected", error: null });
  }
  return { ok: true };
}

/**
 * Resolve the payload against the merged environment, then write it — and log the **resolved** text.
 *
 * `{{random.*}}` mints per send here, which is the correct behaviour for a stream and the *opposite* of the
 * request pipeline's problem (`plan/19` §8 trap 3): each frame is its own event, not a retry of an earlier
 * one, so re-minting is what the caller asked for rather than a hazard.
 */
export function sendSocketMessage(
  workspaceId: string,
  socketId: string,
  data: string,
): { ok: boolean; error?: string } {
  const entry = SOCKETS.get(keyOf(workspaceId, socketId));
  const socket = entry?.socket ?? null;
  if (!entry || !socket || socket.readyState !== WebSocket.OPEN) {
    return { ok: false, error: "Not connected" };
  }

  const resolved = resolveVars(data, loadMergedEnvironment(workspaceId));
  try {
    socket.send(resolved);
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : "Send failed" };
  }
  push(entry, "sent", resolved);
  return { ok: true };
}

/**
 * Drop the retained log for one socket. The socket is untouched — this is `clearMessages`, not a reset.
 *
 * It is a command because the log is **engine state**: `ws.status` returns it, so a renderer-only clear would
 * be undone by the next re-attach. That is the exact class of bug this work item exists to remove.
 */
export function clearSocketMessages(
  workspaceId: string,
  socketId: string,
): { ok: boolean; error?: string } {
  const entry = SOCKETS.get(keyOf(workspaceId, socketId));
  if (!entry) {
    return { ok: false, error: `No socket "${socketId}" in workspace "${workspaceId}".` };
  }
  entry.messages.length = 0;
  return { ok: true };
}

// ── internals ─────────────────────────────────────────────────────────────────────────────────────

function push(entry: Entry, direction: "sent" | "received", data: string): void {
  const message: WsMessage = {
    id: `${entry.socketId}-${++entry.seq}`,
    ts: Date.now(),
    direction,
    data,
  };
  entry.messages.push(message);
  // Drop from the front once over the ceiling — the tail is what a re-attaching client needs, and the
  // alternative (refusing to record) would make the log stop at an arbitrary point instead.
  if (entry.messages.length > MAX_WS_MESSAGES) {
    entry.messages.splice(0, entry.messages.length - MAX_WS_MESSAGES);
  }
  entry.emit?.message(message);
}

/**
 * A copy, not the live entry — the entry holds a `WebSocket` and a set of emitters, and handing either to a
 * command's caller would let it mutate the registry's own state through the wire.
 *
 * `messages` is copied by value for the same reason, one level down: a caller that pushed onto the array it
 * was given would be editing the retained log without going through `clearSocketMessages`.
 */
function snapshotOf(entry: Entry): SocketSnapshot {
  return {
    workspaceId: entry.workspaceId,
    socketId: entry.socketId,
    url: entry.url,
    status: entry.status,
    error: entry.error,
    ...(entry.closeCode === undefined ? {} : { closeCode: entry.closeCode }),
    ...(entry.closeReason === undefined ? {} : { closeReason: entry.closeReason }),
    openedAt: entry.openedAt,
    messages: entry.messages.map((m) => ({ ...m })),
  };
}
