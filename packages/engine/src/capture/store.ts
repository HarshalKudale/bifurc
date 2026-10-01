import * as fs from "fs";
import * as path from "path";
import { EngineError } from "@bifurc/protocol";
import { logEmitter, type RequestLogEntry } from "../kernel/telemetry";
import { dataDir } from "../workspace/paths";

/**
 * The captured-traffic history, per workspace. **The engine records it**, whether or not any client is
 * watching: it used to live in the renderer's localStorage, filled only while the Capture panel was mounted,
 * so traffic that passed while the user was on another panel was never captured, and no other client (CLI,
 * another UI) could read it at all.
 *
 * Newest first, capped at {@link MAX_CAPTURE_ENTRIES}. Persisted under `<dataDir>/.capture/`, outside the
 * git-tracked workspace directories: captured traffic carries real headers and bodies, and must never be
 * committed with the workspace.
 */
export const MAX_CAPTURE_ENTRIES = 200;
const WRITE_DELAY_MS = 500;

const entriesByWs = new Map<string, RequestLogEntry[]>();
const pendingWrites = new Map<string, ReturnType<typeof setTimeout>>();

function captureFile(wsId: string): string {
  return path.join(dataDir(), ".capture", `${encodeURIComponent(wsId)}.json`);
}

function entriesOf(wsId: string): RequestLogEntry[] {
  let entries = entriesByWs.get(wsId);
  if (entries === undefined) {
    try {
      const parsed = JSON.parse(fs.readFileSync(captureFile(wsId), "utf8"));
      entries = Array.isArray(parsed) ? (parsed as RequestLogEntry[]).slice(0, MAX_CAPTURE_ENTRIES) : [];
    } catch {
      entries = [];
    }
    entriesByWs.set(wsId, entries);
  }
  return entries;
}

function writeNow(wsId: string): void {
  pendingWrites.delete(wsId);
  const file = captureFile(wsId);
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(entriesByWs.get(wsId) ?? []));
  } catch (err) {
    console.warn(`[capture] could not persist ${file}: ${(err as Error).message}`);
  }
}

/** Traffic arrives in bursts; coalesce the writes. The timer is unref'd so it never holds the process open. */
function scheduleWrite(wsId: string): void {
  if (pendingWrites.has(wsId)) return;
  const timer = setTimeout(() => writeNow(wsId), WRITE_DELAY_MS);
  timer.unref?.();
  pendingWrites.set(wsId, timer);
}

/** Record one exchange. An entry with no workspace (a host serving none) is not attributable, so not kept. */
export function recordCapture(entry: RequestLogEntry): void {
  if (!entry.workspaceId) return;
  const entries = entriesOf(entry.workspaceId).filter((e) => e.id !== entry.id);
  entries.unshift(entry);
  entries.length = Math.min(entries.length, MAX_CAPTURE_ENTRIES);
  entriesByWs.set(entry.workspaceId, entries);
  scheduleWrite(entry.workspaceId);
}

/** The workspace's captured exchanges, newest first. */
export function listCapture(wsId: string): RequestLogEntry[] {
  return [...entriesOf(wsId)];
}

export function getCapture(wsId: string, id: string): RequestLogEntry | undefined {
  return entriesOf(wsId).find((e) => e.id === id);
}

/** The entry a `capture.build*` call names: the whole `entry`, or a `{workspaceId, id}` reference into the store. */
export function resolveCaptureEntry(
  params: { entry: Record<string, unknown> } | { workspaceId: string; id: string },
): RequestLogEntry {
  if ("entry" in params) return params.entry as unknown as RequestLogEntry;
  const found = getCapture(params.workspaceId, params.id);
  if (!found) {
    throw new EngineError("NOT_FOUND", `No captured entry "${params.id}" in workspace "${params.workspaceId}".`);
  }
  return found;
}

export function clearCapture(wsId: string): void {
  entriesByWs.set(wsId, []);
  scheduleWrite(wsId);
}

/** Drop the given exchanges; returns how many were present. */
export function removeCapture(wsId: string, ids: readonly string[]): number {
  const drop = new Set(ids);
  const before = entriesOf(wsId);
  const after = before.filter((e) => !drop.has(e.id));
  entriesByWs.set(wsId, after);
  if (after.length !== before.length) scheduleWrite(wsId);
  return before.length - after.length;
}

/** Start recording every captured exchange. Returns the undo, which also flushes pending writes. */
export function startCaptureRecording(source = logEmitter): () => void {
  source.on("request", recordCapture);
  return () => {
    source.off("request", recordCapture);
    flushCapture();
  };
}

/** Write every pending workspace now (shutdown, tests). */
export function flushCapture(): void {
  for (const [wsId, timer] of [...pendingWrites]) {
    clearTimeout(timer);
    writeNow(wsId);
  }
}

/** Forget the in-memory state, so the next read comes from disk (tests, data-root changes). */
export function resetCaptureForTests(): void {
  for (const timer of pendingWrites.values()) clearTimeout(timer);
  pendingWrites.clear();
  entriesByWs.clear();
}
