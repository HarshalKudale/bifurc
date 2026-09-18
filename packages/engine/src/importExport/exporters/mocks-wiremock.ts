import { loadConfig, MockRule } from "../../store/config";
import type { ExportResult } from "../types";

function mockToWireMock(m: MockRule): object {
  const urlMatcher = m.useRegex
    ? { urlPattern: m.urlPattern }
    : { url: m.urlPattern };

  const stub: Record<string, unknown> = {
    id: m.id,
    name: m.name,
    request: {
      method: m.method === "*" ? "ANY" : m.method,
      ...urlMatcher,
    },
    response: {
      status: m.responseStatus,
      headers: m.responseHeaders,
      body: m.responseBody,
    },
  };

  if (m.responseDelay && m.responseDelay > 0) {
    stub.response = {
      ...(stub.response as object),
      fixedDelayMilliseconds: m.responseDelay,
    };
  }

  return stub;
}

export async function run(wsId: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const mocks = cfg.mocks.filter((m) => m.workspaceId === wsId);
    const mappings = mocks.map(mockToWireMock);
    const content = JSON.stringify({ mappings }, null, 2);
    return { ok: true, content, suggestedName: "mocks-export.json" };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
