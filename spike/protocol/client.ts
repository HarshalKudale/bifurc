/**
 * Spike 4 — typed RPC client.
 *
 * THE WHOLE THESIS OF P0/P5 LIVES IN THIS FILE.
 *
 * Every method below is a **positional, `window.api`-shaped** signature. The
 * renderer calls `window.api.setEntityEnabled(wsId, kind, id, enabled)` today;
 * after the seam it calls exactly the same thing on this client. The object-shaped
 * wire payload is an internal detail the renderer never sees.
 *
 * `shape.ts` proves at compile time that this interface is mutually assignable
 * with `Pick<BifurcApi, …>` for the 10 spike methods. If that file compiles, the
 * renderer needs zero changes for these commands.
 *
 * SPIKE CODE — throwaway, do not productionise.
 */
import { z } from "zod";
import type { AppConfig } from "../../renderer/types/config";
import type { Folder, MockRule } from "../../renderer/types/entities";
import type { RpcRequest, RpcResponse } from "./envelope";
import type { Transport } from "./transport";
import { commands, type CommandAction, type CommandDef } from "./commands";

export class ProtocolClientError extends Error {
  constructor(
    public readonly code: "UNKNOWN_COMMAND" | "INVALID_PARAMS" | "INTERNAL" | string,
    message: string,
  ) {
    super(`[${code}] ${message}`);
    this.name = "ProtocolClientError";
  }
}

/** Errors the legacy handlers report as *resolved* `{ok:false, error}` values stay resolved. */
export interface ProtocolClient {
  // read
  getConfig(): Promise<AppConfig>;
  serverStatus(): Promise<{ running: boolean; port: number; error: string | null }>;
  // mutate
  addMock(mock: Omit<MockRule, "id" | "createdAt" | "workspaceId">): Promise<MockRule>;
  deleteMock(id: string): Promise<{ ok: boolean }>;
  addFolder(
    kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "graphqlMock" | "grpcRequest" | "grpcMock" | "soapRequest" | "soapMock",
    folder: Omit<Folder, "id" | "createdAt" | "workspaceId">,
  ): Promise<Folder>;
  moveFolder(
    kind: "mock" | "request" | "ws" | "webhook" | "rule" | "graphqlRequest" | "graphqlMock" | "grpcRequest" | "grpcMock" | "soapRequest" | "soapMock",
    id: string,
    parentId: string | null,
  ): Promise<{ ok: boolean }>;
  setEntityEnabled(wsId: string, kind: string, id: string, enabled: boolean): Promise<{ ok: boolean; error?: string }>;
  // network / long-running
  graphqlIntrospect(endpointUrl: string, headers: Record<string, string>): Promise<{ ok: boolean; sdl?: string; error?: string }>;
  soapExecute(
    endpointUrl: string,
    soapAction: string,
    headers: Record<string, string>,
    body: string,
  ): Promise<{ status: number; headers: Record<string, string>; body: string; durationMs: number }>;
  // privileged
  tlsGenerate(): Promise<{ ok: boolean; certPath?: string; keyPath?: string; error?: string }>;
}

let nextId = 0;

async function call<P, R>(transport: Transport, action: CommandAction, payload: P, def: CommandDef<P, R>): Promise<R> {
  // Client-side validation: catch programming errors before they hit the wire.
  const parsed = def.params.safeParse(payload);
  if (!parsed.success) {
    throw new ProtocolClientError("INVALID_PARAMS", z.prettifyError(parsed.error));
  }
  const request: RpcRequest = { id: `c${++nextId}`, action, payload: parsed.data };
  const response: RpcResponse = await transport.send(request);
  if (!response.ok) {
    throw new ProtocolClientError(response.error.code, response.error.message);
  }
  return response.data as R;
}

export function createProtocolClient(transport: Transport): ProtocolClient {
  return {
    // read
    getConfig: () => call(transport, "config.get", {}, commands["config.get"]),
    serverStatus: () => call(transport, "server.status", {}, commands["server.status"]),
    // mutate
    addMock: (mock) => call(transport, "mock.add", { mock }, commands["mock.add"]),
    deleteMock: (id) => call(transport, "mock.delete", { id }, commands["mock.delete"]),
    addFolder: (kind, folder) => call(transport, "folder.add", { kind, folder }, commands["folder.add"]),
    moveFolder: (kind, id, parentId) => call(transport, "folder.move", { kind, id, parentId }, commands["folder.move"]),
    setEntityEnabled: (wsId, kind, id, enabled) =>
      call(transport, "entity.setEnabled", { wsId, kind, id, enabled }, commands["entity.setEnabled"]),
    // network / long-running
    graphqlIntrospect: (endpointUrl, headers) =>
      call(transport, "graphql.introspect", { url: endpointUrl, headers }, commands["graphql.introspect"]),
    soapExecute: (endpointUrl, soapAction, headers, body) =>
      call(transport, "soap.execute", { endpointUrl, soapAction, headers, body }, commands["soap.execute"]),
    // privileged
    tlsGenerate: () => call(transport, "tls.generate", {}, commands["tls.generate"]),
  };
}
