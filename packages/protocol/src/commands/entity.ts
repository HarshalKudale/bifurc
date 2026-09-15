/**
 * P1 work item 2 — the collapsed CRUD surface.
 *
 * 36 generated channels (12 entity kinds x add/update/delete) plus the pre-existing
 * `entity:load` / `entity:setEnabled` collapse to 6 generic commands. See
 * `plan/handler-classification.md` "Collapsed CRUD" for the full per-kind quirk table
 * (`unshift` vs `push`, flat vs foldered storage, enabled-state tracking, the
 * mapping->proxyRule delete cascade) — those quirks are ENGINE-side dispatch-table concerns,
 * not protocol concerns. The protocol only needs to carry `{kind, entity}` / `{kind, id}`.
 */
import { z } from "zod";

/** The 12 kinds behind the collapsed CRUD surface, from `plan/handler-classification.md`. */
export const EntityKind = z.enum([
  "mappings",
  "proxyRules",
  "mocks",
  "requests",
  "wsConnections",
  "webhooks",
  "graphqlRequests",
  "graphqlMocks",
  "grpcRequests",
  "grpcMocks",
  "soapRequests",
  "soapMocks",
  // Non-generated but same-shaped generic entity kinds (work item 1, coreHandlers.ts):
  "environments",
  "graphqlSchemas",
  "protoFiles",
  "wsdls",
]);
export type EntityKindValue = z.infer<typeof EntityKind>;

// An entity's own field shape stays `z.record` at this layer deliberately — see the P0 spike
// finding "`z.custom<T>()` is a viable escape hatch for `any`-typed renderer payloads": the wire
// envelope (kind, presence of an id, etc.) is validated; the entity's internal shape is not
// re-validated by the protocol layer, because it is already validated by the domain-specific
// `validate()` callbacks in `entityCrudFactory.ts` today. Do not let this become a habit for new
// entity kinds — see the R3 risk note in `02-phase-1-protocol.md`.
const EntityBody = z.record(z.string(), z.unknown());

export const EntityCreateParams = z.object({
  kind: EntityKind,
  entity: EntityBody,
  workspaceId: z.string().optional(),
});
export type EntityCreateParams = z.infer<typeof EntityCreateParams>;
export interface EntityCreateResult {
  id: string;
  entity: Record<string, unknown>;
}

export const EntityUpdateParams = z.object({
  kind: EntityKind,
  entity: EntityBody,
});
export type EntityUpdateParams = z.infer<typeof EntityUpdateParams>;
export interface EntityUpdateResult {
  ok: boolean;
}

export const EntityDeleteParams = z.object({
  kind: EntityKind,
  id: z.string(),
});
export type EntityDeleteParams = z.infer<typeof EntityDeleteParams>;
export interface EntityDeleteResult {
  ok: boolean;
}

export const EntityLoadParams = z.object({
  workspaceId: z.string(),
  kind: EntityKind,
  id: z.string(),
});
export type EntityLoadParams = z.infer<typeof EntityLoadParams>;
export interface EntityLoadResult {
  ok: boolean;
  entity?: Record<string, unknown>;
}

export const EntityListParams = z.object({
  workspaceId: z.string(),
  kind: EntityKind,
});
export type EntityListParams = z.infer<typeof EntityListParams>;
export interface EntityListResult {
  entities: Record<string, unknown>[];
}

/** `kind` here is the plural fs-kind string, e.g. `"mocks"` — see the spike finding about the
 * asymmetry with `folder.*`'s singular `FolderKind` enum; both are kept as-is for now. */
export const EntitySetEnabledParams = z.object({
  workspaceId: z.string(),
  kind: z.enum(["mocks", "mappings", "rules", "graphqlMocks", "soapMocks", "grpcMocks"]),
  id: z.string(),
  enabled: z.boolean(),
});
export type EntitySetEnabledParams = z.infer<typeof EntitySetEnabledParams>;
export interface EntitySetEnabledResult {
  ok: boolean;
  error?: string;
}
