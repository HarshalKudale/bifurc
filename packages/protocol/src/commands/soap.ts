/**
 * `soap.*` (network-calling half only). `soap.execute` is one of the 10 P0 spike commands —
 * confirmed working end-to-end, including the dead-endpoint path (`plan/spike-results.md`).
 */
import { z } from "zod";
import type { HttpLikeResult } from "./graphql";

const HeaderMap = z.record(z.string(), z.string());

export const SoapFetchWsdlParams = z.object({
  url: z.string(),
}).strict();
export type SoapFetchWsdlParams = z.infer<typeof SoapFetchWsdlParams>;
export interface SoapFetchWsdlResult {
  ok: boolean;
  content?: string;
  error?: string;
}

export const SoapExecuteParams = z.object({
  endpointUrl: z.string(),
  soapAction: z.string(),
  headers: HeaderMap,
  body: z.string(),
}).strict();
export type SoapExecuteParams = z.infer<typeof SoapExecuteParams>;
export type SoapExecuteResult = HttpLikeResult;
