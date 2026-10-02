import { randomBytes } from "crypto";
import type { Environment } from "../workspace/types";
import { resolveVars } from "../env/resolve";

/** One field of a `multipart/form-data` body. `value` is text for `kind: "text"`, base64 bytes for `"file"`. */
export interface RequestBodyPart {
  name: string;
  kind: "text" | "file";
  value: string;
  fileName?: string;
  contentType?: string;
}

/** The body-related fields shared by `SavedRequest`, `RequestSendRequest` and `PipelineRequest`. */
export interface RequestBodyFields {
  headers: Record<string, string>;
  body?: string;
  bodyEncoding?: "utf8" | "base64";
  bodyParts?: RequestBodyPart[];
}

function hasParts(r: RequestBodyFields): r is RequestBodyFields & { bodyParts: RequestBodyPart[] } {
  return Array.isArray(r.bodyParts) && r.bodyParts.length > 0;
}

/**
 * Resolve `{{variables}}` in the body fields: text bodies, part names, text-part values and file names.
 * Base64 data (a base64 body, a file part's bytes) is never touched.
 */
export function resolveRequestBody(
  request: { body?: string; bodyEncoding?: "utf8" | "base64"; bodyParts?: RequestBodyPart[] },
  env: Environment | null,
): { body: string; bodyEncoding?: "utf8" | "base64"; bodyParts?: RequestBodyPart[] } {
  const out: { body: string; bodyEncoding?: "utf8" | "base64"; bodyParts?: RequestBodyPart[] } = {
    body: request.bodyEncoding === "base64" ? (request.body ?? "") : resolveVars(request.body ?? "", env),
  };
  if (request.bodyEncoding) out.bodyEncoding = request.bodyEncoding;
  if (Array.isArray(request.bodyParts)) {
    out.bodyParts = request.bodyParts.map((p) => {
      const part: RequestBodyPart = { ...p, name: resolveVars(p.name, env) };
      if (p.kind === "text") part.value = resolveVars(p.value, env);
      if (p.fileName !== undefined) part.fileName = resolveVars(p.fileName, env);
      return part;
    });
  }
  return out;
}

function quoteParam(s: string): string {
  return s.replace(/[\r\n]/g, "").replace(/"/g, "%22");
}

function buildMultipart(parts: RequestBodyPart[]): { payload: Buffer; contentType: string } {
  const boundary = `----bifurc${randomBytes(12).toString("hex")}`;
  const chunks: Buffer[] = [];
  for (const p of parts) {
    let head = `--${boundary}\r\nContent-Disposition: form-data; name="${quoteParam(p.name)}"`;
    if (p.kind === "file") {
      head += `; filename="${quoteParam(p.fileName ?? "")}"\r\nContent-Type: ${p.contentType || "application/octet-stream"}`;
    }
    head += "\r\n\r\n";
    chunks.push(Buffer.from(head, "utf8"));
    chunks.push(p.kind === "file" ? Buffer.from(p.value, "base64") : Buffer.from(p.value, "utf8"));
    chunks.push(Buffer.from("\r\n", "utf8"));
  }
  chunks.push(Buffer.from(`--${boundary}--\r\n`, "utf8"));
  return { payload: Buffer.concat(chunks), contentType: `multipart/form-data; boundary=${boundary}` };
}

/**
 * The one place a request's body becomes wire bytes: `bodyParts` (multipart) wins, then a base64 body,
 * then a text body (a blank text body sends nothing). Returns the base64 `request.replay` expects and the
 * headers to send (the content-type is replaced for multipart). Variables must already be resolved.
 */
export function encodeRequestBody(request: RequestBodyFields): { bodyBase64: string; headers: Record<string, string> } {
  if (hasParts(request)) {
    const { payload, contentType } = buildMultipart(request.bodyParts);
    const headers: Record<string, string> = {};
    for (const [k, v] of Object.entries(request.headers ?? {})) {
      if (k.toLowerCase() !== "content-type") headers[k] = v;
    }
    headers["content-type"] = contentType;
    return { bodyBase64: payload.toString("base64"), headers };
  }
  const headers = { ...(request.headers ?? {}) };
  const body = request.body ?? "";
  if (request.bodyEncoding === "base64") return { bodyBase64: body, headers };
  if (!body.trim()) return { bodyBase64: "", headers };
  return { bodyBase64: Buffer.from(body, "utf-8").toString("base64"), headers };
}
