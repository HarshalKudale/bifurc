import { describe, it, expect, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { setDataRoot, resetDataRootForTests } from "@bifurc/engine/workspace/paths";
import { emitLog, type RequestLogEntry } from "@bifurc/engine/kernel/telemetry";
import {
  MAX_CAPTURE_ENTRIES, clearCapture, flushCapture, isCapturePaused, listCapture, removeCapture, resetCaptureForTests,
  setCapturePaused, startCaptureRecording,
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

  it("drops entries for a paused workspace only, and records again once resumed", () => {
    emitLog(entry("before", "ws1"));
    setCapturePaused("ws1", true);
    expect(isCapturePaused("ws1")).toBe(true);
    expect(isCapturePaused("ws2")).toBe(false);
    emitLog(entry("while-paused", "ws1"));
    emitLog(entry("other", "ws2"));
    expect(listCapture("ws1").map((e) => e.id)).toEqual(["before"]);
    expect(listCapture("ws2").map((e) => e.id)).toEqual(["other"]);

    setCapturePaused("ws1", false);
    emitLog(entry("after", "ws1"));
    expect(listCapture("ws1").map((e) => e.id)).toEqual(["after", "before"]);
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

  it("setPaused stops recording, list reports it, and get/remove/clear keep working", () => {
    const registry = new CommandRegistry();
    registerCaptureCommands(registry);
    const ctx = { bus };
    emitLog(entry("a", "ws1"));
    expect(registry.invoke("capture.setPaused", { workspaceId: "ws1", paused: true }, ctx)).toEqual({ paused: true });
    emitLog(entry("b", "ws1"));
    const listed = registry.invoke("capture.list", { workspaceId: "ws1" }, ctx) as { entries: RequestLogEntry[]; paused: boolean };
    expect(listed.paused).toBe(true);
    expect(listed.entries.map((e) => e.id)).toEqual(["a"]);
    expect((registry.invoke("capture.get", { workspaceId: "ws1", id: "a" }, ctx) as { entry: RequestLogEntry }).entry.id).toBe("a");
    expect(registry.invoke("capture.clear", { workspaceId: "ws1" }, ctx)).toEqual({ ok: true });
    expect(registry.invoke("capture.setPaused", { workspaceId: "ws1", paused: false }, ctx)).toEqual({ paused: false });
    expect((registry.invoke("capture.list", { workspaceId: "ws1" }, ctx) as { paused: boolean }).paused).toBe(false);
    expect(() => registry.invoke("capture.setPaused", { workspaceId: "ws1" }, ctx)).toThrow(/Invalid payload/);
  });

  it("list summary returns the wire shape; get returns the full entry or null; build* accept a reference", async () => {
    const registry = new CommandRegistry();
    registerCaptureCommands(registry);
    const { registerRestCommands } = await import("@bifurc/engine/rest/commands");
    const { registerMockCommands } = await import("@bifurc/engine/mocks/commands");
    registerRestCommands(registry);
    registerMockCommands(registry);
    const ctx = { bus };
    emitLog({ ...entry("a", "ws1"), resHeaders: { "Content-Type": "text/plain" }, resBody: "AAEC" });

    const summary = (registry.invoke("capture.list", { workspaceId: "ws1", summary: true }, ctx) as { entries: Record<string, unknown>[] }).entries;
    expect(summary).toEqual([{
      id: "a", timestamp: 1, method: "GET", url: "http://x/a", status: 200, durationMs: 1,
      workspaceId: "ws1", via: "proxy", target: null, contentType: "text/plain", size: 3,
    }]);

    const full = registry.invoke("capture.get", { workspaceId: "ws1", id: "a" }, ctx) as { entry: RequestLogEntry };
    expect(full.entry.resBody).toBe("AAEC");
    expect(registry.invoke("capture.get", { workspaceId: "ws1", id: "nope" }, ctx)).toEqual({ entry: null });

    expect((registry.invoke("capture.buildRequest", { workspaceId: "ws1", id: "a" }, ctx) as { request: { url: string } }).request.url)
      .toBe("http://x/a");
    expect((registry.invoke("capture.buildMock", { workspaceId: "ws1", id: "a" }, ctx) as { mock: { urlPattern: string } }).mock.urlPattern)
      .toBe("http://x/a");
    expect(() => registry.invoke("capture.buildMock", { workspaceId: "ws1", id: "nope" }, ctx)).toThrow(/No captured entry/);
    expect(() => registry.invoke("capture.buildMock", { entryId: "a" }, ctx)).toThrow(/Invalid payload/);
  });
});
