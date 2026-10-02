/**
 * `websocket/` — **the engine's live-socket registry** (`plan/19` WI 6).
 *
 * ## This file used to say the opposite, and the change is the point
 *
 * Until 2026-09-30 this module was **reserved and empty**. Its docblock said, quoting
 * `Engine_Module_Plan.md` §9: *"do not give `websocket/` invented content. There is no WebSocket server here
 * to extract, no handler to move and no state to own."* That was true then — the WebSocket *client* lived in
 * the renderer, and there was genuinely nothing engine-side to move.
 *
 * `plan/19` WI 6 is what made it false, and in the direction that matters: there **was** state to own, it was
 * just in the wrong process. `packages/gui/src/hooks/useWebSocket.ts` held a module-level `Map` of live
 * sockets, so a socket's lifetime was a browser context's — it died with the window, and no second client
 * could ever open one. Moving it here is not "giving the directory content": it is moving the content that
 * existed, which is why the reserved directory was worth reserving.
 *
 * ## What this module owns
 *
 * | file | what |
 * |---|---|
 * | `wsRegistry.ts` | the sockets, their state, and the four decisions `Protocol_Module_Contract.md` §4 requires (scope split, lifetime D29, concurrency, event completeness) |
 * | `wsCommands.ts` | the six `ws.*` commands and the bus binding |
 *
 * ## What it deliberately still does **not** own
 *
 * - **The `wsConnections` entity kind.** It stays where it was: `workspace/config.ts` emits it as stubs, the
 *   generic CRUD table routes it, and `importexport/` handles it as data. A *saved* WebSocket connection is
 *   workspace data; a *live* one is engine state. Those are different lifetimes, and this module owns only the
 *   second. Keeping the kind out of here is what stops a reconnect from being confused with a save.
 * - **`reverseproxy/edge.ts`'s `upgrade` branch.** Still reserved, still deliberately bodiless — it is a
 *   routing decision nobody has made yet, and a branch returning "not built yet" would be inventing it. §9
 *   names that explicitly as a thing not to do, and nothing in this module changes it: an outbound `ws`
 *   client and an inbound upgrade are not the same feature.
 */
export {
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
  type ConnectSocketOptions,
  type SocketEmitters,
  type SocketSnapshot,
} from "./wsRegistry";
export { registerWebsocketCommands } from "./wsCommands";
