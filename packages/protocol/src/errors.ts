/**
 * P1 work item 5 — error taxonomy.
 *
 * Today's handlers return ad-hoc `{ok:false, error: "some string"}` or throw. Neither survives
 * a network boundary or lets a client react differently per failure. This is the enrichment
 * layer — see the important caveat below about how it composes with the LEGACY envelope
 * semantics validated in the P0 spike (`spike/protocol/legacyTransport.ts`,
 * `plan/spike-results.md`).
 */
import { z } from "zod";

export const ErrorCode = {
  BAD_REQUEST: "BAD_REQUEST", // schema validation failed
  NOT_FOUND: "NOT_FOUND",
  CONFLICT: "CONFLICT",
  UNAUTHORIZED: "UNAUTHORIZED", // missing/invalid token
  FORBIDDEN: "FORBIDDEN", // authenticated but not permitted
  UPSTREAM_FAILED: "UPSTREAM_FAILED", // the proxied/introspected service failed
  TIMEOUT: "TIMEOUT",
  UNSUPPORTED: "UNSUPPORTED", // capability not negotiated
  UNKNOWN_COMMAND: "UNKNOWN_COMMAND",
  ENGINE_ERROR: "ENGINE_ERROR", // unhandled internal
} as const;

export type ErrorCodeValue = (typeof ErrorCode)[keyof typeof ErrorCode];

/** Codes for which the client's retry logic may safely retry the same request unmodified. */
const RETRYABLE_CODES: ReadonlySet<ErrorCodeValue> = new Set([
  ErrorCode.UPSTREAM_FAILED,
  ErrorCode.TIMEOUT,
  ErrorCode.ENGINE_ERROR,
]);

export function isRetryable(code: ErrorCodeValue): boolean {
  return RETRYABLE_CODES.has(code);
}

export const RpcErrorSchema = z.object({
  code: z.enum(Object.values(ErrorCode) as [ErrorCodeValue, ...ErrorCodeValue[]]),
  message: z.string(),
  details: z.unknown().optional(),
  retryable: z.boolean(),
});

export type RpcError = z.infer<typeof RpcErrorSchema>;

export function makeError(code: ErrorCodeValue, message: string, details?: unknown): RpcError {
  return { code, message, details, retryable: isRetryable(code) };
}

export class EngineError extends Error {
  constructor(
    public readonly code: ErrorCodeValue,
    message: string,
    public readonly details?: unknown,
  ) {
    super(message);
    this.name = "EngineError";
  }

  get retryable(): boolean {
    return isRetryable(this.code);
  }

  toRpcError(): RpcError {
    return makeError(this.code, this.message, this.details);
  }
}
