/**
 * Spike 4 — the 10 representative commands.
 *
 * The 10 chosen per `plan/01-phase-0-derisk.md`: at least one mutating, one
 * network-calling, one long-running, one error-producing, one privileged.
 *
 * Two deliberate design decisions under test here:
 *
 * 1. **Params are object-shaped on the wire, positional at the client.**
 *    The legacy IPC channels take positional args (`ipcRenderer.invoke("entity:setEnabled",
 *    wsId, kind, id, enabled)`), while a schema wants named fields. Each command therefore
 *    carries `toArgs` — the adapter that turns the validated payload back into the
 *    positional call the existing handler expects. This is the "compatibility shim" P5
 *    talks about, and its per-command cost is the thing this spike measures.
 *
 * 2. **Results are typed, not schema-validated.** The existing handlers return
 *    heterogeneous shapes (`config:get` → bare AppConfig, `mock:add` → the entity,
 *    `entity:setEnabled` → `{ok, error?}`). The renderer already tolerates all of them,
 *    so the protocol passes them through untouched and carries only the TS type. See
 *    `plan/spike-results.md` for why validating results would break the
 *    zero-renderer-change thesis.
 *
 * SPIKE CODE — throwaway, do not productionise.
 */
import { z } from "zod";
import type { AppConfig } from "../../renderer/types/config";
import type { Folder, MockRule } from "../../renderer/types/entities";

export interface CommandDef<P, R> {
  /** Wire action name, `namespace.verb` per the P1 naming convention. */
  readonly action: string;
  /** Strict schema for the wire payload. */
  readonly params: z.ZodType<P>;
  /** Adapter back to the positional args the legacy handler takes. */
  readonly toArgs: (params: P) => unknown[];
  /** Phantom, type-level only — never read at runtime. */
  readonly resultType?: R;
}

function cmd<P, R>(action: string, params: z.ZodType<P>, toArgs: (p: P) => unknown[]): CommandDef<P, R> {
  return { action, params, toArgs };
}

/* eslint-disable @typescript-eslint/no-explicit-any */

/**
 * The 10 commands. `R` is declared explicitly from the renderer's own types —
 * the shape proof in `shape.ts` then verifies the client built from these
 * satisfies `Pick<BifurcApi, …>` exactly.
 */
export const commands = {
  // read
  "config.get": cmd<Record<string, never>, AppConfig>(
    "config.get", z.object({}).strict(), () => []),
  "server.status": cmd<Record<string, never>, { running: boolean; port: number; error: string | null }>(
    "server.status", z.object({}).strict(), () => []),

  // mutate
  "mock.add": cmd<{ mock: Omit<MockRule, "id" | "createdAt" | "workspaceId"> }, MockRule>(
    "mock.add",
    z.object({ mock: z.custom<Omit<MockRule, "id" | "createdAt" | "workspaceId">>() }).strict(),
    (p) => [p.mock]),
  "mock.delete": cmd<{ id: string }, { ok: boolean }>(
    "mock.delete",
    z.object({ id: z.string() }).strict(),
    (p) => [p.id]),
  "folder.add": cmd<{ kind: string; folder: Omit<Folder, "id" | "createdAt" | "workspaceId"> }, Folder>(
    "folder.add",
    z.object({ kind: z.string(), folder: z.custom<Omit<Folder, "id" | "createdAt" | "workspaceId">>() }).strict(),
    (p) => [p.kind, p.folder]),
  "folder.move": cmd<{ kind: string; id: string; parentId: string | null }, { ok: boolean }>(
    "folder.move",
    z.object({ kind: z.string(), id: z.string(), parentId: z.string().nullable() }).strict(),
    (p) => [p.kind, p.id, p.parentId]),
  "entity.setEnabled": cmd<{ wsId: string; kind: string; id: string; enabled: boolean }, { ok: boolean; error?: string }>(
    "entity.setEnabled",
    z.object({ wsId: z.string(), kind: z.string(), id: z.string(), enabled: z.boolean() }).strict(),
    (p) => [p.wsId, p.kind, p.id, p.enabled]),

  // network-calling / long-running
  "graphql.introspect": cmd<{ url: string; headers: Record<string, string> }, { ok: boolean; sdl?: string; error?: string }>(
    "graphql.introspect",
    z.object({ url: z.string(), headers: z.record(z.string(), z.string()) }).strict(),
    (p) => [{ url: p.url, headers: p.headers }]),
  "soap.execute": cmd<
    { endpointUrl: string; soapAction: string; headers: Record<string, string>; body: string },
    { status: number; headers: Record<string, string>; body: string; durationMs: number }
  >(
    "soap.execute",
    z.object({
      endpointUrl: z.string(),
      soapAction: z.string(),
      headers: z.record(z.string(), z.string()),
      body: z.string(),
    }).strict(),
    (p) => [{ endpointUrl: p.endpointUrl, soapAction: p.soapAction, headers: p.headers, body: p.body }]),

  // privileged
  "tls.generate": cmd<Record<string, never>, { ok: boolean; certPath?: string; keyPath?: string; error?: string }>(
    "tls.generate", z.object({}).strict(), () => []),
} as const;

export type CommandAction = keyof typeof commands;

/** The legacy IPC channel each action dispatches to. */
export const LEGACY_CHANNEL: Record<CommandAction, string> = {
  "config.get": "config:get",
  "server.status": "server:status",
  "mock.add": "mock:add",
  "mock.delete": "mock:delete",
  "folder.add": "folder:add",
  "folder.move": "folder:move",
  "entity.setEnabled": "entity:setEnabled",
  "graphql.introspect": "graphql:introspect",
  "soap.execute": "soap:execute",
  "tls.generate": "tls:generate",
};

export function isKnownAction(action: string): action is CommandAction {
  return Object.prototype.hasOwnProperty.call(commands, action);
}
