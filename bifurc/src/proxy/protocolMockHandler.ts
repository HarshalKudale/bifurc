import * as net from "net";
import { Environment } from "@/store/config";
import { resolveVars, serveMock } from "@/proxy/mockHandler";

/**
 * GraphQL mock: matches by endpoint URL pattern + operation type + operation name.
 * Parses the request body for query/operationName fields.
 */
export interface GraphQLMockDef {
    id: string;
    enabled: boolean;
    endpointPattern: string;
    useRegex: boolean;
    operationType: "query" | "mutation" | "subscription" | "any";
    operationName: string;
    responseStatus: number;
    responseHeaders: Record<string, string>;
    responseBody: string;
    responseDelay?: number;
}

/**
 * SOAP mock: matches by endpoint URL pattern + SOAPAction header + operation name in body.
 */
export interface SoapMockDef {
    id: string;
    enabled: boolean;
    endpointPattern: string;
    useRegex: boolean;
    soapActionPattern: string;
    operationName?: string;
    responseStatus: number;
    responseHeaders: Record<string, string>;
    responseBody: string;
    responseDelay?: number;
}

// ── GraphQL Mock Matching ──────────────────────────────────────────────────

export function matchGraphQLMock(
    mocks: GraphQLMockDef[],
    url: string,
    bodyStr: string,
    env: Environment | null,
): GraphQLMockDef | null {
    if (!mocks || mocks.length === 0) return null;

    // Parse the request body to extract operation info
    let parsedQuery = "";
    let parsedOpName = "";
    try {
        const parsed = JSON.parse(bodyStr);
        parsedQuery = parsed.query ?? "";
        parsedOpName = parsed.operationName ?? "";
    } catch {
        return null; // Not valid JSON, can't be a GraphQL request
    }

    // Determine operation type from query string
    const opType = detectOperationType(parsedQuery);

    for (const m of mocks) {
        if (!m.enabled) continue;

        // URL pattern match
        const pattern = resolveVars(m.endpointPattern, env);
        const urlMatch = m.useRegex
            ? safeRegexTest(pattern, url)
            : url.includes(pattern);
        if (!urlMatch) continue;

        // Operation type match
        if (m.operationType !== "any" && opType !== m.operationType) continue;

        // Operation name match
        if (m.operationName) {
            const nameMatch = parsedOpName === m.operationName ||
                extractOperationNameFromQuery(parsedQuery) === m.operationName;
            if (!nameMatch) continue;
        }

        return m;
    }
    return null;
}

// ── SOAP Mock Matching ─────────────────────────────────────────────────────

export function matchSoapMock(
    mocks: SoapMockDef[],
    url: string,
    headers: Record<string, string>,
    bodyStr: string,
    env: Environment | null,
): SoapMockDef | null {
    if (!mocks || mocks.length === 0) return null;

    const soapAction = headers["soapaction"] || headers["SOAPAction"] || "";
    const opName = extractSoapOperationName(bodyStr);

    for (const m of mocks) {
        if (!m.enabled) continue;

        // URL pattern match
        const pattern = resolveVars(m.endpointPattern, env);
        const urlMatch = m.useRegex
            ? safeRegexTest(pattern, url)
            : url.includes(pattern);
        if (!urlMatch) continue;

        // SOAPAction match
        if (m.soapActionPattern) {
            const actionMatch = m.useRegex
                ? safeRegexTest(m.soapActionPattern, soapAction)
                : soapAction.includes(m.soapActionPattern);
            if (!actionMatch) continue;
        }

        // Operation name match (optional)
        if (m.operationName && opName && m.operationName !== opName) continue;

        return m;
    }
    return null;
}

// ── Serve GraphQL/SOAP Mock Response ───────────────────────────────────────

export function serveProtocolMock(
    socket: net.Socket,
    mock: { responseStatus: number; responseHeaders: Record<string, string>; responseBody: string; responseDelay?: number },
    env: Environment | null,
): void {
    const delay = mock.responseDelay && mock.responseDelay > 0 ? mock.responseDelay : 0;

    const send = () => {
        if (!socket.writable) return;
        const body = Buffer.from(resolveVars(mock.responseBody, env), "utf-8");
        const headers: Record<string, string> = { ...mock.responseHeaders };
        headers["content-length"] = String(body.length);
        headers["connection"] = "close";
        if (!headers["content-type"]) headers["content-type"] = "application/json";

        let head = `HTTP/1.1 ${mock.responseStatus} ${statusText(mock.responseStatus)}\r\n`;
        for (const [k, v] of Object.entries(headers)) head += `${k}: ${resolveVars(v, env)}\r\n`;
        head += "\r\n";

        socket.write(head);
        socket.write(body);
        socket.end();
    };

    if (delay > 0) setTimeout(send, delay);
    else send();
}

// ── Helpers ────────────────────────────────────────────────────────────────

function detectOperationType(query: string): "query" | "mutation" | "subscription" | "any" {
    const trimmed = query.trim();
    if (trimmed.startsWith("mutation")) return "mutation";
    if (trimmed.startsWith("subscription")) return "subscription";
    if (trimmed.startsWith("query") || trimmed.startsWith("{")) return "query";
    return "any";
}

function extractOperationNameFromQuery(query: string): string {
    // Match: query OperationName or mutation OperationName
    const match = /^(?:query|mutation|subscription)\s+(\w+)/m.exec(query.trim());
    return match?.[1] ?? "";
}

function extractSoapOperationName(xml: string): string {
    // Extract the first child element of <Body> as operation name
    const bodyMatch = /<(?:soap|SOAP|s):Body[^>]*>([\s\S]*?)<\/(?:soap|SOAP|s):Body>/i.exec(xml);
    if (!bodyMatch) return "";
    const bodyContent = bodyMatch[1].trim();
    // Get first element name (skip namespace prefix)
    const elementMatch = /<(?:\w+:)?(\w+)/i.exec(bodyContent);
    return elementMatch?.[1] ?? "";
}

function safeRegexTest(pattern: string, input: string): boolean {
    try {
        return new RegExp(pattern).test(input);
    } catch {
        return false;
    }
}

function statusText(code: number): string {
    const map: Record<number, string> = {
        200: "OK", 201: "Created", 204: "No Content",
        400: "Bad Request", 401: "Unauthorized", 403: "Forbidden",
        404: "Not Found", 500: "Internal Server Error", 502: "Bad Gateway",
        503: "Service Unavailable",
    };
    return map[code] ?? "OK";
}
