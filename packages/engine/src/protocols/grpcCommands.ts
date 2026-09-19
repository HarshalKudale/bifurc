/**
 * `grpc.execute` / `grpc.reflect` / `grpc.mockServerStatus` / `grpc.startMockServer` /
 * `grpc.stopMockServer` — the five gRPC commands, moved out of `src/ipc/handlers/grpcHandlers.ts`
 * (P6 step 3e, slice 4).
 *
 * ## Why they moved (§5c)
 *
 * They were registered at **module scope** in the shell file, so a containerised engine (P9) answered
 * `UNKNOWN_COMMAND` for all five. The rest of that file — `grpc:addRequest/updateRequest/deleteRequest`,
 * `grpc:addMock/updateMock/deleteMock` and `grpc:addProto/deleteProto/listProtos` — stays where it is:
 * those route through the shell's CRUD collapse (`registerEntityCrudHandlers()` /
 * `registerSimpleEntityHandlers()`), which §5c's `^commandRegistry.register(` grep never counted.
 *
 * ## These five are deliberate stubs, and moving them is not an invitation to implement them
 *
 * All five resolve a fixed shape without touching the network, the filesystem or the bus — which is
 * also why this was the cheapest slice in the §5c pass: no dependency had to travel, so no importer
 * could break. The disposition is recorded in `Cleanup_plan.md` §1.5 D6 as **wontfix**, and
 * `packages/protocol/src/commands/grpc.ts` carries the same note; the engine has no `@grpc/grpc-js`
 * dependency and adding one is a product decision, not a refactor.
 *
 * The `error` strings are the user-facing explanation, so they are moved character-for-character —
 * including the package names, which are the actionable part.
 *
 * ## Three shapes worth reading before "tidying" them
 *
 *  - `grpc.execute` / `grpc.reflect` / `grpc.startMockServer` resolve `{ok: false, error}`. That is a
 *    resolved value, so the transport reports an envelope **success** — "the command ran and its
 *    answer is a failure", not "the command failed". Turning one into a throw would change the
 *    envelope the renderer sees.
 *  - `grpc.mockServerStatus` has **no `ok` field at all**: it resolves `{running: false, port: 9102}`.
 *    `9102` is a hardcoded constant, not the configured mock port and not derived from anything —
 *    it is a placeholder shape for a server that does not exist. It is preserved because the renderer
 *    reads `running`/`port` off it, and because inventing a real value here would be implementing the
 *    feature.
 *  - `grpc.stopMockServer` resolves `{ok: true}` while `grpc.startMockServer` resolves `{ok: false}`.
 *    Stopping something that was never started reports success. Asymmetric, and kept: it is the
 *    shell's behaviour and the renderer's "stop" button depends on not being told it failed.
 */
import type {
    GrpcExecuteParams,
    GrpcReflectParams,
    GrpcMockServerStatusParams,
    GrpcStartMockServerParams,
    GrpcStopMockServerParams,
} from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";

export function registerGrpcCommands(registry: CommandRegistry): void {
    registry.register("grpc.execute", async (_params: GrpcExecuteParams) => {
        return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js and @grpc/proto-loader to enable gRPC calls." };
    });

    registry.register("grpc.reflect", async (_params: GrpcReflectParams) => {
        return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js to enable server reflection." };
    });

    registry.register("grpc.mockServerStatus", async (_params: GrpcMockServerStatusParams) => {
        return { running: false, port: 9102 };
    });

    registry.register("grpc.startMockServer", async (_params: GrpcStartMockServerParams) => {
        return { ok: false, error: "gRPC mock server not yet implemented. Install @grpc/grpc-js to enable." };
    });

    registry.register("grpc.stopMockServer", async (_params: GrpcStopMockServerParams) => {
        return { ok: true };
    });
}
