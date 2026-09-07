import * as fs from "fs";
import * as path from "path";
import {
  loadConfig, saveConfig, generateId, SavedRequest, Folder,
} from "@/store/config";
import { writeEntity, upsertNameEntry } from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

interface OpenApiOperation {
  operationId?: string;
  summary?: string;
  requestBody?: {
    content?: Record<string, { schema?: object; example?: string }>;
  };
  parameters?: Array<{ name: string; in: string; schema?: { type?: string } }>;
}

interface OpenApiSpec {
  openapi?: string;
  swagger?: string;
  info?: { title?: string };
  servers?: Array<{ url: string }>;
  paths?: Record<string, Record<string, OpenApiOperation>>;
}

const METHODS = ["get", "post", "put", "patch", "delete", "head", "options", "trace"];

async function loadSpec(filePath: string): Promise<OpenApiSpec> {
  const text = fs.readFileSync(filePath, "utf-8");
  const ext = path.extname(filePath).toLowerCase();
  if (ext === ".yaml" || ext === ".yml") {
    const yaml = await import("js-yaml");
    return yaml.load(text) as OpenApiSpec;
  }
  return JSON.parse(text) as OpenApiSpec;
}

export function preflight(_wsId: string, filePath: string): PreflightResult {
  try {
    const text = fs.readFileSync(filePath, "utf-8");
    // Just check it looks like an OpenAPI spec
    let spec: OpenApiSpec;
    try {
      spec = JSON.parse(text);
    } catch {
      // Might be YAML — count lines as rough estimate
      const lines = text.split("\n").filter((l) => l.includes(":")).length;
      return { ok: true, filePath, itemCount: Math.floor(lines / 3), collisionIds: [] };
    }
    if (!spec.openapi && !spec.swagger) {
      return { ok: false, error: "Not a valid OpenAPI specification" };
    }
    const count = Object.values(spec.paths ?? {})
      .flatMap((p) => Object.keys(p).filter((m) => METHODS.includes(m))).length;
    return { ok: true, filePath, itemCount: count, collisionIds: [] };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  filePath: string,
  _strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const spec = await loadSpec(filePath);
    if (!spec.openapi && !spec.swagger) {
      return { ok: false, error: "Not a valid OpenAPI specification" };
    }

    const cfg = loadConfig();
    const baseUrl = spec.servers?.[0]?.url ?? "";
    const title = spec.info?.title ?? "Imported";

    // Create a single folder for this import
    const folderId = generateId();
    const newFolder: Folder = { id: folderId, name: title, parentId: null, createdAt: Date.now(), workspaceId: wsId };
    cfg.requestFolders = [...cfg.requestFolders, newFolder];

    let imported = 0;

    for (const [opPath, pathItem] of Object.entries(spec.paths ?? {})) {
      for (const method of METHODS) {
        if (!pathItem[method]) continue;
        const op = pathItem[method] as OpenApiOperation;

        const url = `${baseUrl}${opPath}`;
        const headers: Record<string, string> = {};

        let body = "";
        if (op.requestBody?.content) {
          const ct = Object.keys(op.requestBody.content)[0];
          if (ct) {
            headers["Content-Type"] = ct;
            body = op.requestBody.content[ct].example ?? "";
          }
        }

        const id = generateId();
        const name = op.summary ?? op.operationId ?? `${method.toUpperCase()} ${opPath}`;
        const newReq: SavedRequest = {
          id,
          name,
          method: method.toUpperCase(),
          url,
          headers,
          body,
          createdAt: Date.now(),
          folderId,
          workspaceId: wsId,
        };
        writeEntity(wsId, "requests", id, newReq, title);
        upsertNameEntry(wsId, "requests", id, { name, method: newReq.method, url });
        imported++;
      }
    }

    saveConfig(cfg);
    reloadConfig();
    return { ok: true, imported };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
