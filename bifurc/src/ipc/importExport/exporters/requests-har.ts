import * as fs from "fs";
import { loadConfig, SavedRequest } from "@/store/config";
import { readAllEntities } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";

function toHarEntry(req: SavedRequest): object {
  const headers = Object.entries(req.headers ?? {}).map(([name, value]) => ({ name, value }));

  const postData = req.body?.trim()
    ? {
        mimeType: req.headers?.["Content-Type"] ?? req.headers?.["content-type"] ?? "text/plain",
        text: req.body,
      }
    : undefined;

  return {
    startedDateTime: new Date(req.createdAt).toISOString(),
    time: 0,
    request: {
      method: req.method,
      url: req.url,
      httpVersion: "HTTP/1.1",
      headers,
      queryString: [],
      cookies: [],
      headersSize: -1,
      bodySize: req.body?.length ?? 0,
      ...(postData ? { postData } : {}),
    },
    response: {
      status: 0,
      statusText: "",
      httpVersion: "HTTP/1.1",
      headers: [],
      cookies: [],
      content: { size: 0, mimeType: "text/plain" },
      redirectURL: "",
      headersSize: -1,
      bodySize: -1,
    },
    cache: {},
    timings: { send: 0, wait: 0, receive: 0 },
  };
}

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const requests = readAllEntities<SavedRequest>(wsId, "requests").filter((r) => r.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);

    const har = {
      log: {
        version: "1.2",
        creator: { name: "Bifurc", version: "1.0" },
        comment: ws?.name ?? "Bifurc Requests",
        entries: requests.map(toHarEntry),
      },
    };

    fs.writeFileSync(filePath, JSON.stringify(har, null, 2), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
