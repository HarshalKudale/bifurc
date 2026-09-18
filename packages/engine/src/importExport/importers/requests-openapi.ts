import * as path from "path";
import * as yaml from "js-yaml";
import {
  loadConfig, saveConfig, generateId, SavedRequest, Folder,
} from "../../store/config";
import { writeEntity, upsertNameEntry } from "../../store/workspaceFs";
import { reloadConfig } from "../../proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy, ImportSource } from "../types";

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

// ── The second path-derivation site, which `plan/04` does not name ───────────
//
// `plan/04` work item 2 says to "grep for any other `filePath.split` / `path.basename(filePath)`"
// before declaring the name-derivation fix done. That grep does not match `path.extname`,
// which is what this function used. It is worse than a naming bug: a staged blob's content
// file has no extension at all, so `path.extname()` on it is `""` and EVERY YAML spec would
// have silently taken the JSON branch and failed to parse.
//
// The extension now comes from the client-supplied filename. `path` is a pure string
// helper, so keeping the import does not reintroduce filesystem access.
//
// **Synchronous, and shared with `preflight`.** It used to be `async` because it loaded
// `js-yaml` with `await import(...)`. `preflight` is a synchronous method on `ImporterFn`, so it
// could not use it and instead kept its own JSON-only parse with a line-count fallback — which
// meant a YAML spec's `itemCount` was `Math.floor(lines containing ":" / 3)`, and a malformed JSON
// file was reported as importable. A static import makes one parser serve both halves, which is
// what `ImporterFn`'s contract says they are: "preflight is a dry run against the same bytes `run`
// will consume" (`importExport/registry.ts`).
function loadSpec(source: ImportSource): OpenApiSpec {
  const text = source.content;
  const ext = path.extname(source.filename).toLowerCase();
  if (ext === ".yaml" || ext === ".yml") {
    return yaml.load(text) as OpenApiSpec;
  }
  return JSON.parse(text) as OpenApiSpec;
}

export function preflight(_wsId: string, source: ImportSource): PreflightResult {
  try {
    const spec = loadSpec(source);
    if (!spec.openapi && !spec.swagger) {
      return { ok: false, error: "Not a valid OpenAPI specification" };
    }
    // `collisionIds` is empty **by construction**, not by omission: `run` writes every operation
    // under a fresh `generateId()`, so an OpenAPI import cannot collide with an existing entity.
    // (The old line-count branch could not have reported one either — it never looked.)
    const count = Object.values(spec.paths ?? {})
      .flatMap((p) => Object.keys(p).filter((m) => METHODS.includes(m))).length;
    return { ok: true, itemCount: count, collisionIds: [] };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}

export async function run(
  wsId: string,
  source: ImportSource,
  _strategy: CollisionStrategy,
): Promise<ImportResult> {
  try {
    const spec = loadSpec(source);
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
