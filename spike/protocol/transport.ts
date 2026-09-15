/**
 * Spike 4 — transport interface.
 *
 * The only thing a client and a server agree on. P4 will implement this three
 * times (in-process, stdio, ws); the spike implements it once, in-process, over
 * the real registered IPC handlers.
 *
 * SPIKE CODE — throwaway, do not productionise.
 */
import type { RpcRequest, RpcResponse } from "./envelope";

export interface Transport {
  send(request: RpcRequest): Promise<RpcResponse>;
}
