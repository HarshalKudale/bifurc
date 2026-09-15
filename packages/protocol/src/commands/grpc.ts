/**
 * `grpc.*` (network-calling half only). `grpc.execute`/`grpc.reflect`/`grpc.startMockServer`
 * are STUBS today — pinned by `tests/integration/protocolExecution.integration.test.ts`
 * (Cleanup_plan.md §1.5, D6 disposition: wontfix). They still get real wire command names;
 * the protocol carries the stub response through unchanged. See
 * `plan/handler-classification.md` "Open questions".
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

export const GrpcMockServerStatusParams = z.object({}).strict();
export type GrpcMockServerStatusParams = z.infer<typeof GrpcMockServerStatusParams>;
export interface GrpcMockServerStatusResult {
  running: boolean;
  port: number;
}

export const GrpcStartMockServerParams = z.object({}).strict();
export type GrpcStartMockServerParams = z.infer<typeof GrpcStartMockServerParams>;
export interface GrpcMockServerActionResult {
  ok: boolean;
  error?: string;
}

export const GrpcStopMockServerParams = z.object({}).strict();
export type GrpcStopMockServerParams = z.infer<typeof GrpcStopMockServerParams>;
