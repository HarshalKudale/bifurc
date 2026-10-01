import type { LogEntryEvent } from "@bifurc/protocol";
import type { RequestLogEntry } from "../kernel/telemetry";

/**
 * Project an engine `RequestLogEntry` onto the protocol's wire shape (`LogEntryEvent`). Lives in `utils/`
 * so both the event pump and `capture.list`'s summary mode share it. Defensive: any field may be missing.
 */
export function toLogEntryEvent(payload: unknown): LogEntryEvent {
  const e = (typeof payload === "object" && payload !== null ? payload : {}) as Partial<RequestLogEntry>;
  const out: LogEntryEvent = {
    id: typeof e.id === "string" ? e.id : "",
    timestamp: typeof e.ts === "number" ? e.ts : 0,
    method: typeof e.method === "string" ? e.method : "",
    url: typeof e.url === "string" ? e.url : "",
    status: typeof e.status === "number" ? e.status : null,
    durationMs: typeof e.durationMs === "number" ? e.durationMs : null,
  };
  if (typeof e.workspaceId === "string") out.workspaceId = e.workspaceId;
  if (typeof e.via === "string") out.via = e.via;
  if (typeof e.target === "string" || e.target === null) out.target = e.target;
  const contentType = headerValue(e.resHeaders, "content-type");
  if (contentType !== undefined) out.contentType = contentType;
  out.size = base64Size(e.resBody);
  return out;
}

function headerValue(headers: unknown, name: string): string | undefined {
  if (typeof headers !== "object" || headers === null) return undefined;
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === name && typeof v === "string") return v;
  }
  return undefined;
}

/** Decoded byte length of a base64 string, without decoding it. */
function base64Size(b64: unknown): number {
  if (typeof b64 !== "string" || b64.length === 0) return 0;
  let padding = 0;
  if (b64.endsWith("==")) padding = 2;
  else if (b64.endsWith("=")) padding = 1;
  return Math.max(0, Math.floor((b64.length * 3) / 4) - padding);
}
