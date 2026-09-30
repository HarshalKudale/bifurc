import { describe, it, expect, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { setDataRoot, resetDataRootForTests } from "@bifurc/engine/workspace/paths";
import { emitLog, type RequestLogEntry } from "@bifurc/engine/kernel/telemetry";
import {
  MAX_CAPTURE_ENTRIES, clearCapture, flushCapture, listCapture, removeCapture, resetCaptureForTests,
  startCaptureRecording,
} from "@bifurc/engine/capture/store";
import { CommandRegistry } from "@bifurc/engine/kernel/registry";
import { bus } from "@bifurc/engine/kernel/eventBus";
import { registerCaptureCommands } from "@bifurc/engine/capture/commands";

// The capture history is engine state: it records every exchange whether or not a UI is open, files it under
// the workspace that served it, and persists outside the git-tracked workspace directories.

const entry = (id: string, workspaceId?: string): RequestLogEntry =>
  ({
    id, ts: 1, method: "GET", url: `http://x/${id}`, host: "x", status: 200, via: "proxy", target: null,
    durationMs: 1, reqHeaders: {}, reqBody: "", resHeaders: {}, resBody: "", resStatus: 200, workspaceId,
  }) as RequestLogEntry;

let root: string;
let stop: () => void;

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-capture-"));
  setDataRoot(root);
  resetCaptureForTests();
  stop = startCaptureRecording();
});

afterEach(() => {
  stop();
  resetCaptureForTests();
  resetDataRootForTests();
  fs.rmSync(root, { recursive: true, force: true });
});

describe("capture store", () => {
  it("records what the proxy emits, newest first, under the workspace that served it", () => {
    emitLog(entry("a", "ws1"));
    emitLog(entry("b", "ws1"));
    emitLog(entry("c", "ws2"));
    expect(listCapture("ws1").map((e) => e.id)).toEqual(["b", "a"]);
    expect(listCapture("ws2").map((e) => e.id)).toEqual(["c"]);
  });

  it("keeps nothing it cannot attribute to a workspace", () => {
    emitLog(entry("orphan"));
    expect(listCapture("ws1")).toEqual([]);
  });

  it("holds at most the cap, dropping the oldest", () => {
    for (let i = 0; i < MAX_CAPTURE_ENTRIES + 5; i++) emitLog(entry(`e${i}`, "ws1"));
    const ids = listCapture("ws1").map((e) => e.id);
    expect(ids).toHaveLength(MAX_CAPTURE_ENTRIES);
    expect(ids[0]).toBe(`e${MAX_CAPTURE_ENTRIES + 4}`);
    expect(ids).not.toContain("e0");
  });

  it("persists under <dataDir>/.capture, never inside the workspace's git directory, and reloads from it", () => {
    emitLog(entry("a", "ws1"));
    flushCapture();
    expect(fs.existsSync(path.join(root, ".capture", "ws1.json"))).toBe(true);
    expect(fs.existsSync(path.join(root, "ws1"))).toBe(false);

    resetCaptureForTests();
    expect(listCapture("ws1").map((e) => e.id)).toEqual(["a"]);
  });

  it("removes by id and clears", () => {
    emitLog(entry("a", "ws1"));
    emitLog(entry("b", "ws1"));
    expect(removeCapture("ws1", ["a", "missing"])).toBe(1);
    expect(listCapture("ws1").map((e) => e.id)).toEqual(["b"]);
    clearCapture("ws1");
    expect(listCapture("ws1")).toEqual([]);
  });

  it("stops recording once undone", () => {
    stop();
    emitLog(entry("late", "ws1"));
    expect(listCapture("ws1")).toEqual([]);
    stop = startCaptureRecording();
  });
});

describe("capture.* commands", () => {
  it("list, remove and clear through the registry with protocol validation", () => {
    const registry = new CommandRegistry();
    registerCaptureCommands(registry);
    const ctx = { bus };
    emitLog(entry("a", "ws1"));
    emitLog(entry("b", "ws1"));

    expect((registry.invoke("capture.list", { workspaceId: "ws1" }, ctx) as { entries: RequestLogEntry[] }).entries)
      .toHaveLength(2);
    expect(registry.invoke("capture.remove", { workspaceId: "ws1", ids: ["a"] }, ctx)).toEqual({ ok: true, removed: 1 });
    expect(registry.invoke("capture.clear", { workspaceId: "ws1" }, ctx)).toEqual({ ok: true });
    expect(() => registry.invoke("capture.remove", { workspaceId: "ws1", ids: [] }, ctx)).toThrow(/Invalid payload/);
  });
});
