/**
 * `request.send` — `plan/19` D24/D27's REST pipeline, on the wire.
 *
 * ## Why this file is here and not beside the other three
 *
 * The three thin counterparts (`graphql.executeSaved`, `soap.executeSaved`, `grpc.executeSaved`) live in
 * `workspace/entities/savedRequestCommands.ts`, and `request.send` used to live with them. It cannot any
 * more, and the reason is `Engine_Module_Plan.md` §5.1 rather than taste: this command calls the pipeline
 * (`pipeline/`), and `workspace/`'s §6 row is exactly `kernel/` + `utils/` — so a command that composes
 * the pipeline is a `workspace/ → pipeline/` edge, which the layering ratchet rejects. §5.1's own remedy
 * is to put the module in the layer it reads from, so the REST command lives with the pipeline it drives.
 *
 * The split is not a loss of cohesion, and the 2026-09-29 module header's argument survives it: that
 * argument was that four *identical* load-and-dispatch bodies belong in one file. Three still do. REST's
 * body is no longer identical to theirs — it is a pipeline — so the "one module, not four" reasoning stops
 * applying to exactly one member, for the reason it was written.
 *
 * ## The two input forms, and where the request comes from
 *
 * | form | request from | scripts from |
 * |---|---|---|
 * | `{ workspaceId, id }` | the stored `requests` entity | that entity's `preScript` / `postScript` / `testScript` |
 * | `{ workspaceId, request, scripts }` | the caller | the caller |
 *
 * **Nothing else differs** — that is D24's claim, and it is why one function serves both. The id form is
 * not a separate code path: it is two lines that fetch the request and the scripts, and then the same
 * call.
 *
 * ## The one transformation: `SavedRequest.body` is TEXT and `request.replay` wants BASE64
 *
 * `encodeBody()` is the engine-side equivalent of the renderer's `textToB64`, **including its
 * blank-means-empty rule** — a whitespace-only body becomes `""` rather than three spaces. That rule is
 * why this is not just `Buffer.from`: the renderer sends `""` for a whitespace-only body, so a pipeline
 * that encoded literally would make "no body" and "three spaces" different requests.
 *
 * It is the **only** encode in the pipeline, and that is `plan/19` §5's sub-decision paying off: the wire
 * carries text, so the caller does not encode and the engine does not decode; the raw primitive keeps its
 * base64 and exactly one place bridges the two. The `request.replay` below is invoked with base64 and
 * nothing else in this feature touches the boundary.
 *
 * ## Why the send goes through `registry.invoke()` rather than `replayRequest`
 *
 * `request.replay` is a one-line delegation to `utils/replay.ts`, so calling the function directly would
 * work — and it would be wrong for the same reason the 2026-09-29 counterparts dispatch through the
 * registry: the counterpart invokes the *command*, so whatever that command does today and after any
 * future change to it is what this does. It also keeps the raw primitive a real, reachable part of the
 * protocol rather than something only this file knows how to reach.
 *
 * `ctx` is threaded through, so the inner command sees the caller's session. It does not re-gate: the
 * scope check is in the transport decorator (`transport/auth/authenticated.ts`), and `request.send`
 * carries the same `execute` scope as the `request.replay` it invokes (`auth/scopes.ts`), so a session
 * that can reach the pipeline can reach its send and no widening is possible.
 *
 * ## A failed send throws, and that is the transport's problem to shape
 *
 * `runRequestPipeline` propagates a send rejection (its header says why: a tab surfaces it, a runner
 * records it and continues). Here it becomes the command's rejection, which the transport turns into an
 * error envelope. Script errors do **not** throw — they come back as `preScriptError` /
 * `postScriptError` on a successful result, because a script that threw has still produced a request and a
 * response, and the UI has always displayed them beside one.
 */
import { EngineError } from "@bifurc/protocol";
import type { RequestSendParams, RequestSendResult } from "@bifurc/protocol";
import type { CommandContext, CommandRegistry } from "../kernel/registry";
import { loadEntity } from "../workspace/config";
import type { SavedRequest } from "../workspace/types";
import { runRequestPipeline, type PipelineRequest } from "./runPipeline";
import { encodeRequestBody } from "./requestBody";

/** The stored entity's fields that the pipeline consumes, as the pipeline's own request shape. */
function requestFromEntity(entity: SavedRequest): PipelineRequest {
  const request: PipelineRequest = {
    method: entity.method,
    url: entity.url,
    headers: entity.headers ?? {},
    body: entity.body ?? "",
  };
  if (entity.bodyEncoding) request.bodyEncoding = entity.bodyEncoding;
  if (entity.bodyParts) request.bodyParts = entity.bodyParts;
  return request;
}

export function registerPipelineCommands(registry: CommandRegistry): void {
  registry.register(
    "request.send",
    async (params: RequestSendParams, ctx: CommandContext): Promise<RequestSendResult> => {
      /**
       * The id form loads first; the inline form is already in hand. `"id" in params` is the
       * discriminator rather than a truthiness test, because `RequestSendParams` is a union of two
       * `.strict()` shapes and `id` cannot appear on the inline one.
       */
      let request: PipelineRequest;
      let scripts: { pre?: string; post?: string; test?: string };

      if ("id" in params) {
        const entity = loadEntity<SavedRequest>(params.workspaceId, "requests", params.id);
        if (!entity) {
          // Thrown, not a failure-shaped value: `RequestSendResult` has no `ok` or `error` field, so a
          // `{ok:false}` would be a lie about the declared contract and the client would read
          // `status: undefined` off a successful envelope. `@bifurc/client`'s retry policy treats
          // `NOT_FOUND` as non-retryable, correctly — the entity is not going to appear on a second try.
          throw new EngineError(
            "NOT_FOUND",
            `No saved requests entity "${params.id}" in workspace "${params.workspaceId}".`,
          );
        }
        request = requestFromEntity(entity);
        scripts = {
          pre: entity.preScript,
          post: entity.postScript,
          test: entity.testScript,
        };
      } else {
        request = params.request;
        scripts = params.scripts;
      }

      const result = await runRequestPipeline({
        workspaceId: params.workspaceId,
        request,
        scripts,
        // The environment is the pipeline's to choose (`plan/19` D23) — this closure deliberately
        // ignores the `env` argument, because every field it sends has already been resolved.
        send: async (resolved) => {
          const encoded = encodeRequestBody(resolved);
          return (await registry.invoke(
            "request.replay",
            {
              method: resolved.method,
              url: resolved.url,
              headers: encoded.headers,
              body: encoded.bodyBase64,
            },
            ctx,
          )) as { status: number; headers: Record<string, string>; body: string };
        },
      });

      return {
        status: result.response.status,
        headers: result.response.headers,
        body: result.response.body,
        durationMs: result.durationMs,
        request: result.request,
        tests: result.tests,
        testLogs: result.testLogs,
        preScriptError: result.preScriptError,
        postScriptError: result.postScriptError,
      };
    },
  );
}
