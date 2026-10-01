import { describe, it, expect, beforeEach, vi } from "vitest";

/**
 * `ws.connect` as a request type: the saved form takes only an id, like `request.send` and its siblings.
 *
 * The registry underneath is already covered by `wsRegistry.test.ts`; what is asserted here is the one thing
 * `wsCommands.ts` adds — where the target comes from. `connectSocket` is replaced by a recorder so the
 * assertions are about *what the command asked the registry to connect to*, not about a socket.
 */
const connectSocket = vi.hoisted(() => vi.fn(() => ({ ok: true })));
vi.mock("@bifurc/engine/websocket/wsRegistry", () => ({
  MAX_WS_CONNECTIONS: 5,
  connectSocket,
  clearSocketMessages: vi.fn(),
  disconnectSocket: vi.fn(),
  getSocket: vi.fn(),
  listSockets: vi.fn(() => []),
  sendSocketMessage: vi.fn(),
}));

const saved = vi.hoisted(() => ({ entity: null as Record<string, unknown> | null }));
vi.mock("@bifurc/engine/workspace/config", () => ({
  loadEntity: (_ws: string, kind: string, id: string) =>
    kind === "sockets" && saved.entity !== null && saved.entity.id === id ? saved.entity : null,
}));

import { CommandRegistry, type CommandContext } from "@bifurc/engine/kernel/registry";
import { registerWebsocketCommands } from "@bifurc/engine/websocket/wsCommands";

const ctx = { bus: { emitTyped: vi.fn() } } as unknown as CommandContext;

function makeRegistry(): CommandRegistry {
  const registry = new CommandRegistry();
  registerWebsocketCommands(registry);
  return registry;
}

beforeEach(() => {
  connectSocket.mockClear();
  saved.entity = { id: "r1", name: "echo", url: "wss://saved.test/echo", headers: { Authorization: "Bearer {{T}}" } };
});

describe("ws.connect by saved id", () => {
  it("reads the url and headers from the saved entity when only the id is given", () => {
    makeRegistry().invoke("ws.connect", { workspaceId: "w1", socketId: "r1" }, ctx);

    expect(connectSocket).toHaveBeenCalledWith(
      expect.objectContaining({
        workspaceId: "w1",
        socketId: "r1",
        url: "wss://saved.test/echo",
        headers: { Authorization: "Bearer {{T}}" },
      }),
    );
  });

  it("uses the draft's own url and headers and never reads the store", () => {
    makeRegistry().invoke(
      "ws.connect",
      { workspaceId: "w1", socketId: "r1", url: "wss://draft.test", headers: { A: "b" } },
      ctx,
    );

    expect(connectSocket).toHaveBeenCalledWith(
      expect.objectContaining({ url: "wss://draft.test", headers: { A: "b" } }),
    );
  });

  it("reports NOT_FOUND for an id with no saved entity and no url", () => {
    expect(() =>
      makeRegistry().invoke("ws.connect", { workspaceId: "w1", socketId: "missing" }, ctx),
    ).toThrow(/No saved sockets entity "missing"/);
    expect(connectSocket).not.toHaveBeenCalled();
  });

  it("refuses headers without a url instead of half-overriding the saved request", () => {
    expect(() =>
      makeRegistry().invoke("ws.connect", { workspaceId: "w1", socketId: "r1", headers: { A: "b" } }, ctx),
    ).toThrow(/only together with `url`/);
    expect(connectSocket).not.toHaveBeenCalled();
  });
});
