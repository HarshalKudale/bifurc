import { describe, it, expect, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import simpleGit from "simple-git";
import {
  getFileDiff,
  discardChanges,
  syncChanges,
  getFileHistory,
} from "../../src/sync/gitOps";
import {
  writeEntity,
  readEntity,
  upsertNameEntry,
  readNamesIndex,
  wsDir,
} from "../../src/store/workspaceFs";

let tmpDir: string;
const WS_ID = "ws-gitops-test";

async function setup() {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lp-gitops-test-"));

  const { setDataDirOverride } = await import("../../src/store/gitStore");
  setDataDirOverride(tmpDir);

  const { setSettingsPathOverride } = await import("../../src/store/appSettings");
  setSettingsPathOverride(path.join(tmpDir, "app.json"));

  fs.writeFileSync(
    path.join(tmpDir, "app.json"),
    JSON.stringify({
      port: 80,
      minimizeToTray: false,
      workspaces: [{ id: WS_ID, name: "Test Workspace", activeEnvironmentId: null }],
      activeWorkspaceId: WS_ID,
    }),
    "utf-8",
  );

  const { initWorkspaceDir } = await import("../../src/store/workspaceFs");
  initWorkspaceDir(WS_ID, "Test Workspace");

  const { initWorkspaceRepo } = await import("../../src/store/gitStore");
  await initWorkspaceRepo(WS_ID);
}

async function teardown() {
  const { setDataDirOverride } = await import("../../src/store/gitStore");
  setDataDirOverride("");

  const { setSettingsPathOverride } = await import("../../src/store/appSettings");
  setSettingsPathOverride(null);

  fs.rmSync(tmpDir, { recursive: true, force: true });
}

describe("Git Operations (gitOps)", () => {
  beforeEach(async () => {
    await setup();
  });

  afterEach(async () => {
    await teardown();
  });

  it("detects clean, new, modified, and deleted diffs", async () => {
    const req = {
      id: "req-1",
      name: "Get Users",
      method: "GET",
      url: "https://api.example.com/users",
      headers: {},
      body: "",
      createdAt: 1000,
    };
    const relPath = "requests/req-1.json";

    // Untracked new file
    writeEntity(WS_ID, "requests", req.id, req);
    upsertNameEntry(WS_ID, "requests", req.id, { name: req.name, method: req.method, url: req.url });

    const newDiff = await getFileDiff(WS_ID, relPath);
    expect(newDiff.hasDiff).toBe(true);
    expect(newDiff.status).toBe("new");

    // Commit via syncChanges
    const syncRes = await syncChanges(WS_ID, [relPath], "Add Get Users request");
    expect(syncRes.ok).toBe(true);

    const cleanDiff = await getFileDiff(WS_ID, relPath);
    expect(cleanDiff.hasDiff).toBe(false);
    expect(cleanDiff.status).toBe("clean");

    // Modify file
    const modifiedReq = { ...req, name: "Get Users Modified", url: "https://api.example.com/v2/users" };
    writeEntity(WS_ID, "requests", req.id, modifiedReq);
    upsertNameEntry(WS_ID, "requests", req.id, {
      name: modifiedReq.name,
      method: modifiedReq.method,
      url: modifiedReq.url,
    });

    const modDiff = await getFileDiff(WS_ID, relPath);
    expect(modDiff.hasDiff).toBe(true);
    expect(modDiff.status).toBe("modified");
    expect(modDiff.diff).toContain("Get Users Modified");
  });

  it("discards uncommitted modifications and restores original content and names.json", async () => {
    const req = {
      id: "req-revert-1",
      name: "Original Request",
      method: "POST",
      url: "https://api.example.com/submit",
      headers: { "Content-Type": "application/json" },
      body: '{"foo":"bar"}',
      createdAt: 1000,
    };
    const relPath = "requests/req-revert-1.json";

    // 1. Create and commit original
    writeEntity(WS_ID, "requests", req.id, req);
    upsertNameEntry(WS_ID, "requests", req.id, { name: req.name, method: req.method, url: req.url });
    await syncChanges(WS_ID, [relPath]);

    // 2. Make uncommitted changes
    const modifiedReq = {
      ...req,
      name: "Edited Uncommitted Request",
      url: "https://api.example.com/changed",
      body: '{"modified":true}',
    };
    writeEntity(WS_ID, "requests", req.id, modifiedReq);
    upsertNameEntry(WS_ID, "requests", req.id, {
      name: modifiedReq.name,
      method: modifiedReq.method,
      url: modifiedReq.url,
    });

    // Check that disk and names.json reflect edits
    expect(readEntity<any>(WS_ID, "requests", req.id)?.name).toBe("Edited Uncommitted Request");
    expect(readNamesIndex(WS_ID, "requests")[req.id]?.name).toBe("Edited Uncommitted Request");

    // 3. Discard changes
    const discardRes = await discardChanges(WS_ID, relPath);
    expect(discardRes.ok).toBe(true);

    // 4. Verify restored state
    const restored = readEntity<any>(WS_ID, "requests", req.id);
    expect(restored).not.toBeNull();
    expect(restored.name).toBe("Original Request");
    expect(restored.url).toBe("https://api.example.com/submit");
    expect(restored.body).toBe('{"foo":"bar"}');

    // names.json should also be reverted back to original
    const restoredNames = readNamesIndex(WS_ID, "requests");
    expect(restoredNames[req.id]?.name).toBe("Original Request");
    expect(restoredNames[req.id]?.url).toBe("https://api.example.com/submit");

    // git diff should now be clean
    const status = await getFileDiff(WS_ID, relPath);
    expect(status.hasDiff).toBe(false);
    expect(status.status).toBe("clean");
  });

  it("discards untracked entity by deleting file and removing from names.json", async () => {
    const mock = {
      id: "mock-untracked-1",
      name: "New Untracked Mock",
      method: "GET",
      urlPattern: "/api/untracked",
      responseStatus: 200,
      createdAt: 2000,
    };
    const relPath = "mocks/mock-untracked-1.json";

    writeEntity(WS_ID, "mocks", mock.id, mock);
    upsertNameEntry(WS_ID, "mocks", mock.id, {
      name: mock.name,
      method: mock.method,
      url: mock.urlPattern,
    });

    expect(fs.existsSync(path.join(wsDir(WS_ID), relPath))).toBe(true);
    expect(readNamesIndex(WS_ID, "mocks")[mock.id]).toBeDefined();

    const discardRes = await discardChanges(WS_ID, relPath);
    expect(discardRes.ok).toBe(true);

    // File should be deleted
    expect(fs.existsSync(path.join(wsDir(WS_ID), relPath))).toBe(false);
    // Entry in names.json should be removed
    expect(readNamesIndex(WS_ID, "mocks")[mock.id]).toBeUndefined();
  });

  it("retrieves commit history for a file", async () => {
    const req = {
      id: "req-hist",
      name: "V1",
      method: "GET",
      url: "/v1",
      createdAt: 1000,
    };
    const relPath = "requests/req-hist.json";

    writeEntity(WS_ID, "requests", req.id, req);
    await syncChanges(WS_ID, [relPath]);

    writeEntity(WS_ID, "requests", req.id, { ...req, name: "V2" });
    await syncChanges(WS_ID, [relPath]);

    const history = await getFileHistory(WS_ID, relPath);
    expect(history.total).toBe(2);
    expect(history.entries[0].action).toBeDefined();
  });
});
