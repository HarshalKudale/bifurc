/**
 * The `ws.*` commands — `plan/19` WI 6.
 *
 * Thin by construction: every decision lives in `wsRegistry.ts`, and this file's whole job is to bind the
 * registry to the wire and to the bus. That shape is `runner/runnerCommands.ts`'s, and it is available here
 * for the same reason — the state module is testable without a registry, and the command module has nothing in
 * it that could disagree with the state module about what the rules are.
 *
 * ## Why the bus is `ctx.bus` and not the singleton
 *
 * A headless host builds its own registry **and** its own bus. Reaching for the `kernel/eventBus` singleton
 * here would deliver a socket's events to a bus nobody is listening to — which is the trap
 * `savedRequestCommands.ts` recorded for the registry before it was deleted, one layer over. So the bus is
 * injected through the command context, and the registry takes the emitters it is handed.
 *
 * ## Why `ws.connect` returns before the socket is open
 *
 * `connectSocket` resolves as soon as the `WebSocket` has been *constructed*. The transition to `connected`
 * arrives as `event.ws.status`, and `ws.status` answers for the state in between — the same shape D29 gave
 * `runner.start`, and for the same reason: a caller that had to await the open would be a caller holding the
 * connection's lifetime, which is exactly what moved to the engine.
 */
import type {
    WsClearMessagesParams,
    WsConnectParams,
    WsDisconnectParams,
    WsListParams,
    WsListResult,
    WsOkResult,
    WsSendParams,
    WsStatusParams,
    WsStatusResult,
} from "@bifurc/protocol";
import type { CommandContext, CommandRegistry } from "../kernel/registry";
import {
    MAX_WS_CONNECTIONS,
    clearSocketMessages,
    connectSocket,
    disconnectSocket,
    getSocket,
    listSockets,
    sendSocketMessage,
    type SocketEmitters,
} from "./wsRegistry";

export function registerWebsocketCommands(registry: CommandRegistry): void {
    /**
     * `ws.connect` — open or re-open a socket, named by the caller.
     *
     * The emitters are built here, per call, and **capture `params`**: every event a socket emits belongs to
     * one `{workspaceId, socketId}`, and the bus is shared by every socket in the process. A subscriber that
     * could not tell which socket an event came from could only guess — and the renderer has one subscriber
     * per open tab, all listening to the same two events.
     */
    registry.register("ws.connect", (params: WsConnectParams, ctx: CommandContext): WsOkResult => {
        const emitters: SocketEmitters = {
            status: (state) => {
                ctx.bus.emitTyped("ws.status", {
                    workspaceId: params.workspaceId,
                    socketId: params.socketId,
                    status: state.status,
                    error: state.error,
                    ...(state.code === undefined ? {} : { code: state.code }),
                    ...(state.reason === undefined ? {} : { reason: state.reason }),
                });
            },
            message: (message) => {
                ctx.bus.emitTyped("ws.message", {
                    workspaceId: params.workspaceId,
                    socketId: params.socketId,
                    message,
                });
            },
        };

        return connectSocket({
            workspaceId: params.workspaceId,
            socketId: params.socketId,
            url: params.url,
            headers: params.headers,
            emit: emitters,
        });
    });

    /** `ws.disconnect` — the only lever that closes a socket, which is D29 stated as code. */
    registry.register("ws.disconnect", (params: WsDisconnectParams): WsOkResult =>
        disconnectSocket(params.workspaceId, params.socketId, params.code, params.reason),
    );

    /** `ws.send` — resolve engine-side, then write. See `wsRegistry` for why `{{random.*}}` re-mints here. */
    registry.register("ws.send", (params: WsSendParams): WsOkResult =>
        sendSocketMessage(params.workspaceId, params.socketId, params.data),
    );

    /**
     * `ws.status` — the snapshot, or `null`.
     *
     * **Returned as-is, and that is the point.** `SocketSnapshot` and the protocol's `WsStatusResult` are
     * declared twice on purpose — the wire shape has to be readable by a browser and this module is Node —
     * and this return type is what keeps them in step: a field added to one and not the other is a compile
     * error here rather than a silently dropped field on the way out. That is the failure mode
     * `RunnerConfigSchema`'s note records this codebase already paying for once, which is why there is no cast
     * on this line.
     */
    registry.register("ws.status", (params: WsStatusParams): WsStatusResult | null =>
        getSocket(params.workspaceId, params.socketId),
    );

    /**
     * `ws.list` — the **live** sockets, plus the registry's limit.
     *
     * `max` travels with the list rather than being mirrored into the UI: the registry is what refuses the
     * sixth connection, so the registry is what knows the number. A pinned `5` in the renderer would be the
     * D28 list-vs-logic split spent on a constant the engine can simply answer — and would have been wrong the
     * day the limit became configurable.
     */
    registry.register("ws.list", (params: WsListParams): WsListResult => ({
        connections: listSockets(params.workspaceId),
        max: MAX_WS_CONNECTIONS,
    }));

    /** `ws.clearMessages` — drop the retained log. The socket is untouched. */
    registry.register("ws.clearMessages", (params: WsClearMessagesParams): WsOkResult =>
        clearSocketMessages(params.workspaceId, params.socketId),
    );
}
