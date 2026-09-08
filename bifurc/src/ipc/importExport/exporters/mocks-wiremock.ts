import * as fs from "fs";
import { loadConfig, MockRule } from "@/store/config";
import type { ExportResult } from "@/ipc/importExport/types";

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

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const mocks = cfg.mocks.filter((m) => m.workspaceId === wsId);
    const mappings = mocks.map(mockToWireMock);
    fs.writeFileSync(filePath, JSON.stringify({ mappings }, null, 2), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
