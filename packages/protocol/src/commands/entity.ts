/**
 * P1 work item 2 — the collapsed CRUD surface.
 *
 * 36 generated channels (12 entity kinds x add/update/delete) plus the pre-existing
 * `entity:load` / `entity:setEnabled` collapse to 6 generic commands. See
 * `plan/handler-classification.md` "Collapsed CRUD" for the full per-kind quirk table
 * (`unshift` vs `push`, flat vs foldered storage, enabled-state tracking, the
 * mapping->proxyRule delete cascade) — those quirks are ENGINE-side dispatch-table concerns,
 * not protocol concerns. The protocol only needs to carry `{kind, entity}` / `{kind, id}`.
 *
 * ## `plan/17` work item 8 removed the three protocol MOCK kinds
 *
 * `graphqlMocks`, `grpcMocks` and `soapMocks` are gone, and so are their folder kinds. The owner's
 * decision is **REST mocks only**: a GraphQL call and a SOAP call are both just POSTs, so either is
 * mockable as an ordinary REST mock, and the gRPC mock's serving half was a `wontfix` stub with no
 * UI left to drive it.
 *
 * **The four *request* kinds all stay** — `graphqlRequests`, `grpcRequests` and `soapRequests`
 * remain, as do `graphqlSchemas`, `protoFiles` and `wsdls`. `plan/17` §3 is explicit that the gRPC
 * request client, the proto files and their folders are **not** part of the removal.
 */
import { z } from "zod";

/** The entity kinds behind the collapsed CRUD surface, from `plan/handler-classification.md`. */
export const EntityKind = z.enum([
  "mappings",
  "proxyRules",
  "mocks",
  "requests",
  "wsConnections",
  "graphqlRequests",
  "grpcRequests",
  "soapRequests",
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
// `validate()` callbacks on the `CrudFactoryOpts` registrations in `src/ipc/handlers/*.ts` today. Do not
// let this become a habit for new entity kinds — see the R3 risk note in `02-phase-1-protocol.md`.
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
 * asymmetry with `folder.*`'s singular `FolderKind` enum; both are kept as-is for now.
 *
 * This enum is the set of kinds that carry enabled-state, so `plan/17` work item 8 narrowed it to
 * the three that remain: the three protocol mock kinds were the other members. */
export const EntitySetEnabledParams = z.object({
  workspaceId: z.string(),
  kind: z.enum(["mocks", "mappings", "rules"]),
  id: z.string(),
  enabled: z.boolean(),
});
export type EntitySetEnabledParams = z.infer<typeof EntitySetEnabledParams>;
export interface EntitySetEnabledResult {
  ok: boolean;
  error?: string;
}
