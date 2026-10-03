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
 *
 * ## `grpc.send` — the fourth command, and the last of `plan/19` WI 3 (2026-09-30)
 *
 * It replaced `grpc.executeSaved`, whose params were `{workspaceId, id}`; the new params are a **union** of
 * that form and `{workspaceId, request}`, so a draft tab and the CLI use one command. With it, **all four
 * request-executing protocols are migrated**, and `workspace/entities/savedRequestCommands.ts` — the module
 * that used to hold four load-and-dispatch bodies — is **deleted**.
 *
 * **It resolves every field and runs no scripts**, the same shape as `graphql.send` and `soap.send`:
 *
 * - *Every field* — `serverAddress`, `serviceName`, `methodName`, `requestBody` **and every `metadata`
 *   value**, against the workspace's merged environment (`workspace/envState.ts`, D23). The tab resolved
 *   `serverAddress`, `requestBody` and the metadata values but **not `serviceName` or `methodName`**, while
 *   the Send gate's token scan treated all five as resolvable — so a `{{token}}` in either never resolved in
 *   *either* path. The gate was written from intent and the tab from practice, and the engine follows the
 *   gate. `protoFileId` is deliberately **not** resolved: it names a stored file rather than being template
 *   text, and the gate never scanned it.
 * - *No scripts* — the sandbox's context is a `{method, url, headers, body}` request and a gRPC call is not
 *   one, so `grpc.send` has no `scripts` member rather than one nothing could run.
 *
 * **⚠ The stub makes this one's resolution UNOBSERVABLE, and that is a recorded gap rather than an
 * oversight.** `grpc.execute` ignores every parameter it is handed and answers `{ok:false, error}`, so unlike
 * its three siblings there is no upstream at which a resolved field could be seen — an integration case could
 * assert the stub's message and nothing about resolution. The only way to observe it today would be to
 * replace `grpc.execute` with an echo, and `CommandRegistry.register()` **throws on a duplicate**, so that
 * needs a monkey-patched registry inside a test: a worse trade than the gap, given that the resolution here is
 * the same shape as `soap.send`'s and *that* one is verified end to end field by field.
 *
 * **What this command does prove is the load half** — the `NOT_FOUND` case is what shows the entity was read,
 * and it is the only thing that does, since the stub ignores everything else. When the stub is implemented,
 * the resolution starts mattering and an integration case should be added with it.
 */
import { EngineError } from "@bifurc/protocol";
import type { GrpcExecuteParams, GrpcReflectParams, GrpcSendParams } from "@bifurc/protocol";
import type { CommandContext, CommandRegistry } from "../kernel/registry";
import { loadEntity } from "../workspace/config";
import { loadMergedEnvironment } from "../env/envState";
import { resolveVars } from "../env/resolve";

export function registerGrpcCommands(registry: CommandRegistry): void {
    registry.register("grpc.execute", async (_params: GrpcExecuteParams) => {
        return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js and @grpc/proto-loader to enable gRPC calls." };
    });

    registry.register("grpc.reflect", async (_params: GrpcReflectParams) => {
        return { ok: false, error: "gRPC runtime not yet configured. Install @grpc/grpc-js to enable server reflection." };
    });

    // Live calls are Rust-engine only, like the two stubs above.
    for (const action of ["grpc.stream.open", "grpc.stream.send", "grpc.stream.end", "grpc.stream.cancel"] as const) {
        registry.register(action, async () => ({ ok: false, error: "Live gRPC calls need the Rust engine." }));
    }

    registry.register("grpc.send", async (params: GrpcSendParams, ctx: CommandContext) => {
        const stored = "id" in params
            ? requireStoredGrpcRequest(params.workspaceId, params.id)
            : params.request;
        // One read of the workspace's environment state for the whole call. `plan/19` D23: the engine is the
        // authority for which environment a request resolves against, and the merge is part of that.
        const env = loadMergedEnvironment(params.workspaceId);
        const metadata: Record<string, string> = {};
        for (const [k, v] of Object.entries(stored.metadata ?? {})) metadata[k] = resolveVars(v, env);
        return registry.invoke(
            "grpc.execute",
            {
                // **Every** scanned field is resolved — including `serviceName` and `methodName`, which the
                // tab never resolved while the Send gate treated them as resolvable (see the header).
                serverAddress: resolveVars(stored.serverAddress, env),
                serviceName: resolveVars(stored.serviceName, env),
                methodName: resolveVars(stored.methodName, env),
                requestBody: resolveVars(stored.requestBody, env),
                metadata,
                // Deliberately **not** resolved: it names a stored proto file rather than being template
                // text, and the Send gate never scanned it.
                protoFileId: stored.protoFileId ?? null,
                useReflection: stored.useReflection ?? false,
            },
            ctx,
        );
    });
}

/**
 * The stored entity's fields, as a **projection** — exactly the ones the call reads.
 *
 * Declared *after* its only reader, which is legal for a type and keeps this file's shape "the three
 * commands, then what they read". The canonical declaration lives beside the `grpcRequests` kind in
 * `grpc/kinds.ts` and is not re-exported, because nothing outside those two files had ever needed it. (This
 * projection lived in `workspace/entities/savedRequestCommands.ts` until 2026-09-30 — that module is now
 * deleted, and this was the last projection it held.)
 */
interface StoredGrpcRequest {
    serverAddress: string;
    serviceName: string;
    methodName: string;
    requestBody: string;
    metadata?: Record<string, string>;
    protoFileId?: string | null;
    useReflection?: boolean;
}

/**
 * Load the entity or fail loudly, rather than answering a failure-shaped value.
 *
 * `GrpcExecuteResult` **does** declare `ok` and `error`, so unlike its three siblings a `{ok:false}` result
 * here would not be a contract lie — but it would be a **different** one: `{ok:false, error}` is what the
 * stub returns for "the runtime is not configured", and a missing entity reported the same way would be
 * indistinguishable from it. Throwing keeps the two apart, and the transport preserves the code.
 */
function requireStoredGrpcRequest(workspaceId: string, id: string): StoredGrpcRequest {
    const entity = loadEntity<StoredGrpcRequest>(workspaceId, "grpcRequests", id);
    if (!entity) {
        throw new EngineError(
            "NOT_FOUND",
            `No saved grpcRequests entity "${id}" in workspace "${workspaceId}".`,
        );
    }
    return entity;
}
