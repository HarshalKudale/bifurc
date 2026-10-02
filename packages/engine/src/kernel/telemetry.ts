import { EventEmitter } from "events";

export interface RequestLogEntry {
    id: string;
    ts: number;
    method: string;
    url: string;
    host: string;
    status: number | null;
    /**
     * How the request was fulfilled.
     *
     * `rfc6761` and `mapping` are the **same code path** — the edge's branch 2, where the request's
     * `Host` matched a mapping — split into two labels because both are true and they say different
     * things. A `.localhost` name works with no setup at all (RFC 6761 resolves it to the *browser's*
     * loopback), while any other domain works because someone pointed that domain at this engine.
     * `mapping` arrived with `plan/15` work item 4, which is what made the second case reachable:
     * before it, the edge only routed a host whose name ended in `.localhost`.
     *
     * This field is **engine-local**. It is not in `@bifurc/protocol`'s `LogEntryEventSchema`
     * (`packages/protocol/src/events.ts`), and the remote transports project an entry down to
     * `{id, timestamp, method, url, status, durationMs}` before it goes on the wire
     * (`transport/eventPump.ts`). The Electron host forwards this interface over IPC unchanged
     * instead, which is why the two hosts' capture panels do not see the same fields —
     * `packages/gui/src/types/ipc.ts` mirrors this union **by hand**. Widening it therefore costs a
     * mirror and a `ViaBadge` label, and no protocol change.
     */
    via: "rfc6761" | "mapping" | "proxy" | "rule" | "mock" | "error";
    target: string | null;
    durationMs: number | null;
    // full capture for replay / mock
    reqHeaders: Record<string, string>;
    reqBody: string;      // base64
    resHeaders: Record<string, string>;
    resBody: string;      // base64, first 512 KB
    resStatus: number | null;
    /** The workspace whose config served this request — what the capture store files it under. */
    workspaceId?: string;
    /** Which listener took it: the forward proxy (`forward`) or the reverse proxy serving mapped domains (`reverse`). */
    origin?: "forward" | "reverse";
}

export const logEmitter = new EventEmitter();

export function emitLog(entry: RequestLogEntry): void {
    logEmitter.emit("request", entry);
}

export function emitLogChunk(logId: string, chunk: Buffer, done: boolean): void {
    logEmitter.emit("chunk", { logId, chunk: chunk.toString("base64"), done });
}
