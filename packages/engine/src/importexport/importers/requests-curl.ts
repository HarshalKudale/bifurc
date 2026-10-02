import {
  loadConfig, saveConfig, generateId, SavedRequest,
} from "../../workspace/config";
import { writeEntity, upsertNameEntry } from "../../workspace/workspaceFs";
import { notifyConfigChanged } from "../../runtime/instances";
// The cURL parser is `utils/curl.ts` — **one implementation**, shared with the `request.parseCurl`
// command (`plan/19` WI 5). This file used to carry a near-verbatim copy of the renderer's parser,
// whose own comment said so; both copies are gone and this is the only one left.
import { parseCurlCommand as parseSingleCurl } from "../../utils/curl";
import type { PreflightResult, ImportResult, CollisionStrategy, ImportSource } from "../types";

function splitCurlBlocks(text: string): string[] {
  // Split on blank lines, then group lines that are part of the same curl command
  const blocks: string[] = [];
  let current = "";
  for (const line of text.split(/\r?\n/)) {
    const stripped = line.trim();
    if (!stripped || stripped.startsWith("#")) {
      if (current.trim().startsWith("curl")) {
        blocks.push(current.trim());
        current = "";
      }
      continue;
    }
    current += (current ? "\n" : "") + stripped;
  }
  if (current.trim().startsWith("curl")) blocks.push(current.trim());
  return blocks;
}

export function preflight(_wsId: string, source: ImportSource): PreflightResult {
  try {
    const blocks = splitCurlBlocks(source.content);
    const valid = blocks.filter((b) => parseSingleCurl(b) !== null);
    return { ok: true, itemCount: valid.length, collisionIds: [] };
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
    const text = source.content;
    const blocks = splitCurlBlocks(text);
    const cfg = loadConfig();

    // Extract comment names from lines preceding each block
    const lines = text.split(/\r?\n/);
    const nameMap = new Map<string, string>();
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      if (line.startsWith("#")) {
        const next = lines.slice(i + 1).find((l) => l.trim().startsWith("curl"));
        if (next) nameMap.set(next.trim().split("\n")[0], line.slice(1).trim());
      }
    }

    let imported = 0;
    for (const block of blocks) {
      const parsed = parseSingleCurl(block);
      if (!parsed) continue;
      const id = generateId();
      const firstLine = block.split("\n")[0];
      const name = nameMap.get(firstLine) || `${parsed.method} ${parsed.url}`;
      const newReq: SavedRequest = {
        id,
        name,
        method: parsed.method,
        url: parsed.url,
        headers: parsed.headers,
        body: parsed.body,
        createdAt: Date.now(),
        folderId: null,
        workspaceId: wsId,
      };
      writeEntity(wsId, "requests", id, newReq, null);
      upsertNameEntry(wsId, "requests", id, { name, method: parsed.method, url: parsed.url });
      imported++;
    }

    notifyConfigChanged();
    return { ok: true, imported };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
