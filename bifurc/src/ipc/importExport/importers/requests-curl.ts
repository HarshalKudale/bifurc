import * as fs from "fs";
import {
  loadConfig, saveConfig, generateId, SavedRequest,
} from "@/store/config";
import { writeEntity, upsertNameEntry } from "@/store/workspaceFs";
import { reloadConfig } from "@/proxy/server";
import type { PreflightResult, ImportResult, CollisionStrategy } from "@/ipc/importExport/types";

// ── cURL parser (duplicated from renderer/lib/curlParser.ts for main process use) ──

const SKIP_HEADERS = new Set([
  "host", "proxy-connection", "connection", "content-length", "transfer-encoding",
]);

function tokenizeCurl(input: string): string[] {
  const str = input.replace(/\\\r?\n/g, " ");
  const tokens: string[] = [];
  let i = 0;
  while (i < str.length) {
    while (i < str.length && /[ \t]/.test(str[i])) i++;
    if (i >= str.length) break;
    if (str[i] === "'") {
      i++;
      let tok = "";
      while (i < str.length && str[i] !== "'") tok += str[i++];
      i++;
      tokens.push(tok);
    } else if (str[i] === '"') {
      i++;
      let tok = "";
      while (i < str.length && str[i] !== '"') {
        if (str[i] === "\\" && i + 1 < str.length) { i++; tok += str[i]; }
        else tok += str[i];
        i++;
      }
      i++;
      tokens.push(tok);
    } else {
      let tok = "";
      while (i < str.length && !/[ \t]/.test(str[i])) tok += str[i++];
      tokens.push(tok);
    }
  }
  return tokens;
}

function parseSingleCurl(curlStr: string): { method: string; url: string; headers: Record<string, string>; body: string } | null {
  const tokens = tokenizeCurl(curlStr.trim());
  if (!tokens.length || tokens[0] !== "curl") return null;
  let method = "";
  let url = "";
  const headers: Record<string, string> = {};
  let body = "";
  let i = 0;
  while (i < tokens.length) {
    const tok = tokens[i];
    if (tok === "curl") { i++; continue; }
    if (tok === "-X" || tok === "--request") {
      if (++i < tokens.length) method = tokens[i].toUpperCase();
    } else if (tok === "-H" || tok === "--header") {
      if (++i < tokens.length) {
        const ci = tokens[i].indexOf(":");
        if (ci > 0) {
          const k = tokens[i].slice(0, ci).trim().toLowerCase();
          if (!SKIP_HEADERS.has(k)) headers[k] = tokens[i].slice(ci + 1).trim();
        }
      }
    } else if (tok === "-d" || tok === "--data" || tok === "--data-raw" || tok === "--data-binary") {
      if (++i < tokens.length) body = tokens[i];
    } else if (tok === "--url") {
      if (++i < tokens.length) url = tokens[i];
    } else if (!tok.startsWith("-") && !url && (tok.startsWith("http://") || tok.startsWith("https://"))) {
      url = tok;
    }
    i++;
  }
  if (!url) return null;
  if (!method) method = body ? "POST" : "GET";
  return { method, url, headers, body };
}

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

export function preflight(_wsId: string, filePath: string): PreflightResult {
  try {
    const text = fs.readFileSync(filePath, "utf-8");
    const blocks = splitCurlBlocks(text);
    const valid = blocks.filter((b) => parseSingleCurl(b) !== null);
    return { ok: true, filePath, itemCount: valid.length, collisionIds: [] };
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
    const text = fs.readFileSync(filePath, "utf-8");
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

    reloadConfig();
    return { ok: true, imported };
  } catch (err) {
    return { ok: false, error: String(err) };
  }
}
