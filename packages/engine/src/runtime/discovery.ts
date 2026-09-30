import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

/**
 * How a client finds the engine that serves a data directory.
 *
 * One engine owns one data directory. It publishes where it listens in `<dataDir>/runtime.json` while it
 * runs, and its default socket address is derived from the data directory (not its pid), so a client that
 * knows only the data directory — the desktop shell, the CLI, a future native UI — can always find it.
 */

export type EngineTransportKind = "socket" | "ws" | "stdio";

export interface EngineRuntimeInfo {
  pid: number;
  transport: EngineTransportKind;
  /** A socket/pipe path, a `ws://` URL, or `"stdio"`. */
  address: string;
  dataDir: string;
  protocolVersion: string;
  engineVersion: string;
  startedAt: number;
}

export const RUNTIME_FILE = "runtime.json";

function dataDirKey(dataDir: string): string {
  const resolved = path.resolve(dataDir);
  const normalized = process.platform === "win32" ? resolved.toLowerCase() : resolved;
  return createHash("sha256").update(normalized).digest("hex").slice(0, 16);
}

/**
 * The engine's default RPC address for a data directory: a named pipe on Windows, a unix socket elsewhere.
 * Stable across restarts. The unix socket lives in the runtime dir rather than the data dir because socket
 * paths are length-limited (~104 bytes on macOS) and data dirs can be long.
 */
export function engineSocketPath(dataDir: string): string {
  const key = dataDirKey(dataDir);
  if (process.platform === "win32") return `\\\\.\\pipe\\bifurc-${key}`;
  const runtimeDir = process.env.XDG_RUNTIME_DIR;
  return path.join(runtimeDir !== undefined && runtimeDir.length > 0 ? runtimeDir : os.tmpdir(), `bifurc-${key}.sock`);
}

export function runtimeFilePath(dataDir: string): string {
  return path.join(dataDir, RUNTIME_FILE);
}

export function writeRuntimeInfo(info: EngineRuntimeInfo): void {
  fs.mkdirSync(info.dataDir, { recursive: true });
  fs.writeFileSync(runtimeFilePath(info.dataDir), JSON.stringify(info, null, 2), { mode: 0o600 });
}

export function readRuntimeInfo(dataDir: string): EngineRuntimeInfo | null {
  try {
    const parsed = JSON.parse(fs.readFileSync(runtimeFilePath(dataDir), "utf8")) as EngineRuntimeInfo;
    return typeof parsed.pid === "number" && typeof parsed.address === "string" ? parsed : null;
  } catch {
    return null;
  }
}

/** Remove the file, but only if it still describes `pid` — a newer engine may already have replaced it. */
export function removeRuntimeInfo(dataDir: string, pid: number = process.pid): void {
  if (readRuntimeInfo(dataDir)?.pid !== pid) return;
  try {
    fs.unlinkSync(runtimeFilePath(dataDir));
  } catch {
    /* already gone */
  }
}

export function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    // EPERM: the process exists but belongs to someone else — still alive.
    return (err as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** The engine currently serving `dataDir`, or `null` if none is (a stale file from a crash counts as none). */
export function findRunningEngine(dataDir: string): EngineRuntimeInfo | null {
  const info = readRuntimeInfo(dataDir);
  return info !== null && isProcessAlive(info.pid) ? info : null;
}
