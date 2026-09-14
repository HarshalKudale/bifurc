import * as fs from "fs";
import { loadConfig, SavedRequest } from "@/store/config";
import { readAllEntities } from "@/store/workspaceFs";
import type { ExportResult } from "@/ipc/importExport/types";

function extractPathParams(url: string): string[] {
  const params: string[] = [];
  try {
    const pathname = new URL(url).pathname;
    const matches = pathname.matchAll(/\{([^}]+)\}/g);
    for (const m of matches) params.push(m[1]);
  } catch {}
  return params;
}

function toOpenApiPath(url: string): string {
  try {
    return new URL(url).pathname;
  } catch {
    const match = url.match(/^(?:https?:\/\/[^/]+)?(\/.*?)(?:\?.*)?$/);
    return match?.[1] ?? url;
  }
}

function detectContentType(body: string): string {
  const trimmed = body.trim();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) return "application/json";
  if (trimmed.startsWith("<")) return "application/xml";
  return "text/plain";
}

export async function run(wsId: string, filePath: string): Promise<ExportResult> {
  try {
    const cfg = loadConfig();
    const requests = readAllEntities<SavedRequest>(wsId, "requests").filter((r) => r.workspaceId === wsId);
    const ws = cfg.workspaces.find((w) => w.id === wsId);

    const paths: Record<string, Record<string, unknown>> = {};

    for (const req of requests) {
      const opPath = toOpenApiPath(req.url);
      const method = req.method.toLowerCase();
      if (!paths[opPath]) paths[opPath] = {};

      const params = extractPathParams(opPath).map((name) => ({
        name,
        in: "path",
        required: true,
        schema: { type: "string" },
      }));

      const headers = Object.entries(req.headers ?? {})
        .filter(([k]) => k.toLowerCase() !== "content-type")
        .map(([name, example]) => ({
          name,
          in: "header",
          schema: { type: "string" },
          example,
        }));

      const operation: Record<string, unknown> = {
        operationId: req.id,
        summary: req.name,
        parameters: [...params, ...headers],
        responses: { "200": { description: "OK" } },
      };

      if (req.body?.trim() && !["GET", "HEAD", "DELETE"].includes(req.method.toUpperCase())) {
        const ct = req.headers?.["Content-Type"] ?? req.headers?.["content-type"] ?? detectContentType(req.body);
        operation.requestBody = {
          content: {
            [ct]: { schema: { type: "string" }, example: req.body },
          },
        };
      }

      paths[opPath][method] = operation;
    }

    const spec = {
      openapi: "3.0.3",
      info: { title: ws?.name ?? "Bifurc", version: "1.0.0" },
      paths,
    };

    fs.writeFileSync(filePath, JSON.stringify(spec, null, 2), "utf-8");
    return { ok: true, filePath };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
