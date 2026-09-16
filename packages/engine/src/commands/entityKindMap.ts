/**
 * P2 work item 7 — the CRUD-collapse follow-up.
 *
 * `entityCrudFactory.ts`'s internal storage `kind` string (used for on-disk paths, the
 * enabled-set files, and `entity:load`'s existing `kind` argument) does not always match the
 * frozen protocol's `EntityKind` enum (`packages/protocol/src/commands/entity.ts`). Two of the
 * twelve CRUD kinds were renamed when the protocol was designed:
 *
 *   engine-internal "rules"   <-> protocol "proxyRules"
 *   engine-internal "sockets" <-> protocol "wsConnections"
 *
 * The other ten kinds (`mappings`, `mocks`, `requests`, `webhooks`, `graphqlRequests`,
 * `graphqlMocks`, `grpcRequests`, `grpcMocks`, `soapRequests`, `soapMocks`) plus the
 * non-CRUD-factory kinds `environments`/`graphqlSchemas`/`protoFiles`/`wsdls` are identical on
 * both sides, so this map only needs to carry the two exceptions — everything else round-trips
 * unchanged.
 *
 * Note `EntitySetEnabledParams.kind` (unlike `EntityKind`) is already restricted to the six
 * literal engine-internal strings (`mocks`, `mappings`, `rules`, `graphqlMocks`, `soapMocks`,
 * `grpcMocks`) that actually carry enabled-state, so `entity.setEnabled` needs no translation at
 * all — only `entity.create`/`entity.update`/`entity.delete`/`entity.load`, which are typed
 * against the generic `EntityKind` enum, need it.
 */
const ENGINE_TO_PROTOCOL_KIND: Record<string, string> = {
  rules: "proxyRules",
  sockets: "wsConnections",
};

const PROTOCOL_TO_ENGINE_KIND: Record<string, string> = {
  proxyRules: "rules",
  wsConnections: "sockets",
};

/** Engine-internal storage kind (e.g. `"rules"`) -> protocol `EntityKind` value. */
export function toProtocolKind(engineKind: string): string {
  return ENGINE_TO_PROTOCOL_KIND[engineKind] ?? engineKind;
}

/** Protocol `EntityKind` value -> engine-internal storage kind (e.g. `"proxyRules"` -> `"rules"`). */
export function toEngineKind(protocolKind: string): string {
  return PROTOCOL_TO_ENGINE_KIND[protocolKind] ?? protocolKind;
}
