/**
 * `grpc.*` (network-calling half only). `grpc.execute` / `grpc.reflect` are STUBS today — pinned by
 * `tests/integration/protocolExecution.integration.test.ts` (Cleanup_plan.md §1.5, D6 disposition:
 * wontfix). They still get real wire command names; the protocol carries the stub response through
 * unchanged. See `plan/handler-classification.md` "Open questions".
 *
 * ## `plan/17` work item 8 removed the three mock-server commands
 *
 * `grpc.mockServerStatus` / `grpc.startMockServer` / `grpc.stopMockServer` were the gRPC **mock**'s
 * serving half. They were the same `wontfix` stubs as the two above, but they are mock-side and so
 * went with the `grpcMocks` entity kind — with that kind gone they had nothing left to serve, and
 * nothing rendered their UI (only the `protocols.ts` labels and the `window.api` declarations
 * remained). **The gRPC *request* client is untouched**, per `plan/17` §3.
 */
import { z } from "zod";

const HeaderMap = z.record(z.string(), z.string());

export const GrpcExecuteParams = z.object({
  serverAddress: z.string(),
  serviceName: z.string(),
  methodName: z.string(),
  requestBody: z.string(),
  metadata: HeaderMap,
  protoFileId: z.string().nullable(),
  useReflection: z.boolean(),
  workspaceId: z.string().optional(),
}).strict();
export type GrpcExecuteParams = z.infer<typeof GrpcExecuteParams>;
export interface GrpcExecuteResult {
  ok: boolean;
  responses?: string[];
  metadata?: Record<string, string>;
  status?: number;
  statusMessage?: string;
  durationMs?: number;
  error?: string;
}

export const GrpcReflectParams = z.object({
  serverAddress: z.string(),
  protoFileId: z.string().nullable().optional(),
  workspaceId: z.string().optional(),
}).strict();
export type GrpcReflectParams = z.infer<typeof GrpcReflectParams>;
export interface GrpcMethodInfo {
  name: string;
  inputType: string;
  outputType: string;
  clientStreaming: boolean;
  serverStreaming: boolean;
}
export interface GrpcServiceInfo {
  name: string;
  methods: GrpcMethodInfo[];
}
export interface GrpcReflectResult {
  ok: boolean;
  services?: GrpcServiceInfo[];
  error?: string;
}

/**
 * Live gRPC calls. `grpc.send` runs a call to its end and answers once; these keep one open, the way the
 * socket commands keep a WebSocket open: the caller names the stream (`streamId`), the messages and the end
 * of the call arrive as `event.grpc.stream.message` / `event.grpc.stream.status`.
 *
 * `open` starts a saved request. A method that takes one message sends the request's body when it starts; a
 * method that streams from the client starts with nothing sent, and `send` / `end` supply the rest.
 */
export const GrpcStreamOpenParams = z.object({
  workspaceId: z.string(),
  streamId: z.string(),
  id: z.string(),
}).strict();
export type GrpcStreamOpenParams = z.infer<typeof GrpcStreamOpenParams>;

export const GrpcStreamSendParams = z.object({
  workspaceId: z.string(),
  streamId: z.string(),
  data: z.string(),
}).strict();
export type GrpcStreamSendParams = z.infer<typeof GrpcStreamSendParams>;

export const GrpcStreamRefParams = z.object({
  workspaceId: z.string(),
  streamId: z.string(),
}).strict();
export type GrpcStreamRefParams = z.infer<typeof GrpcStreamRefParams>;

export interface GrpcStreamOpenResult {
  ok: boolean;
  /** How the method streams: `unary`, `server`, `client` or `bidi`. */
  kind?: "unary" | "server" | "client" | "bidi";
  error?: string;
}
