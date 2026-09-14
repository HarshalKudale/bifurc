/**
 * GraphQL / SOAP / gRPC tab reducers.
 *
 * WHY THIS EXISTS
 * ---------------
 * These three are the state machines behind the request and mock editors for the
 * non-REST protocols. They were at 0% coverage: `restTabReducer` had a suite, its three
 * siblings had none. They are pure functions, so they are cheap to pin exactly — and
 * pinning them immediately surfaced a real bug.
 *
 * The bug: `createTabReducer` (the shared layer all four reducers are built on) handled
 * only the save/send cases. `REFRESH` fell through to each protocol reducer's `default:`
 * and returned the state unchanged. The panels *do* dispatch it — `RequestTabContent`
 * calls `tabRefs.current[tabId].refresh(entity)` after reverting an entity — so
 * discarding changes on a GraphQL/SOAP/gRPC tab left the editor showing the edits the
 * user had just thrown away. REST never hit it because it implements REFRESH itself.
 *
 * The `REFRESH` block below is the regression guard for that fix.
 */

import { describe, it, expect } from "vitest";

import {
  graphqlTabReducer,
  initGraphQLState,
  stateToRequestPayload,
  stateToMockPayload,
  stateToDraft as gqlStateToDraft,
  isDraftEmpty as isGqlDraftEmpty,
  type GraphQLTabState,
} from "@/components/graphql/graphqlTabReducer";
import {
  soapTabReducer,
  initSoapState,
  soapStateToSavePayload,
  soapStateToDraft,
  isSoapDraftEmpty,
  type SoapTabState,
} from "@/components/soap/soapTabReducer";
import {
  grpcTabReducer,
  initGrpcRequestState,
  initGrpcMockState,
  stateToRequestDraft,
  stateToMockDraft,
  requestToSaveData,
  mockToSaveData,
  type GrpcTabState,
} from "@/components/grpc/grpcTabReducer";

// ── Fixtures ──────────────────────────────────────────────────────────────────

const GQL_REQ: any = {
  id: "gql-1", workspaceId: "ws-1", createdAt: 1,
  name: "Get Users",
  endpointUrl: "https://api.example.com/graphql",
  headers: { "x-api-key": "secret" },
  query: "query GetUsers { users { id } }",
  variables: '{"limit":10}',
  operationName: "GetUsers",
  preScript: "pm.environment.set('a', 1);",
  postScript: "pm.test('ok', () => true);",
  schemaId: "schema-1",
  folderId: "folder-1",
};

const GQL_MOCK: any = {
  id: "gqlm-1", workspaceId: "ws-1", createdAt: 1,
  name: "User Mock",
  endpointPattern: "https://api.example.com/graphql",
  useRegex: false,
  operationType: "query",
  operationName: "GetUsers",
  responseStatus: 200,
  responseHeaders: { "content-type": "application/json" },
  responseBody: '{"data":{"users":[]}}',
  responseDelay: 0,
  schemaId: null,
  folderId: null,
};

const SOAP_REQ: any = {
  id: "soap-1", workspaceId: "ws-1", createdAt: 1,
  name: "GetQuote",
  endpointUrl: "https://api.example.com/soap",
  soapAction: "http://example.com/GetQuote",
  headers: { "content-type": "text/xml" },
  body: "<soap:Envelope><soap:Body/></soap:Envelope>",
  wsdlId: "wsdl-1",
  operationName: "GetQuote",
  preScript: "", postScript: "",
  folderId: "folder-2",
};

const SOAP_MOCK: any = {
  id: "soapm-1", workspaceId: "ws-1", createdAt: 1,
  name: "Quote Mock",
  endpointPattern: "https://api.example.com/soap",
  useRegex: true,
  soapActionPattern: ".*GetQuote",
  responseStatus: 500,
  responseHeaders: { "Content-Type": "text/xml; charset=utf-8" },
  responseBody: "<soap:Fault/>",
  responseDelay: 25,
  wsdlId: null,
  operationName: "",
  folderId: null,
};

const GRPC_REQ: any = {
  id: "grpc-1", workspaceId: "ws-1", createdAt: 1,
  name: "SayHello",
  serverAddress: "grpc.example.com:443",
  serviceName: "helloworld.Greeter",
  methodName: "SayHello",
  requestBody: '{"name":"world"}',
  metadata: { authorization: "Bearer t" },
  protoFileId: "proto-1",
  useReflection: true,
  streamingType: "server",
  preScript: "pm.environment.set('b', 2);",
  postScript: "",
  folderId: "folder-3",
};

const GRPC_MOCK: any = {
  id: "grpcm-1", workspaceId: "ws-1", createdAt: 1,
  name: "Greeter Mock",
  serviceName: "helloworld.Greeter",
  methodName: "SayHello",
  responseBody: '{"message":"hi"}',
  responseMetadata: { "x-mocked": "1" },
  responseDelay: 10,
  streamingResponses: ["a", "b"],
  errorCode: 5,
  errorMessage: "not found",
  protoFileId: "proto-2",
  enabled: false,
  folderId: null,
};

// ── Shared REFRESH semantics ──────────────────────────────────────────────────

/**
 * The fields `createTabReducer` treats as runtime state. A REFRESH must leave these
 * untouched — it re-derives entity data, it does not wipe the response pane.
 */
const RUNTIME_FIELDS = new Set([
  "loading", "result", "sendErr", "durationMs",
  "sending", "resStatus", "resHeaders", "resBody", "resDuration", "resError",
  "responses", "resMetadata", "resStatusMessage",
  "testLoading", "testError",
  "saving", "dirty",
]);

/** Everything except the runtime fields — i.e. the entity-derived part of the state. */
function entityPart(state: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(state)) {
    if (!RUNTIME_FIELDS.has(key)) out[key] = state[key];
  }
  return out;
}

describe("protocol reducers — REFRESH (the discarded-edits bug)", () => {
  it("restores GraphQL request fields to the entity, discarding local edits", () => {
    const fresh = initGraphQLState(GQL_REQ, null, "request");
    const edited: GraphQLTabState = graphqlTabReducer(fresh, {
      type: "SET_FIELD", field: "endpointUrl", value: "https://wrong.example.com",
    });
    expect(edited.endpointUrl).toBe("https://wrong.example.com");

    const refreshed = graphqlTabReducer(edited, { type: "REFRESH", entity: GQL_REQ, tabType: "request" } as any);
    expect(refreshed.endpointUrl).toBe(GQL_REQ.endpointUrl);
    expect(entityPart(refreshed as any)).toEqual(entityPart(fresh as any));
  });

  it("restores a GraphQL mock field whose entity value equals the default", () => {
    // The trap: a naive "copy only the keys that differ from the blank state" merge
    // would keep the edited 404 because the entity's 200 matches the default.
    const fresh = initGraphQLState(GQL_MOCK, null, "mock");
    const edited = graphqlTabReducer(fresh, { type: "SET_FIELD", field: "responseStatus", value: 404 });
    expect(edited.responseStatus).toBe(404);

    const refreshed = graphqlTabReducer(edited, { type: "REFRESH", entity: GQL_MOCK, tabType: "mock" } as any);
    expect(refreshed.responseStatus).toBe(200);
  });

  it("preserves the response pane and in-flight flags across a REFRESH", () => {
    const fresh = initGraphQLState(GQL_REQ, null, "request");
    const sent = graphqlTabReducer(fresh, {
      type: "SEND_SUCCESS", status: 201, headers: { "x-res": "1" }, body: '{"ok":true}', durationMs: 42,
    });
    const refreshed = graphqlTabReducer(sent, { type: "REFRESH", entity: GQL_REQ, tabType: "request" } as any);

    expect(refreshed.resStatus).toBe(201);
    expect(refreshed.resBody).toBe('{"ok":true}');
    expect(refreshed.resDuration).toBe(42);
    expect(refreshed.sending).toBe(false);
  });

  it("restores SOAP request fields, and preserves the response pane", () => {
    const fresh = initSoapState(SOAP_REQ, null, "request");
    const edited = soapTabReducer(fresh, { type: "SET_FIELD", field: "soapAction", value: "wrong" });
    const sent = soapTabReducer(edited, {
      type: "SEND_SUCCESS", status: 200, headers: { "content-type": "text/xml" }, body: "<ok/>", durationMs: 7,
    });

    const refreshed = soapTabReducer(sent, { type: "REFRESH", entity: SOAP_REQ, tabType: "request" } as any);
    expect(refreshed.soapAction).toBe(SOAP_REQ.soapAction);
    expect(entityPart(refreshed as any)).toEqual(entityPart(fresh as any));
    expect(refreshed.resBody).toBe("<ok/>");
  });

  it("restores gRPC request fields, and preserves the response stream", () => {
    const fresh = initGrpcRequestState(GRPC_REQ);
    const edited = grpcTabReducer(fresh, { type: "SET_FIELD", field: "methodName", value: "Nope" });
    const sent = grpcTabReducer(edited, {
      type: "SEND_SUCCESS", responses: ["one", "two"], metadata: { "x-m": "1" }, status: 0, statusMessage: "OK", durationMs: 11,
    });

    const refreshed = grpcTabReducer(sent, { type: "REFRESH", entity: GRPC_REQ, tabType: "request" } as any);
    expect(refreshed.methodName).toBe("SayHello");
    expect(entityPart(refreshed as any)).toEqual(entityPart(fresh as any));
    expect(refreshed.responses).toEqual(["one", "two"]);
    expect(refreshed.resStatusMessage).toBe("OK");
  });

  it("restores gRPC mock fields, and preserves the response stream", () => {
    const fresh = initGrpcMockState(GRPC_MOCK);
    const edited = grpcTabReducer(fresh, { type: "SET_FIELD", field: "errorCode", value: 99 });
    const refreshed = grpcTabReducer(edited, { type: "REFRESH", entity: GRPC_MOCK, tabType: "mock" } as any);

    expect(refreshed.errorCode).toBe(5);
    expect(entityPart(refreshed as any)).toEqual(entityPart(fresh as any));
  });

  it("rebuilds from scratch on LOAD_ENTITY, resetting the response pane", () => {
    const sent = graphqlTabReducer(initGraphQLState(GQL_REQ, null, "request"), {
      type: "SEND_SUCCESS", status: 500, headers: {}, body: "boom", durationMs: 1,
    });
    const loaded = graphqlTabReducer(sent, { type: "LOAD_ENTITY", entity: GQL_REQ, tabType: "request" } as any);

    expect(loaded.resStatus).toBeNull();
    expect(loaded.resBody).toBe("");
    expect(loaded.sending).toBe(false);
    expect(loaded.name).toBe("Get Users");
  });

  it("rebuilds from a draft on LOAD_DRAFT", () => {
    const draft = {
      name: "Draft", endpointUrl: "https://draft.example.com", headers: {}, query: "query { x }",
      variables: "", operationName: "", preScript: "", postScript: "", folderId: null,
    };
    const loaded = graphqlTabReducer(initGraphQLState(null, null, "request"), {
      type: "LOAD_DRAFT", draft, tabType: "request",
    } as any);

    expect(loaded.name).toBe("Draft");
    expect(loaded.endpointUrl).toBe("https://draft.example.com");
  });
});

// ── GraphQL ───────────────────────────────────────────────────────────────────

describe("graphqlTabReducer — init", () => {
  it("returns blank request state, with no mock fields leaking in", () => {
    const s = initGraphQLState(null, null, "request");
    expect(s.tabType).toBe("request");
    expect(s.name).toBe("");
    expect(s.endpointUrl).toBe("");
    expect(s.query).toBe("");
    expect(s.sending).toBe(false);
    expect(s.saving).toBe(false);
    expect(s.dirty).toBe(false);
  });

  it("defaults the mock response to an empty GraphQL data envelope", () => {
    const s = initGraphQLState(null, null, "mock");
    expect(s.responseStatus).toBe(200);
    expect(JSON.parse(s.responseBody)).toEqual({ data: {} });
    expect(s.operationType).toBe("any");
    expect(s.enabled).toBe(true);
  });

  it("loads a request entity", () => {
    const s = initGraphQLState(GQL_REQ, null, "request");
    expect(s.name).toBe("Get Users");
    expect(s.endpointUrl).toBe(GQL_REQ.endpointUrl);
    expect(s.query).toBe(GQL_REQ.query);
    expect(s.operationName).toBe("GetUsers");
    expect(s.schemaId).toBe("schema-1");
    expect(s.folderId).toBe("folder-1");
    expect(s.headers).toEqual({ "x-api-key": "secret" });
  });

  it("maps the mock entity's operationName onto operationNameMatch", () => {
    const s = initGraphQLState(GQL_MOCK, null, "mock");
    expect(s.operationNameMatch).toBe("GetUsers");
    expect(s.endpointPattern).toBe(GQL_MOCK.endpointPattern);
    expect(s.responseBody).toBe(GQL_MOCK.responseBody);
  });

  it("lets a draft win over the entity", () => {
    const draft = {
      name: "Draft", endpointUrl: "https://draft", headers: {}, query: "q", variables: "",
      operationName: "", preScript: "", postScript: "", folderId: "f9",
    };
    const s = initGraphQLState(GQL_REQ, draft, "request");
    expect(s.name).toBe("Draft");
    expect(s.folderId).toBe("f9");
  });
});

describe("graphqlTabReducer — lifecycle", () => {
  it("SEND_START clears the previous response", () => {
    const s0 = graphqlTabReducer(initGraphQLState(GQL_REQ, null, "request"), {
      type: "SEND_SUCCESS", status: 200, headers: { a: "b" }, body: "old", durationMs: 3,
    });
    const s1 = graphqlTabReducer(s0, { type: "SEND_START" });
    expect(s1.sending).toBe(true);
    expect(s1.resStatus).toBeNull();
    expect(s1.resBody).toBe("");
    expect(s1.resHeaders).toEqual({});
    expect(s1.resDuration).toBeNull();
    expect(s1.resError).toBeNull();
  });

  it("SEND_ERROR clears the sending flag and records the error", () => {
    const s0 = graphqlTabReducer(initGraphQLState(GQL_REQ, null, "request"), { type: "SEND_START" });
    const s1 = graphqlTabReducer(s0, { type: "SEND_ERROR", error: "ECONNREFUSED" });
    expect(s1.sending).toBe(false);
    expect(s1.resError).toBe("ECONNREFUSED");
  });

  it("SET_FIELD marks the tab dirty", () => {
    const s0 = initGraphQLState(GQL_REQ, null, "request");
    expect(s0.dirty).toBe(false);
    const s1 = graphqlTabReducer(s0, { type: "SET_FIELD", field: "name", value: "Renamed" });
    expect(s1.name).toBe("Renamed");
    expect(s1.dirty).toBe(true);
  });

  it("SAVE_* drives the saving flag and clears dirty on success", () => {
    const s0 = graphqlTabReducer(initGraphQLState(GQL_REQ, null, "request"), {
      type: "SET_FIELD", field: "name", value: "X",
    });
    const s1 = graphqlTabReducer(s0, { type: "SAVE_START" });
    expect(s1.saving).toBe(true);
    expect(s1.saveErr).toBeNull();

    const s2 = graphqlTabReducer(s1, { type: "SAVE_SUCCESS" });
    expect(s2.saving).toBe(false);
    expect(s2.dirty).toBe(false);

    const s3 = graphqlTabReducer(s1, { type: "SAVE_ERROR", error: "disk full" });
    expect(s3.saving).toBe(false);
    expect(s3.saveErr).toBe("disk full");
  });

  it("treats SAVE_DONE like SAVE_SUCCESS", () => {
    const s0 = graphqlTabReducer(initGraphQLState(GQL_REQ, null, "request"), { type: "SAVE_START" });
    const s1 = graphqlTabReducer(s0, { type: "SAVE_DONE" });
    expect(s1.saving).toBe(false);
  });

  it("ignores an unknown action", () => {
    const s0 = initGraphQLState(GQL_REQ, null, "request");
    const s1 = graphqlTabReducer(s0, { type: "SOMETHING_ELSE" } as any);
    expect(s1).toBe(s0);
  });
});

describe("graphqlTabReducer — serialization", () => {
  it("builds the request save payload, dropping empty scripts", () => {
    const s = initGraphQLState(GQL_REQ, null, "request");
    const payload = stateToRequestPayload(s);
    expect(payload.name).toBe("Get Users");
    expect(payload.endpointUrl).toBe(GQL_REQ.endpointUrl);
    expect(payload.query).toBe(GQL_REQ.query);
    expect(payload.schemaId).toBe("schema-1");
  });

  it("builds the mock save payload, mapping operationNameMatch back and defaulting delay", () => {
    const s = initGraphQLState(GQL_MOCK, null, "mock");
    const payload = stateToMockPayload(s);
    expect(payload.operationName).toBe("GetUsers");
    expect(payload.endpointPattern).toBe(GQL_MOCK.endpointPattern);
    expect(payload.responseStatus).toBe(200);
    // `responseDelay || undefined` — a zero delay is written as absent, not as 0.
    expect(payload.responseDelay).toBeUndefined();
  });

  it("keeps a non-zero mock delay", () => {
    const s = initGraphQLState({ ...GQL_MOCK, responseDelay: 250 }, null, "mock");
    expect(stateToMockPayload(s).responseDelay).toBe(250);
  });

  it("round-trips state through a draft", () => {
    const s = initGraphQLState(GQL_REQ, null, "request");
    const d = gqlStateToDraft(s, "request") as any;
    expect(d.endpointUrl).toBe(GQL_REQ.endpointUrl);
    expect(d.endpointPattern).toBeUndefined();

    const m = gqlStateToDraft(initGraphQLState(GQL_MOCK, null, "mock"), "mock") as any;
    expect(m.endpointPattern).toBe(GQL_MOCK.endpointPattern);
    expect(m.endpointUrl).toBeUndefined();
  });

  it("reports an empty draft for a blank request, but not for a blank mock", () => {
    expect(isGqlDraftEmpty(initGraphQLState(null, null, "request"), "request")).toBe(true);
    expect(isGqlDraftEmpty({ ...initGraphQLState(null, null, "request"), query: "q" }, "request")).toBe(false);
    expect(isGqlDraftEmpty({ ...initGraphQLState(null, null, "request"), name: "N" }, "request")).toBe(false);

    // Asymmetry worth knowing about: a *blank* GraphQL mock is NOT reported as empty,
    // because the default response body is a non-empty `{ data: {} }` envelope and this
    // predicate only checks truthiness. SOAP avoids this by comparing its response body
    // against the default envelope. Harmless in practice — `useDraftPersist` uses the
    // predicate only to skip its debounced save, and it saves on mount and unmount
    // regardless — but it means a GraphQL mock draft is never suppressed as "blank".
    expect(isGqlDraftEmpty(initGraphQLState(null, null, "mock"), "mock")).toBe(false);
    expect(isGqlDraftEmpty({ ...initGraphQLState(null, null, "mock"), name: "N" }, "mock")).toBe(false);
  });
});

// ── SOAP ──────────────────────────────────────────────────────────────────────

describe("soapTabReducer", () => {
  it("seeds a blank request with a SOAP envelope and the XML content type", () => {
    const s = initSoapState(null, null, "request");
    expect(s.body).toContain("soap:Envelope");
    expect(s.responseBody).toContain("soap:Envelope");
    expect(s.responseHeaders["Content-Type"]).toBe("text/xml; charset=utf-8");
    expect(s.reqTab).toBe("body");
    expect(s.resTab).toBe("body");
  });

  it("falls back to the default envelope when the entity body is empty", () => {
    const s = initSoapState({ ...SOAP_REQ, body: "" }, null, "request");
    expect(s.body).toContain("soap:Envelope");
  });

  it("loads a request entity", () => {
    const s = initSoapState(SOAP_REQ, null, "request");
    expect(s.soapAction).toBe(SOAP_REQ.soapAction);
    expect(s.wsdlId).toBe("wsdl-1");
    expect(s.operationName).toBe("GetQuote");
    expect(s.body).toBe(SOAP_REQ.body);
  });

  it("loads a mock entity", () => {
    const s = initSoapState(SOAP_MOCK, null, "mock");
    expect(s.endpointPattern).toBe(SOAP_MOCK.endpointPattern);
    expect(s.useRegex).toBe(true);
    expect(s.soapActionPattern).toBe(".*GetQuote");
    expect(s.responseStatus).toBe(500);
    expect(s.responseDelay).toBe(25);
  });

  it("lets a draft win over the entity", () => {
    const draft = {
      name: "D", endpointUrl: "https://d", soapAction: "act", headers: {}, body: "<x/>",
      folderId: null, preScript: "", postScript: "",
    };
    const s = initSoapState(SOAP_REQ, draft, "request");
    expect(s.name).toBe("D");
    expect(s.soapAction).toBe("act");
  });

  it("runs the send lifecycle", () => {
    const s0 = initSoapState(SOAP_REQ, null, "request");
    const s1 = soapTabReducer(s0, { type: "SEND_START" });
    expect(s1.sending).toBe(true);
    expect(s1.resBody).toBe("");

    const s2 = soapTabReducer(s1, { type: "SEND_SUCCESS", status: 200, headers: { a: "b" }, body: "<ok/>", durationMs: 12 });
    expect(s2.sending).toBe(false);
    expect(s2.resStatus).toBe(200);
    expect(s2.resBody).toBe("<ok/>");

    const s3 = soapTabReducer(s1, { type: "SEND_ERROR", error: "boom" });
    expect(s3.sending).toBe(false);
    expect(s3.resError).toBe("boom");
  });

  it("marks the tab dirty on edit and clean on save", () => {
    const s0 = initSoapState(SOAP_REQ, null, "request");
    const s1 = soapTabReducer(s0, { type: "SET_FIELD", field: "soapAction", value: "other" });
    expect(s1.dirty).toBe(true);
    expect(soapTabReducer(soapTabReducer(s1, { type: "SAVE_START" }), { type: "SAVE_SUCCESS" }).dirty).toBe(false);
  });

  it("builds the request save payload", () => {
    const payload: any = soapStateToSavePayload(initSoapState(SOAP_REQ, null, "request"), "request");
    expect(payload.endpointUrl).toBe(SOAP_REQ.endpointUrl);
    expect(payload.soapAction).toBe(SOAP_REQ.soapAction);
    expect(payload.wsdlId).toBe("wsdl-1");
  });

  it("builds the mock save payload, always enabled", () => {
    const payload: any = soapStateToSavePayload(initSoapState(SOAP_MOCK, null, "mock"), "mock");
    expect(payload.endpointPattern).toBe(SOAP_MOCK.endpointPattern);
    expect(payload.useRegex).toBe(true);
    expect(payload.responseStatus).toBe(500);
    // SOAP mock state has no `enabled` field, so the payload pins it true.
    expect(payload.enabled).toBe(true);
  });

  it("round-trips state through a draft", () => {
    const d: any = soapStateToDraft(initSoapState(SOAP_REQ, null, "request"), "request");
    expect(d.soapAction).toBe(SOAP_REQ.soapAction);
    expect(d.endpointPattern).toBeUndefined();

    const m: any = soapStateToDraft(initSoapState(SOAP_MOCK, null, "mock"), "mock");
    expect(m.endpointPattern).toBe(SOAP_MOCK.endpointPattern);
    expect(m.soapAction).toBeUndefined();
  });

  it("reports an empty draft only when the identifying fields are blank", () => {
    expect(isSoapDraftEmpty(initSoapState(null, null, "request"), "request")).toBe(true);
    expect(isSoapDraftEmpty(initSoapState(null, null, "mock"), "mock")).toBe(true);
    expect(isSoapDraftEmpty({ ...initSoapState(null, null, "request"), soapAction: "a" }, "request")).toBe(false);
    expect(isSoapDraftEmpty({ ...initSoapState(null, null, "mock"), endpointPattern: "/x" }, "mock")).toBe(false);
  });
});

// ── gRPC ──────────────────────────────────────────────────────────────────────

describe("grpcTabReducer", () => {
  it("seeds a blank request with a local server and an empty JSON message", () => {
    const s = initGrpcRequestState();
    expect(s.tabType).toBe("request");
    expect(s.serverAddress).toBe("localhost:50051");
    expect(s.requestBody).toBe("{}");
    expect(s.streamingType).toBe("unary");
    expect(s.useReflection).toBe(false);
    expect(s.metadata).toEqual({});
  });

  it("seeds a blank mock", () => {
    const s = initGrpcMockState();
    expect(s.tabType).toBe("mock");
    expect(s.responseBody).toBe("{}");
    expect(s.enabled).toBe(true);
    expect(s.streamingResponses).toEqual([]);
  });

  it("loads request and mock entities", () => {
    const r = initGrpcRequestState(GRPC_REQ);
    expect(r.serverAddress).toBe("grpc.example.com:443");
    expect(r.serviceName).toBe("helloworld.Greeter");
    expect(r.useReflection).toBe(true);
    expect(r.streamingType).toBe("server");
    expect(r.protoFileId).toBe("proto-1");

    const m = initGrpcMockState(GRPC_MOCK);
    expect(m.responseBody).toBe('{"message":"hi"}');
    expect(m.streamingResponses).toEqual(["a", "b"]);
    expect(m.errorCode).toBe(5);
    expect(m.enabled).toBe(false);
  });

  it("SET_METADATA replaces metadata and marks the tab dirty", () => {
    const s = grpcTabReducer(initGrpcRequestState(), { type: "SET_METADATA", metadata: { a: "1" } });
    expect(s.metadata).toEqual({ a: "1" });
    expect(s.dirty).toBe(true);
  });

  it("SET_RESPONSE_METADATA replaces response metadata and marks the tab dirty", () => {
    const s = grpcTabReducer(initGrpcMockState(), { type: "SET_RESPONSE_METADATA", metadata: { b: "2" } });
    expect(s.responseMetadata).toEqual({ b: "2" });
    expect(s.dirty).toBe(true);
  });

  it("runs the send lifecycle and keeps every streamed response", () => {
    const s0 = grpcTabReducer(initGrpcRequestState(GRPC_REQ), { type: "SEND_START" });
    expect(s0.sending).toBe(true);
    expect(s0.responses).toEqual([]);

    const s1 = grpcTabReducer(s0, {
      type: "SEND_SUCCESS", responses: ["a", "b", "c"], metadata: { "x-m": "1" },
      status: 0, statusMessage: "OK", durationMs: 30,
    });
    expect(s1.sending).toBe(false);
    expect(s1.responses).toEqual(["a", "b", "c"]);
    expect(s1.resMetadata).toEqual({ "x-m": "1" });
    expect(s1.resStatus).toBe(0);
    expect(s1.resStatusMessage).toBe("OK");
    expect(s1.resDuration).toBe(30);
  });

  it("records a send error on resError (gRPC state has no sendErr field)", () => {
    const s0 = grpcTabReducer(initGrpcRequestState(), { type: "SEND_START" });
    const s1 = grpcTabReducer(s0, { type: "SEND_ERROR", error: "UNAVAILABLE" });
    expect(s1.sending).toBe(false);
    expect(s1.resError).toBe("UNAVAILABLE");
  });

  it("LOAD merges a partial state and clears dirty", () => {
    const dirty = grpcTabReducer(initGrpcRequestState(GRPC_REQ), { type: "SET_FIELD", field: "name", value: "x" });
    expect(dirty.dirty).toBe(true);

    const loaded = grpcTabReducer(dirty, { type: "LOAD", state: { serverAddress: "other:1", streamingType: "bidi" } });
    expect(loaded.serverAddress).toBe("other:1");
    expect(loaded.streamingType).toBe("bidi");
    expect(loaded.name).toBe("x"); // untouched fields survive
    expect(loaded.dirty).toBe(false);
  });

  it("serializes a request draft and a save payload", () => {
    const s = initGrpcRequestState(GRPC_REQ);
    expect(stateToRequestDraft(s).serverAddress).toBe("grpc.example.com:443");
    const payload = requestToSaveData(s);
    expect(payload.serviceName).toBe("helloworld.Greeter");
    expect(payload.metadata).toEqual({ authorization: "Bearer t" });
    expect(payload.streamingType).toBe("server");
  });

  it("serializes a mock draft and a save payload, defaulting protoFileId to empty", () => {
    const s = initGrpcMockState({ ...GRPC_MOCK, protoFileId: null });
    const draft = stateToMockDraft(s);
    expect(draft.responseMetadata).toEqual({ "x-mocked": "1" });
    expect(draft.enabled).toBe(false);

    const payload = mockToSaveData(s);
    expect(payload.errorCode).toBe(5);
    expect(payload.streamingResponses).toEqual(["a", "b"]);
    // `protoFileId ?? ""` — the saved mock uses an empty string, not null.
    expect(payload.protoFileId).toBe("");
  });
});
