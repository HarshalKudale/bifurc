/**
 * GraphQL / SOAP / gRPC tab reducers.
 *
 * WHY THIS EXISTS
 * ---------------
 * These three are the state machines behind the **request** editors for the non-REST
 * protocols. They were at 0% coverage: `restTabReducer` had a suite, its three
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
 *
 * ── 2026-09-25 — `plan/17` work item 8: the mock half is gone ─────────────────
 *
 * Mocks became REST-only, so **the mock half of each of these three reducers was deleted**:
 * `GraphQLTabType` / `SoapTabType` / `GrpcTabType` now have the single value `"request"`, and
 * `initGrpcMockState`, `stateToMockPayload`, `stateToMockDraft` and `mockToSaveData` no longer
 * exist. The mock cases that lived here were deleted with them — they were pinning a state
 * machine with no caller, and a test that keeps passing against dead code is worse than no test
 * because it reads as coverage.
 *
 * Two of the deletions were not straight removals, and both are worth knowing about:
 *
 *  - `initSoapState` used to seed the *mock* response fields (`responseBody`, `responseHeaders`)
 *    unconditionally — including for a request — and the "seeds a blank request" case pinned
 *    that. Those fields are gone; the case now asserts the request half and its name says so.
 *  - `initGrpcRequestState`'s sibling `initGrpcMockState` carried `SET_RESPONSE_METADATA`, whose
 *    request-side counterpart is `resMetadata` (set by `SEND_SUCCESS`, not by an action of its
 *    own). So that case went too rather than being retargeted.
 */

import { describe, it, expect } from "vitest";

import {
  graphqlTabReducer,
  initGraphQLState,
  stateToRequestPayload,
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
  stateToRequestDraft,
  requestToSaveData,
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

  it("round-trips state through a draft", () => {
    const s = initGraphQLState(GQL_REQ, null, "request");
    const d = gqlStateToDraft(s) as any;
    expect(d.endpointUrl).toBe(GQL_REQ.endpointUrl);
    expect(d.endpointPattern).toBeUndefined();
  });

  it("reports an empty draft for a blank request", () => {
    expect(isGqlDraftEmpty(initGraphQLState(null, null, "request"))).toBe(true);
    expect(isGqlDraftEmpty({ ...initGraphQLState(null, null, "request"), query: "q" })).toBe(false);
    expect(isGqlDraftEmpty({ ...initGraphQLState(null, null, "request"), name: "N" })).toBe(false);
  });
});

// ── SOAP ──────────────────────────────────────────────────────────────────────

describe("soapTabReducer", () => {
  it("seeds a blank request with a SOAP envelope", () => {
    /**
     * Until 2026-09-25 this case also asserted `responseBody` and `responseHeaders["Content-Type"]`
     * against a *request* state. Those were the **mock's** canned response, which `initSoapState`
     * seeded unconditionally — so the assertion was pinning a quirk rather than a property of a
     * blank request. `plan/17` work item 8 removed the mock fields, and the two assertions went with
     * them rather than being retargeted: a request's response pane is `resBody` / `resHeaders`, and
     * asserting the mock's defaults here would have preserved the quirk in a new form.
     */
    const s = initSoapState(null, null, "request");
    expect(s.body).toContain("soap:Envelope");
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
    const payload: any = soapStateToSavePayload(initSoapState(SOAP_REQ, null, "request"));
    expect(payload.endpointUrl).toBe(SOAP_REQ.endpointUrl);
    expect(payload.soapAction).toBe(SOAP_REQ.soapAction);
    expect(payload.wsdlId).toBe("wsdl-1");
  });

  it("round-trips state through a draft", () => {
    const d: any = soapStateToDraft(initSoapState(SOAP_REQ, null, "request"));
    expect(d.soapAction).toBe(SOAP_REQ.soapAction);
    expect(d.endpointPattern).toBeUndefined();
  });

  it("reports an empty draft when the identifying fields are blank", () => {
    expect(isSoapDraftEmpty(initSoapState(null, null, "request"))).toBe(true);
    expect(isSoapDraftEmpty({ ...initSoapState(null, null, "request"), soapAction: "a" })).toBe(false);
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

  it("loads a request entity", () => {
    const r = initGrpcRequestState(GRPC_REQ);
    expect(r.serverAddress).toBe("grpc.example.com:443");
    expect(r.serviceName).toBe("helloworld.Greeter");
    expect(r.useReflection).toBe(true);
    expect(r.streamingType).toBe("server");
    expect(r.protoFileId).toBe("proto-1");
  });

  it("SET_METADATA replaces metadata and marks the tab dirty", () => {
    const s = grpcTabReducer(initGrpcRequestState(), { type: "SET_METADATA", metadata: { a: "1" } });
    expect(s.metadata).toEqual({ a: "1" });
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
});
