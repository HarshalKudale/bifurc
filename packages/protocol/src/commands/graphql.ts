/**
 * `graphql.*` (network-calling half only — schema CRUD collapses into `entity.*`).
 * `graphql.introspect` is one of the 10 P0 spike commands — confirmed working end-to-end
 * against the real handler, including the dead-endpoint path (see `plan/spike-results.md`).
 */
import { z } from "zod";

const HeaderMap = z.record(z.string(), z.string());

export const GraphqlIntrospectParams = z.object({
  url: z.string(),
  headers: HeaderMap,
}).strict();
export type GraphqlIntrospectParams = z.infer<typeof GraphqlIntrospectParams>;
export interface GraphqlIntrospectResult {
  ok: boolean;
  sdl?: string;
  error?: string;
}

export const GraphqlExecuteParams = z.object({
  url: z.string(),
  headers: HeaderMap,
  query: z.string(),
  variables: z.string(),
  operationName: z.string(),
}).strict();
export type GraphqlExecuteParams = z.infer<typeof GraphqlExecuteParams>;
export interface HttpLikeResult {
  status: number;
  headers: Record<string, string>;
  body: string;
  durationMs: number;
}
