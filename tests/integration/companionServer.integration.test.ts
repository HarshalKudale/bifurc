/**
 * Companion WebSocket server — the browser extension's bridge into the app.
 *
 * WHY THIS EXISTS
 * ---------------
 * `packages/engine/src/transport/legacyCompanion.ts` (moved there from
 * `src/companion/companionServer.ts` by P4 work item 1) was at 0%. It is the *only* code path by which a
 * lower-trust caller (a browser extension, i.e. anything running in the user's browser)
 * can write into the user's workspace, and `V1_COMPANION_ACTIONS` is the single gate that
 * keeps that surface additive-only. The allowlist itself was pinned by a unit test, but
 * nothing proved the server *enforces* it.
 *
 * That additive-only property is why this file's contract matters beyond the extension: it is
 * the reason the four v1 actions could not simply be aliased onto `entity.create`/`folder.add`
 * when the server moved — the registry's mock create can *disable an existing mock*, which is a
 * mutation of existing state. See `legacyCompanion.ts`'s header for the four verified differences.
 *
 * This suite starts the real `WebSocketServer`, connects real `ws` clients, and asserts
 * against the real files on disk. Nothing is mocked except Electron itself (a browser
 * window, so the renderer-notification contract can be observed).
 */

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import { WebSocket } from "ws";
import { simpleGit } from "simple-git";
import { BrowserWindow } from "electron";

import { createWorkspace, getFreePort, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { setDataDirOverride } from "@bifurc/engine/store/gitStore";
import { wireEventBridge } from "@/ipc/eventBridge";
import {
  startCompanionServer,
  stopCompanionServer,
  restartCompanionServer,
  getCompanionPort,
  isCompanionRunning,
} from "@bifurc/engine/transport/legacyCompanion";

// ── Fixture ───────────────────────────────────────────────────────────────────

let ws: WorkspaceFixture | null = null;
let port = 0;

function wsPath(...parts: string[]): string {
  return path.join(ws!.dataRoot, TEST_WS, ...parts);
}

function readJson<T = any>(...parts: string[]): T {
  return JSON.parse(fs.readFileSync(wsPath(...parts), "utf-8")) as T;
}

/** Give the workspace a real git repo, so `getWorkspaceSyncStatus()` has something to report. */
async function initRepo(): Promise<void> {
  const g = simpleGit(wsPath());
  await g.init();
  await g.addConfig("user.email", "test@bifurc", false, "local");
  await g.addConfig("user.name", "Test Device", false, "local");
  await g.add("workspace.json");
  await g.commit("chore: init workspace repo");
}

beforeEach(async () => {
  // companionServer.ts emits on the engine bus (P2); this test observes "the renderer
  // contract" — what the shell's temporary eventBridge.ts forwards to webContents — so it
  // must wire that bridge itself, same as `registerIpcHandlers()` does in production.
  wireEventBridge();
  ws = createWorkspace({});
  // Clears the git cache and repoints the data root at the fresh temp workspace.
  setDataDirOverride(ws.dataRoot);
  await initRepo();
  port = await getFreePort();
  startCompanionServer(port);
});

afterEach(() => {
  stopCompanionServer();
  setDataDirOverride("");
  ws?.cleanup();
  ws = null;
  vi.mocked(BrowserWindow.getAllWindows).mockReset();
  vi.mocked(BrowserWindow.getAllWindows).mockReturnValue([]);
});

// ── Client plumbing ───────────────────────────────────────────────────────────

function connectClient(): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const client = new WebSocket(`ws://127.0.0.1:${port}`);
    client.once("open", () => resolve(client));
    client.once("error", reject);
  });
}

/** Send one message and resolve with the single reply. */
function ask(client: WebSocket, msg: unknown): Promise<any> {
  return new Promise((resolve, reject) => {
    const onMessage = (raw: Buffer): void => {
      client.off("message", onMessage);
      resolve(JSON.parse(raw.toString()));
    };
    client.on("message", onMessage);
    client.once("error", reject);
    client.send(JSON.stringify(msg));
  });
}

/** Poll until `predicate` holds, so async broadcasts can be observed deterministically. */
async function waitFor(predicate: () => boolean, timeoutMs = 4000): Promise<void> {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > timeoutMs) throw new Error("waitFor timed out");
    await new Promise((r) => setTimeout(r, 20));
  }
}

function fakeWindow(): { isDestroyed: () => boolean; webContents: { send: ReturnType<typeof vi.fn> } } {
  return { isDestroyed: () => false, webContents: { send: vi.fn() } };
}

const validMockPayload = {
  name: "Extension Mock",
  method: "GET",
  urlPattern: "https://api.example.com/users",
  useRegex: false,
  responseStatus: 200,
  responseHeaders: { "content-type": "application/json" },
  responseBody: JSON.stringify({ ok: true }),
  responseBodyEncoding: "utf8",
  responseDelay: 0,
  streamingMode: "none",
  folderId: null,
};

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("companion server — lifecycle", () => {
  it("starts on the requested port and reports itself running", () => {
    expect(isCompanionRunning()).toBe(true);
    expect(getCompanionPort()).toBe(port);
  });

  it("accepts a real WebSocket connection", async () => {
    const client = await connectClient();
    expect(client.readyState).toBe(WebSocket.OPEN);
    client.close();
  });

  it("keeps serving after a restart", async () => {
    const next = await getFreePort();
    restartCompanionServer(next);
    port = next;

    expect(isCompanionRunning()).toBe(true);
    expect(getCompanionPort()).toBe(next);

    const client = await connectClient();
    const reply = await ask(client, { id: "1", action: "config:get" });
    expect(reply.ok).toBe(true);
    client.close();
  });

  it("stops, reports itself stopped, and closes live clients", async () => {
    const client = await connectClient();
    const closed = new Promise<number>((resolve) => client.once("close", (code) => resolve(code)));

    stopCompanionServer();

    expect(isCompanionRunning()).toBe(false);
    await expect(closed).resolves.toBe(1001);
  });
});

describe("companion server — action dispatch", () => {
  it("serves the active workspace through config:get", async () => {
    const client = await connectClient();
    const reply = await ask(client, { id: "cfg", action: "config:get" });

    expect(reply).toMatchObject({ id: "cfg", ok: true });
    expect(reply.data.activeWorkspaceId).toBe(TEST_WS);
    client.close();
  });

  it("persists a mock added through mock:add", async () => {
    const client = await connectClient();
    const reply = await ask(client, { id: "m1", action: "mock:add", payload: validMockPayload });

    expect(reply.ok).toBe(true);
    expect(typeof reply.data.id).toBe("string");
    expect(typeof reply.data.createdAt).toBe("number");
    expect(reply.data.workspaceId).toBe(TEST_WS);
    expect(reply.data.enabled).toBe(true);

    // The entity file itself...
    const onDisk = readJson("mocks", `${reply.data.id}.json`);
    expect(onDisk.name).toBe("Extension Mock");
    expect(onDisk.urlPattern).toBe(validMockPayload.urlPattern);
    // ...which must NOT carry the enabled flag: enabled state lives in enabled.json.
    expect(onDisk).not.toHaveProperty("enabled");

    // ...the enabled set...
    expect(readJson("mocks", "enabled.json")).toContain(reply.data.id);
    // ...and the sidebar name index.
    expect(readJson("mocks", "names.json")[reply.data.id]).toMatchObject({
      name: "Extension Mock",
      method: "GET",
      url: validMockPayload.urlPattern,
    });

    client.close();
  });

  it("honours enabled:false by leaving the mock out of enabled.json", async () => {
    const client = await connectClient();
    const reply = await ask(client, {
      id: "m2",
      action: "mock:add",
      payload: { ...validMockPayload, enabled: false },
    });

    expect(reply.ok).toBe(true);
    expect(reply.data.enabled).toBe(false);
    expect(readJson("mocks", "enabled.json")).not.toContain(reply.data.id);
    client.close();
  });

  it("rejects a mock with no urlPattern", async () => {
    const client = await connectClient();
    const reply = await ask(client, {
      id: "m3",
      action: "mock:add",
      payload: { ...validMockPayload, urlPattern: "" },
    });

    expect(reply).toMatchObject({ id: "m3", ok: false, error: "urlPattern is required" });
    client.close();
  });

  it("rejects a mock with no method", async () => {
    const client = await connectClient();
    const reply = await ask(client, {
      id: "m4",
      action: "mock:add",
      payload: { ...validMockPayload, method: "" },
    });

    expect(reply).toMatchObject({ id: "m4", ok: false, error: "method is required" });
    client.close();
  });

  it("persists a saved request added through request:add", async () => {
    const client = await connectClient();
    const reply = await ask(client, {
      id: "r1",
      action: "request:add",
      payload: { name: "Extension Request", method: "POST", url: "https://api.example.com/users", folderId: null },
    });

    expect(reply.ok).toBe(true);
    expect(readJson("requests", `${reply.data.id}.json`).url).toBe("https://api.example.com/users");
    expect(readJson("requests", "names.json")[reply.data.id]).toMatchObject({
      name: "Extension Request",
      method: "POST",
    });
    client.close();
  });

  it("rejects a request with no url", async () => {
    const client = await connectClient();
    const reply = await ask(client, {
      id: "r2",
      action: "request:add",
      payload: { name: "Bad", method: "GET", url: "" },
    });

    expect(reply).toMatchObject({ id: "r2", ok: false, error: "url is required" });
    client.close();
  });

  it("creates a folder in index.json and on disk", async () => {
    const client = await connectClient();
    const reply = await ask(client, {
      id: "f1",
      action: "folder:add",
      payload: { kind: "mock", name: "Extension Folder" },
    });

    expect(reply.ok).toBe(true);
    expect(reply.data).toMatchObject({ name: "Extension Folder", parentId: null, workspaceId: TEST_WS });

    const index = readJson<{ folders: Array<{ id: string; name: string }> }>("mocks", "index.json");
    expect(index.folders.map((f) => f.id)).toContain(reply.data.id);
    expect(fs.existsSync(wsPath("mocks", "Extension Folder"))).toBe(true);
    client.close();
  });

  it("rejects an unknown folder kind", async () => {
    const client = await connectClient();
    const reply = await ask(client, {
      id: "f2",
      action: "folder:add",
      payload: { kind: "bogus", name: "Nope" },
    });

    expect(reply).toMatchObject({ id: "f2", ok: false, error: "Unknown folder kind: bogus" });
    client.close();
  });

  it("rejects a folder with no name", async () => {
    const client = await connectClient();
    const reply = await ask(client, {
      id: "f3",
      action: "folder:add",
      payload: { kind: "mock", name: "   " },
    });

    expect(reply).toMatchObject({ id: "f3", ok: false, error: "name is required" });
    client.close();
  });

  it("keeps the connection usable after a failed action", async () => {
    const client = await connectClient();

    const bad = await ask(client, { id: "x1", action: "mock:add", payload: { ...validMockPayload, urlPattern: "" } });
    expect(bad.ok).toBe(false);

    const good = await ask(client, { id: "x2", action: "mock:add", payload: validMockPayload });
    expect(good.ok).toBe(true);

    client.close();
  });
});

describe("companion server — the security boundary", () => {
  it("refuses every action that is not on the allowlist, and writes nothing", async () => {
    const client = await connectClient();
    const before = fs.readdirSync(wsPath("mocks")).sort();

    const forbidden = [
      "entity:delete",
      "workspace:delete",
      "config:set",
      "mock:update",
      "settings:save",
      "file:write",
      "exec:run",
    ];

    for (const action of forbidden) {
      const reply = await ask(client, { id: action, action, payload: {} });
      expect(reply, action).toMatchObject({ id: action, ok: false, error: `Action "${action}" is not allowed` });
    }

    expect(fs.readdirSync(wsPath("mocks")).sort()).toEqual(before);
    client.close();
  });

  it("answers malformed JSON without crashing the connection", async () => {
    const client = await connectClient();
    const reply = await new Promise<any>((resolve, reject) => {
      client.once("message", (raw: Buffer) => resolve(JSON.parse(raw.toString())));
      client.once("error", reject);
      client.send("this is not json");
    });

    expect(reply).toMatchObject({ id: "unknown", ok: false, error: "Invalid JSON" });

    // Still usable afterwards.
    const ok = await ask(client, { id: "after", action: "config:get" });
    expect(ok.ok).toBe(true);
    client.close();
  });

  it("rejects a message with no id", async () => {
    const client = await connectClient();
    const reply = await ask(client, { action: "config:get" });
    expect(reply).toMatchObject({ id: "unknown", ok: false, error: "Missing id or action" });
    client.close();
  });

  it("rejects a message with no action", async () => {
    const client = await connectClient();
    const reply = await ask(client, { id: "abc" });
    expect(reply).toMatchObject({ id: "abc", ok: false, error: "Missing id or action" });
    client.close();
  });
});

describe("companion server — the renderer contract", () => {
  it("tells the renderer to refresh, and reports the new entity as dirty", async () => {
    const win = fakeWindow();
    vi.mocked(BrowserWindow.getAllWindows).mockReturnValue([win as any]);

    const client = await connectClient();
    const reply = await ask(client, { id: "b1", action: "mock:add", payload: validMockPayload });
    expect(reply.ok).toBe(true);

    // The refresh ping is sent synchronously with the action...
    const sent = (channel: string): any[] =>
      win.webContents.send.mock.calls.filter((c) => c[0] === channel).map((c) => c[1]);

    expect(sent("companion:refresh")).toHaveLength(1);

    // ...and the entity-status broadcast is computed asynchronously, so wait for it.
    const statusPath = `mocks/${reply.data.id}.json`;
    await waitFor(() => sent("sync:entityStatus").length > 0);

    const payload = sent("sync:entityStatus")[0];
    expect(payload.wsId).toBe(TEST_WS);
    // The freshly written mock must be reported as an unsaved change. If the status were
    // serialized before the (async) git query resolved, this would be an empty object.
    expect(payload.status[statusPath]).toBe("new");

    client.close();
  });
});
