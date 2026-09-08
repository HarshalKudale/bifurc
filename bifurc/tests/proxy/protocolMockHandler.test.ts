import { describe, it, expect, vi } from "vitest";

vi.mock("@/lib/randomizer", () => ({
    resolveRandomizers: vi.fn((text: string) => text),
}));

vi.mock("@/proxy/mockHandler", async (importOriginal) => {
    const orig = await importOriginal<typeof import("@/proxy/mockHandler")>();
    return { ...orig };
});

import { matchGraphQLMock, matchSoapMock, serveProtocolMock } from "@/proxy/protocolMockHandler";

const createSocket = () => ({ writable: true, write: vi.fn(), end: vi.fn() } as any);

function makeGraphQLMock(overrides: Partial<any> = {}): any {
    return {
        id: "gql1",
        enabled: true,
        endpointPattern: "/graphql",
        useRegex: false,
        operationType: "query",
        operationName: "GetUser",
        responseStatus: 200,
        responseHeaders: {},
        responseBody: '{"data":{"user":{"id":"1"}}}',
        responseDelay: 0,
        ...overrides,
    };
}

function makeSoapMock(overrides: Partial<any> = {}): any {
    return {
        id: "soap1",
        enabled: true,
        endpointPattern: "/ws",
        useRegex: false,
        soapActionPattern: "GetWeather",
        operationName: "",
        responseStatus: 200,
        responseHeaders: { "content-type": "text/xml" },
        responseBody: "<soap:Envelope><soap:Body><Response/></soap:Body></soap:Envelope>",
        responseDelay: 0,
        ...overrides,
    };
}

// ── matchGraphQLMock ─────────────────────────────────────────────────────

describe("matchGraphQLMock()", () => {
    it("returns null for empty mocks array", () => {
        expect(matchGraphQLMock([], "/graphql", '{"query":"{ user }"}', null)).toBeNull();
    });

    it("returns null for null mocks", () => {
        expect(matchGraphQLMock(null as any, "/graphql", '{"query":"{ user }"}', null)).toBeNull();
    });

    it("returns null for invalid JSON body", () => {
        const m = makeGraphQLMock();
        expect(matchGraphQLMock([m], "/graphql", "not json", null)).toBeNull();
    });

    it("skips disabled mocks", () => {
        const m = makeGraphQLMock({ enabled: false });
        expect(matchGraphQLMock([m], "/graphql", '{"query":"query GetUser { user { id } }","operationName":"GetUser"}', null)).toBeNull();
    });

    it("matches by endpoint URL and operation name", () => {
        const m = makeGraphQLMock();
        const body = JSON.stringify({ query: "query GetUser { user { id } }", operationName: "GetUser" });
        expect(matchGraphQLMock([m], "/graphql", body, null)).toBe(m);
    });

    it("does not match when URL pattern differs", () => {
        const m = makeGraphQLMock({ endpointPattern: "/api/graphql" });
        const body = JSON.stringify({ query: "query GetUser { user { id } }", operationName: "GetUser" });
        expect(matchGraphQLMock([m], "/graphql", body, null)).toBeNull();
    });

    it("matches any operation type when operationType is 'any'", () => {
        const m = makeGraphQLMock({ operationType: "any", operationName: "" });
        const body = JSON.stringify({ query: "mutation CreateUser { createUser { id } }" });
        expect(matchGraphQLMock([m], "/graphql", body, null)).toBe(m);
    });

    it("does not match wrong operation type", () => {
        const m = makeGraphQLMock({ operationType: "mutation" });
        const body = JSON.stringify({ query: "query GetUser { user { id } }", operationName: "GetUser" });
        expect(matchGraphQLMock([m], "/graphql", body, null)).toBeNull();
    });

    it("matches by operation name extracted from query when operationName field is empty", () => {
        const m = makeGraphQLMock({ operationName: "ListItems" });
        const body = JSON.stringify({ query: "query ListItems { items { id } }" });
        expect(matchGraphQLMock([m], "/graphql", body, null)).toBe(m);
    });

    it("matches with regex endpoint pattern", () => {
        const m = makeGraphQLMock({ endpointPattern: "/api/v\\d+/graphql", useRegex: true, operationName: "" });
        const body = JSON.stringify({ query: "{ user }" });
        expect(matchGraphQLMock([m], "/api/v2/graphql", body, null)).toBe(m);
    });
});

// ── matchSoapMock ────────────────────────────────────────────────────────

describe("matchSoapMock()", () => {
    const soapBody = `<soap:Envelope><soap:Body><GetWeather><City>Portland</City></GetWeather></soap:Body></soap:Envelope>`;

    it("returns null for empty mocks array", () => {
        expect(matchSoapMock([], "/ws", {}, soapBody, null)).toBeNull();
    });

    it("returns null for null mocks", () => {
        expect(matchSoapMock(null as any, "/ws", {}, soapBody, null)).toBeNull();
    });

    it("skips disabled mocks", () => {
        const m = makeSoapMock({ enabled: false });
        expect(matchSoapMock([m], "/ws", { soapaction: "GetWeather" }, soapBody, null)).toBeNull();
    });

    it("matches by URL and SOAPAction header", () => {
        const m = makeSoapMock();
        expect(matchSoapMock([m], "/ws", { soapaction: "GetWeather" }, soapBody, null)).toBe(m);
    });

    it("does not match when URL pattern differs", () => {
        const m = makeSoapMock({ endpointPattern: "/other" });
        expect(matchSoapMock([m], "/ws", { soapaction: "GetWeather" }, soapBody, null)).toBeNull();
    });

    it("does not match when SOAPAction pattern differs", () => {
        const m = makeSoapMock({ soapActionPattern: "GetTemperature" });
        expect(matchSoapMock([m], "/ws", { soapaction: "GetWeather" }, soapBody, null)).toBeNull();
    });

    it("matches with regex URL pattern", () => {
        const m = makeSoapMock({ endpointPattern: "^/ws", useRegex: true });
        expect(matchSoapMock([m], "/ws/endpoint", { soapaction: "GetWeather" }, soapBody, null)).toBe(m);
    });

    it("matches operation name from SOAP body", () => {
        const m = makeSoapMock({ soapActionPattern: "", operationName: "GetWeather" });
        expect(matchSoapMock([m], "/ws", {}, soapBody, null)).toBe(m);
    });

    it("does not match wrong operation name", () => {
        const m = makeSoapMock({ soapActionPattern: "", operationName: "SetWeather" });
        expect(matchSoapMock([m], "/ws", {}, soapBody, null)).toBeNull();
    });
});

// ── serveProtocolMock ────────────────────────────────────────────────────

describe("serveProtocolMock()", () => {
    it("writes HTTP response to socket", () => {
        const sock = createSocket();
        serveProtocolMock(sock, { responseStatus: 200, responseHeaders: {}, responseBody: '{"data":{}}', responseDelay: 0 }, null);
        const head = sock.write.mock.calls[0][0] as string;
        expect(head).toContain("HTTP/1.1 200 OK");
        expect(head).toContain("content-type: application/json");
    });

    it("writes body and ends socket", () => {
        const sock = createSocket();
        serveProtocolMock(sock, { responseStatus: 201, responseHeaders: {}, responseBody: "created", responseDelay: 0 }, null);
        const body = sock.write.mock.calls[1][0];
        expect(body.toString()).toBe("created");
        expect(sock.end).toHaveBeenCalled();
    });

    it("does not write when socket is not writable", () => {
        const sock = createSocket();
        sock.writable = false;
        serveProtocolMock(sock, { responseStatus: 200, responseHeaders: {}, responseBody: "x", responseDelay: 0 }, null);
        expect(sock.write).not.toHaveBeenCalled();
    });

    it("delays response when responseDelay > 0", () => {
        vi.useFakeTimers();
        const sock = createSocket();
        serveProtocolMock(sock, { responseStatus: 200, responseHeaders: {}, responseBody: "x", responseDelay: 100 }, null);
        expect(sock.write).not.toHaveBeenCalled();
        vi.advanceTimersByTime(100);
        expect(sock.write).toHaveBeenCalled();
        vi.useRealTimers();
    });
});
