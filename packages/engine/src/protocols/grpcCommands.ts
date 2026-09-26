/**
 * `grpc.execute` / `grpc.reflect` — the two surviving gRPC commands, moved out of
 * `src/ipc/handlers/grpcHandlers.ts` (P6 step 3e, slice 4).
 *
 * ## `plan/17` work item 8 removed the other three
 *
 * `grpc.mockServerStatus` / `grpc.startMockServer` / `grpc.stopMockServer` were the gRPC **mock**'s
 * serving half, and they went with the `grpcMocks` entity kind. They were stubs, but they were
 * mock-side stubs, so *"remove the gRPC mock"* takes them: with the entity kind gone there was
 * nothing left for a mock server to serve, and no component rendered their UI any more. Their two
 * shapes are recorded here rather than lost, because the asymmetry was a deliberate rendering
 * contract rather than an accident:
 *
 *  - `grpc.mockServerStatus` had **no `ok` field at all**: it resolved `{running: false, port: 9102}`.
 *    `9102` was a hardcoded constant, not the configured mock port and not derived from anything —
 *    a placeholder shape for a server that did not exist.
 *  - `grpc.stopMockServer` resolved `{ok: true}` while `grpc.startMockServer` resolved `{ok: false}`.
 *    Stopping something that was never started reported success.
 *
 * ## Why they moved (§5c)
 *
 * They were registered at **module scope** in the shell file, so a containerised engine (P9) answered
 * `UNKNOWN_COMMAND` for all five. The rest of that file — `grpc:addRequest/updateRequest/deleteRequest`
 * and `grpc:addProto/deleteProto/listProtos` — stays where it is: those route through the shell's CRUD
 * collapse (`registerEntityCrudHandlers()` / `registerSimpleEntityHandlers()`), which §5c's
 * `^commandRegistry.register(` grep never counted.
 *
 * ## The two survivors are deliberate stubs, and moving them is not an invitation to implement them
 *
 * Both resolve a fixed shape without touching the network, the filesystem or the bus — which is also
 * why this was the cheapest slice in the §5c pass: no dependency had to travel, so no importer could
 * break. The disposition is recorded in `Cleanup_plan.md` §1.5 D6 as **wontfix**, and
 * `packages/protocol/src/commands/grpc.ts` carries the same note; the engine has no `@grpc/grpc-js`
 * dependency and adding one is a product decision, not a refactor.
 *
 * The `error` strings are the user-facing explanation, so they are moved character-for-character —
 * including the package names, which are the actionable part.
 *
 * Both resolve `{ok: false, error}`. That is a **resolved value**, so the transport reports an
 * envelope **success** — "the command ran and its answer is a failure", not "the command failed".
 * Turning one into a throw would change the envelope the renderer sees.
 *
 * **The gRPC request client is untouched** — `plan/17` §3 is explicit that removing the mock does not
 * touch the client, the proto files or their folders.
 */
import type { GrpcExecuteParams, GrpcReflectParams } from "@bifurc/protocol";
import type { CommandRegistry } from "../commands/registry";

export function registerGrpcCommands(registry: CommandRegistry): void {
    registry.register("grpc.execute", async (_params: GrpcExecuteParams) => {
        return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js and @grpc/proto-loader to enable gRPC calls." };
    });

    registry.register("grpc.reflect", async (_params: GrpcReflectParams) => {
        return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js to enable server reflection." };
    });
}
