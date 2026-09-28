export type BodyMode = "json" | "text" | "html" | "xml" | "form" | "multipart" | "binary" | "image" | "none";

export interface KVPair {
  key: string;
  value: string;
}

export interface MultipartPart {
  key: string;
  type: "text" | "file";
  value: string;          // text content or base64 for files
  fileName?: string;
  mimeType?: string;
}

export function contentTypeToMode(ct: string | undefined | null): BodyMode {
  if (!ct) return "json";
  const lower = ct.toLowerCase();
  if (lower.includes("application/json")) return "json";
  if (lower.includes("application/x-www-form-urlencoded")) return "form";
  if (lower.includes("multipart/form-data")) return "multipart";
  if (lower.includes("text/html")) return "html";
  if (lower.includes("text/xml") || lower.includes("application/xml") || lower.includes("application/xhtml")) return "xml";
  if (lower.includes("text/plain")) return "text";
  if (isImageContentType(lower)) return "image";
  if (isBinaryContentType(lower)) return "binary";
  return "text";
}

export function modeToContentType(mode: BodyMode): string | null {
  switch (mode) {
    case "json": return "application/json";
    case "text": return "text/plain";
    case "html": return "text/html";
    case "xml":  return "application/xml";
    case "form": return "application/x-www-form-urlencoded";
    case "multipart": return "multipart/form-data";
    case "binary": return "application/octet-stream";
    case "image": return "image/png";
    case "none": return null;
  }
}

export function isBinaryContentType(ct: string): boolean {
  const lower = ct.toLowerCase();
  return (
    lower.includes("image/") ||
    lower.includes("audio/") ||
    lower.includes("video/") ||
    lower.includes("font/") ||
    lower.includes("application/octet-stream") ||
    lower.includes("application/pdf") ||
    lower.includes("application/zip") ||
    lower.includes("application/gzip")
  );
}

export function isImageContentType(ct: string): boolean {
  return ct.toLowerCase().includes("image/");
}

export function getImageDataUrl(base64: string, contentType: string): string {
  return `data:${contentType};base64,${base64}`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function parseFormBody(body: string): KVPair[] {
  if (!body.trim()) return [];
  try {
    return body.split("&")
      .map((pair) => {
        const eq = pair.indexOf("=");
        if (eq === -1) return { key: decodeURIComponent(pair.trim()), value: "" };
        return {
          key: decodeURIComponent(pair.slice(0, eq).trim()),
          value: decodeURIComponent(pair.slice(eq + 1)),
        };
      })
      .filter((p) => p.key !== "");
  } catch {
    return [];
  }
}

export function serializeFormBody(pairs: KVPair[]): string {
  return pairs
    .filter((p) => p.key.trim() !== "")
    .map((p) => `${encodeURIComponent(p.key.trim())}=${encodeURIComponent(p.value)}`)
    .join("&");
}
