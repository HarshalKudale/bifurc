import { EventEmitter } from "events";

export interface RequestLogEntry {
    id: string;
    ts: number;
    method: string;
    url: string;
    host: string;
    status: number | null;
    via: "rfc6761" | "proxy" | "rule" | "mock" | "error";
    target: string | null;
    durationMs: number | null;
    // full capture for replay / mock
    reqHeaders: Record<string, string>;
    reqBody: string;      // base64
    resHeaders: Record<string, string>;
    resBody: string;      // base64, first 512 KB
    resStatus: number | null;
}

export const logEmitter = new EventEmitter();

export function emitLog(entry: RequestLogEntry): void {
    logEmitter.emit("request", entry);
}

export function emitLogChunk(logId: string, chunk: Buffer, done: boolean): void {
    logEmitter.emit("chunk", { logId, chunk: chunk.toString("base64"), done });
}
