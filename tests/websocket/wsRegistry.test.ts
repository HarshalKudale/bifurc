import { describe, it, expect, beforeEach, vi } from "vitest";

/**
 * The engine's live-socket registry — `plan/19` WI 6.
 *
 * ## Why this drives the registry directly rather than through a transport
 *
 * `ws.connect` / `ws.disconnect` / `ws.send` / `ws.status` / `ws.list` / `ws.clearMessages` are a *wrapping*
 * of exactly these functions, so the four decisions `Protocol_Module_Contract.md` §4 requires are asserted
 * against the code that implements them rather than against a transport's encoding of them. The wire is
 * covered by `tests/ipc/handlers.test.ts` (every command is served) and by the protocol's own conformance
 * matrices.
 *
 * ## ⚠ The socket is faked, and the split is deliberate
 *
 * `ws` is mocked rather than a real `WebSocketServer` being started, because this file is in the **unit**
 * project and `tests/integration/` is where real ports and real sockets live (`vitest.config.ts` says so).
 * The fake is controllable in the direction the registry's logic actually depends on: it can be opened,
 * messaged, errored and closed **separately**, and `close()` records the request without firing the event.
 *
 * That separation is not a convenience — it is what makes two of the cases below possible at all, and both
 * are about orderings a real socket would collapse into one step:
 *
 * 1. **A disconnect requested while still CONNECTING.** A real client emits `error` and then `close(1006)`,
 *    which reads as a failure; the registry's `closeRequested` flag is what turns it back into "the user
 *    asked for this". Without the fake, the two arrive glued together and the flag cannot be shown to matter.
 * 2. **A superseded socket's late close.** A reconnect closes the old socket, whose close event may fire
 *    *after* the new socket has already reported `connecting`. The `generation` counter is what stops that
 *    from overwriting the new state, and it is invisible unless the events can be replayed in that order.
 */
const fake = vi.hoisted(() => {
  class FakeSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;
    static instances: FakeSocket[] = [];

    readyState = FakeSocket.CONNECTING;
    url: string;
    options: { headers?: Record<string, string> } | undefined;
    sent: string[] = [];
    /** What `close()` was asked for — recorded, and **not** fired. See the docblock. */
    closeRequest: { code?: number; reason?: string } | null = null;
    private handlers = new Map<string, (...args: unknown[]) => void>();

    constructor(url: string, options?: { headers?: Record<string, string> }) {
      // Faithful to `ws`, which throws **synchronously** on a url it cannot parse — and that is the branch
      // `connectSocket` catches to report `{ok:false, status:"error"}` rather than throwing out of the command.
      if (!/^wss?:\/\//i.test(url)) throw new Error(`Invalid URL: ${url}`);
      this.url = url;
      this.options = options;
      FakeSocket.instances.push(this);
    }

    on(event: string, cb: (...args: unknown[]) => void): this {
      this.handlers.set(event, cb);
      return this;
    }

    send(data: string): void {
      if (this.readyState !== FakeSocket.OPEN) throw new Error("WebSocket is not open");
      this.sent.push(data);
    }

    close(code?: number, reason?: string): void {
      this.closeRequest = { code, reason };
    }

    // ── drivers ────────────────────────────────────────────────────────────
    open(): void { this.readyState = FakeSocket.OPEN; this.handlers.get("open")?.(); }
    message(data: string, isBinary = false): void {
      this.handlers.get("message")?.(Buffer.from(data), isBinary);
    }
    error(message: string): void { this.handlers.get("error")?.(new Error(message)); }
    finish(code: number, reason = ""): void {
      this.readyState = FakeSocket.CLOSED;
      this.handlers.get("close")?.(code, Buffer.from(reason));
    }
  }
  return { FakeSocket };
});

vi.mock("ws", () => ({ WebSocket: fake.FakeSocket }));

/** The environment the registry resolves against. Controlled per case. */
const env = vi.hoisted(() => ({ current: null as { variables: { key: string; value: string }[] } | null }));
vi.mock("@bifurc/engine/workspace/envState", () => ({
  loadMergedEnvironment: () => env.current,
}));

import {
  MAX_WS_CONNECTIONS,
  MAX_WS_MESSAGES,
  clearSocketMessages,
  connectSocket,
  disconnectSocket,
  getSocket,
  listSockets,
  liveSocketCount,
  resetSocketsForTests,
  sendSocketMessage,
  type SocketEmitters,
} from "@bifurc/engine/websocket/wsRegistry";

const WS = "ws-1";
const OTHER_WS = "ws-2";
const ID = "tab-1";

interface Recorder {
  statuses: { status: string; error: string | null; code?: number; reason?: string }[];
  messages: { id: string; direction: string; data: string }[];
}

/** The event seam, recorded. `connectSocket` takes it per socket, exactly as `ws.connect` supplies it. */
function recorder(): SocketEmitters & Recorder {
  const statuses: Recorder["statuses"] = [];
  const messages: Recorder["messages"] = [];
  return {
    statuses,
    messages,
    status: (s) => { statuses.push(s as Recorder["statuses"][number]); },
    message: (m) => { messages.push({ id: m.id, direction: m.direction, data: m.data }); },
  };
}

/** The socket the registry most recently constructed. */
const last = () => fake.FakeSocket.instances[fake.FakeSocket.instances.length - 1];

const connect = (over: Partial<Parameters<typeof connectSocket>[0]> = {}) =>
  connectSocket({ workspaceId: WS, socketId: ID, url: "wss://example.test/socket", emit: recorder(), ...over });

beforeEach(() => {
  resetSocketsForTests();
  fake.FakeSocket.instances.length = 0;
  env.current = null;
});

describe("lifetime — D29, and the registry has no disconnect hook", () => {
  it("returns as soon as the socket is constructed, not when it opens", () => {
    const res = connect();
    expect(res.ok).toBe(true);
    expect(getSocket(WS, ID)?.status).toBe("connecting");
    expect(getSocket(WS, ID)?.openedAt).toBeNull();
  });

  it("⚠ the snapshot OUTLIVES the socket, so ws.status can answer for a closed one", () => {
    const r = recorder();
    connect({ emit: r });
    last().open();
    last().finish(1000);

    // The socket is gone…
    expect(liveSocketCount(WS)).toBe(0);
    expect(listSockets(WS)).toEqual([]);
    // …and the state is not. `null` would mean "never opened", which is a different answer.
    const snap = getSocket(WS, ID);
    expect(snap).not.toBeNull();
    expect(snap?.status).toBe("disconnected");
    expect(snap?.closeCode).toBe(1000);
  });

  it("reads code 1000 as a normal closure and anything else as an error carrying the peer's reason", () => {
    const clean = recorder();
    connect({ emit: clean });
    last().open();
    last().finish(1000);
    expect(clean.statuses.at(-1)).toMatchObject({ status: "disconnected", error: null });

    const dirty = recorder();
    connect({ socketId: "tab-2", emit: dirty });
    last().open();
    last().finish(1011, "server exploded");
    expect(dirty.statuses.at(-1)).toMatchObject({ status: "error", error: "server exploded", code: 1011 });
  });

  it("reports an abnormal close with no reason using the code, matching the renderer's fallback", () => {
    const r = recorder();
    connect({ emit: r });
    last().open();
    last().finish(1006);
    expect(r.statuses.at(-1)).toMatchObject({ status: "error", error: "Connection closed (code 1006)" });
  });

  it("emits every transition: connecting, connected, disconnected", () => {
    const r = recorder();
    connect({ emit: r });
    last().open();
    last().finish(1000);
    expect(r.statuses.map((s) => s.status)).toEqual(["connecting", "connected", "disconnected"]);
  });

  it("⚠ a disconnect requested while CONNECTING is not an error, though the client reports one", () => {
    // The reason `closeRequested` exists. `ws` emits `error` and then `close(1006)` for an aborted handshake,
    // and a user pressing Disconnect mid-connect must not be shown a failure.
    const r = recorder();
    connect({ emit: r });
    expect(disconnectSocket(WS, ID)).toEqual({ ok: true });

    last().error("WebSocket was closed before the connection was established");
    last().finish(1006);

    expect(r.statuses.map((s) => s.status)).toEqual(["connecting", "disconnected"]);
    expect(getSocket(WS, ID)?.error).toBeNull();
  });

  it("passes the close code and reason through to the client", () => {
    connect();
    disconnectSocket(WS, ID, 4000, "Going away");
    expect(last().closeRequest).toEqual({ code: 4000, reason: "Going away" });
  });

  it("answers {ok:true} for an already-closed socket and {ok:false} for one never opened", () => {
    connect();
    last().open();
    last().finish(1000);
    expect(disconnectSocket(WS, ID)).toEqual({ ok: true });

    const unknown = disconnectSocket(WS, "never-opened");
    expect(unknown.ok).toBe(false);
    expect(unknown.error).toContain("never-opened");
  });
});

describe("concurrency — one socket per {workspaceId, socketId}, and a second connect REPLACES", () => {
  it("⚠ replaces rather than refuses, and the live count does not grow", () => {
    const first = recorder();
    connect({ emit: first });
    last().open();
    const firstSocket = last();

    const second = recorder();
    connect({ emit: second });

    expect(liveSocketCount(WS)).toBe(1);
    expect(firstSocket.closeRequest).toEqual({ code: 1000, reason: "Reconnecting" });
    expect(getSocket(WS, ID)?.status).toBe("connecting");
    expect(second.statuses.map((s) => s.status)).toEqual(["connecting"]);
  });

  it("⚠ a superseded socket's late close does NOT overwrite the new socket's state", () => {
    connect();
    last().open();
    const superseded = last();

    const r = recorder();
    connect({ emit: r });
    const fresh = last();
    fresh.open();

    // The old socket's close lands *after* the new one connected — the ordering a real reconnect produces.
    superseded.finish(1006, "aborted");

    expect(r.statuses.map((s) => s.status)).toEqual(["connecting", "connected"]);
    expect(getSocket(WS, ID)?.status).toBe("connected");
  });

  it("keeps the retained log across a reconnect, as the renderer's own state did", () => {
    connect();
    last().open();
    sendSocketMessage(WS, ID, "before");

    connect();
    last().open();

    expect(getSocket(WS, ID)?.messages.map((m) => m.data)).toEqual(["before"]);
  });

  it("tracks sockets independently per workspace and per id", () => {
    connect();
    connect({ socketId: "tab-2" });
    connect({ workspaceId: OTHER_WS, socketId: ID });

    expect(liveSocketCount(WS)).toBe(2);
    expect(liveSocketCount(OTHER_WS)).toBe(1);
    expect(listSockets(WS).map((s) => s.socketId).sort()).toEqual(["tab-1", "tab-2"]);
  });
});

describe("the connection limit", () => {
  it("refuses a sixth distinct socket and names the limit", () => {
    for (let i = 0; i < MAX_WS_CONNECTIONS; i++) {
      expect(connect({ socketId: `tab-${i}` }).ok).toBe(true);
    }
    const refused = connect({ socketId: "one-too-many" });
    expect(refused.ok).toBe(false);
    expect(refused.error).toContain(`Max ${MAX_WS_CONNECTIONS} active connections reached`);
    expect(liveSocketCount(WS)).toBe(MAX_WS_CONNECTIONS);
  });

  it("⚠ a reconnect of a socket you already hold is not refused by the limit", () => {
    // The check runs before the replacement and only when the key has no live socket of its own — otherwise
    // the sixth connect would be refused by the five you are holding, including the one being replaced.
    for (let i = 0; i < MAX_WS_CONNECTIONS; i++) connect({ socketId: `tab-${i}` });
    expect(connect({ socketId: "tab-0" }).ok).toBe(true);
    expect(liveSocketCount(WS)).toBe(MAX_WS_CONNECTIONS);
  });

  it("does not count a closed socket against the limit", () => {
    for (let i = 0; i < MAX_WS_CONNECTIONS; i++) connect({ socketId: `tab-${i}` });
    last().open();
    last().finish(1000);

    expect(connect({ socketId: "fresh" }).ok).toBe(true);
  });

  it("counts per workspace, so one workspace cannot exhaust another's", () => {
    for (let i = 0; i < MAX_WS_CONNECTIONS; i++) connect({ socketId: `tab-${i}` });
    expect(connect({ workspaceId: OTHER_WS, socketId: "x" }).ok).toBe(true);
  });
});

describe("resolution is engine-side (D23)", () => {
  it("resolves the url and every header VALUE against the merged environment", () => {
    env.current = { variables: [{ key: "HOST", value: "resolved.test" }, { key: "TOKEN", value: "s3cret" }] };
    connect({ url: "wss://{{HOST}}/chat", headers: { Authorization: "Bearer {{TOKEN}}", "X-Static": "plain" } });

    expect(last().url).toBe("wss://resolved.test/chat");
    // The renderer could not do this at all: the browser WebSocket constructor cannot send headers, so a
    // resolved Authorization header used to be resolved and discarded. The engine's client can.
    expect(last().options?.headers).toEqual({ Authorization: "Bearer s3cret", "X-Static": "plain" });
  });

  it("leaves the caller's url verbatim on the snapshot, so it can be shown back", () => {
    env.current = { variables: [{ key: "HOST", value: "resolved.test" }] };
    connect({ url: "wss://{{HOST}}/chat" });
    expect(getSocket(WS, ID)?.url).toBe("wss://{{HOST}}/chat");
  });

  it("resolves an outgoing frame at send time", () => {
    env.current = { variables: [{ key: "USER", value: "alice" }] };
    connect();
    last().open();
    sendSocketMessage(WS, ID, '{"user":"{{USER}}"}');

    expect(last().sent).toEqual(['{"user":"alice"}']);
    // The log records what was written, not what was asked for — the resolved text.
    expect(getSocket(WS, ID)?.messages.at(-1)?.data).toBe('{"user":"alice"}');
  });

  it("trims the url the way the renderer did", () => {
    connect({ url: "  wss://example.test/socket  " });
    expect(last().url).toBe("wss://example.test/socket");
  });
});

describe("the message log", () => {
  it("records both directions with an engine-minted id and an engine-side timestamp", () => {
    const r = recorder();
    connect({ emit: r });
    last().open();
    last().message("from the peer");
    sendSocketMessage(WS, ID, "to the peer");

    const snap = getSocket(WS, ID)!;
    expect(snap.messages.map((m) => [m.direction, m.data])).toEqual([
      ["received", "from the peer"],
      ["sent", "to the peer"],
    ]);
    // Distinct ids, because a re-attaching client de-duplicates by id — "the same frame twice" and "two
    // identical frames" have to be tellable apart.
    expect(new Set(snap.messages.map((m) => m.id)).size).toBe(2);
    expect(snap.messages.every((m) => typeof m.ts === "number" && m.ts > 0)).toBe(true);
  });

  it("labels a binary frame rather than decoding it, using the client's own isBinary", () => {
    connect();
    last().open();
    last().message("\x00\x01\x02", true);
    expect(getSocket(WS, ID)?.messages.at(-1)?.data).toBe("[binary data]");
  });

  it("emits a message event per frame, carrying the whole message", () => {
    const r = recorder();
    connect({ emit: r });
    last().open();
    last().message("hi");
    expect(r.messages).toEqual([{ id: `${ID}-1`, direction: "received", data: "hi" }]);
  });

  it(`retains only the last ${MAX_WS_MESSAGES} frames — the bound is a decision, not a default`, () => {
    // An unbounded per-socket log would be a leak in a process that now outlives every client, which is the
    // whole point of D29. The tail is what a re-attaching client needs.
    connect();
    last().open();
    for (let i = 0; i < MAX_WS_MESSAGES + 25; i++) last().message(`m${i}`);

    const messages = getSocket(WS, ID)!.messages;
    expect(messages).toHaveLength(MAX_WS_MESSAGES);
    expect(messages[0].data).toBe("m25");
    expect(messages.at(-1)?.data).toBe(`m${MAX_WS_MESSAGES + 24}`);
  });

  it("clears the log without touching the socket", () => {
    connect();
    last().open();
    last().message("hi");

    expect(clearSocketMessages(WS, ID)).toEqual({ ok: true });
    expect(getSocket(WS, ID)?.messages).toEqual([]);
    expect(getSocket(WS, ID)?.status).toBe("connected");
    expect(liveSocketCount(WS)).toBe(1);
  });

  it("refuses to clear a socket that was never opened", () => {
    const res = clearSocketMessages(WS, "never-opened");
    expect(res.ok).toBe(false);
    expect(res.error).toContain("never-opened");
  });

  it("returns copies, so a caller cannot edit the registry through the wire", () => {
    connect();
    last().open();
    last().message("hi");

    const first = getSocket(WS, ID)!;
    first.messages.push({ id: "forged", ts: 0, direction: "sent", data: "nope" });
    first.messages[0].data = "tampered";

    const second = getSocket(WS, ID)!;
    expect(second.messages).toHaveLength(1);
    expect(second.messages[0].data).toBe("hi");
  });
});

describe("sending", () => {
  it('answers {ok:false, error:"Not connected"} when nothing is open, rather than throwing', () => {
    connect();
    expect(sendSocketMessage(WS, ID, "x")).toEqual({ ok: false, error: "Not connected" });
  });

  it("refuses to send on a socket that has closed", () => {
    connect();
    last().open();
    last().finish(1000);
    expect(sendSocketMessage(WS, ID, "x").ok).toBe(false);
  });

  it("refuses to send to a socket that was never opened", () => {
    expect(sendSocketMessage(WS, "never-opened", "x").ok).toBe(false);
  });

  it("does not log a frame it did not write", () => {
    connect();
    last().open();
    // Force the registry's own gate to refuse: `sendSocketMessage` checks `readyState` before calling the
    // client, so the log and the write cannot disagree — a frame in the transcript that never left would be
    // the same class of lie as a frame the client rejected being recorded as sent.
    last().readyState = fake.FakeSocket.CONNECTING;
    expect(sendSocketMessage(WS, ID, "x").ok).toBe(false);
    expect(getSocket(WS, ID)?.messages).toEqual([]);
  });
});

describe("a malformed url", () => {
  it("is reported as an error state rather than thrown out of the command", () => {
    const r = recorder();
    const res = connect({ url: "not-a-url", emit: r });
    expect(res.ok).toBe(false);
    expect(res.error).toBeTruthy();
    expect(getSocket(WS, ID)?.status).toBe("error");
    expect(r.statuses.map((s) => s.status)).toEqual(["connecting", "error"]);
  });
});
