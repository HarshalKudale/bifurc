import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import simpleGit from "simple-git";

// ── Helpers ───────────────────────────────────────────────────────────────────

let tmpDir: string;
let remoteDir: string;
const WS_ID = "ws-sync-test";

async function setup() {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lp-sync-test-"));
  remoteDir = path.join(tmpDir, "remote.git");

  const { setDataDirOverride } = await import("../../src/store/gitStore");
  setDataDirOverride(tmpDir);

  const { setSettingsPathOverride } = await import("../../src/store/appSettings");
  setSettingsPathOverride(path.join(tmpDir, "app.json"));

  // Write minimal settings
  fs.writeFileSync(
    path.join(tmpDir, "app.json"),
    JSON.stringify({
      port: 80,
      minimizeToTray: false,
      workspaces: [{ id: WS_ID, name: "Test", activeEnvironmentId: null }],
      activeWorkspaceId: WS_ID,
    }),
    "utf-8",
  );

  // Initialize the workspace directory and local git repo
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

async function makeBareRemote(): Promise<string> {
  fs.mkdirSync(remoteDir, { recursive: true });
  await simpleGit(remoteDir).init(true);
  return remoteDir;
}

async function makeNonEmptyRemote(): Promise<string> {
  const cloneDir = path.join(tmpDir, "remote-seeded");
  fs.mkdirSync(cloneDir, { recursive: true });
  const g = simpleGit(cloneDir);
  await g.init();
  await g.addConfig("user.email", "test@test.com", false, "local");
  await g.addConfig("user.name", "Tester", false, "local");
  fs.writeFileSync(path.join(cloneDir, "workspace.json"), JSON.stringify({ id: "other", name: "Other" }), "utf-8");
  await g.add(".");
  await g.commit("init");

  // Create bare remote and push to it
  const bareDir = path.join(tmpDir, "remote-non-empty.git");
  fs.mkdirSync(bareDir, { recursive: true });
  await simpleGit(bareDir).init(true);
  await g.addRemote("origin", bareDir);
  await g.push("origin", "HEAD:main", ["--set-upstream"]);
  return bareDir;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("syncManager", () => {
  beforeEach(async () => { await setup(); }, 30_000);
  afterEach(async () => { await teardown(); }, 30_000);

  it("setRemote: connects empty workspace to empty remote", async () => {
    const bare = await makeBareRemote();
    const { setRemote } = await import("../../src/sync/syncManager");
    const result = await setRemote(WS_ID, bare, "main");
    expect(result.ok).toBe(true);
  }, 30_000);

  it("setRemote: saves syncConfig to appSettings", async () => {
    const bare = await makeBareRemote();
    const { setRemote, getSyncConfig } = await import("../../src/sync/syncManager");
    await setRemote(WS_ID, bare, "main");
    const cfg = getSyncConfig(WS_ID);
    expect(cfg?.remote).toBe(bare);
    expect(cfg?.branch).toBe("main");
    expect(cfg?.autoSync).toBe(false);
  }, 30_000);

  it("setRemote: clones non-empty remote into empty workspace", async () => {
    const nonEmptyRemote = await makeNonEmptyRemote();
    const { setRemote, getSyncConfig } = await import("../../src/sync/syncManager");

    const result = await setRemote(WS_ID, nonEmptyRemote, "main");
    expect(result.ok).toBe(true);
    expect(result.cloned).toBe(true);

    // After clone the workspace adopts the remote's id ("other")
    const adoptedId = result.adoptedId ?? WS_ID;
    const cfg = getSyncConfig(adoptedId);
    expect(cfg?.remote).toBe(nonEmptyRemote);

    // workspace.json from the remote should now be present under the adopted dir
    const { wsDir } = await import("../../src/store/workspaceFs");
    const wsJsonPath = path.join(wsDir(adoptedId), "workspace.json");
    expect(fs.existsSync(wsJsonPath)).toBe(true);
  }, 30_000);

  it("setRemote: clone adopts remote workspace id+name in app.json", async () => {
    const nonEmptyRemote = await makeNonEmptyRemote(); // remote has id="other", name="Other"
    const { setRemote } = await import("../../src/sync/syncManager");

    const result = await setRemote(WS_ID, nonEmptyRemote, "main");
    expect(result.ok).toBe(true);
    expect(result.adoptedId).toBe("other");

    // app.json should now use "other" as the workspace id and active id
    const { loadSettings } = await import("../../src/store/appSettings");
    const settings = loadSettings();
    expect(settings.activeWorkspaceId).toBe("other");
    const ws = settings.workspaces.find((w) => w.id === "other");
    expect(ws).toBeDefined();
    expect(ws?.name).toBe("Other");
    // old local id should no longer exist
    expect(settings.workspaces.find((w) => w.id === WS_ID)).toBeUndefined();
  }, 30_000);

  it("setRemote: empty workspace with only enabled.json/index.json is treated as empty", async () => {
    const nonEmptyRemote = await makeNonEmptyRemote();

    // Bootstrap enabled.json in each kind dir — this was the bug: these were counted as entity files
    const { wsDir, writeEnabledSet } = await import("../../src/store/workspaceFs");
    const dir = wsDir(WS_ID);
    for (const kind of ["mappings", "rules", "mocks", "requests", "sockets", "environments"]) {
      fs.mkdirSync(path.join(dir, kind), { recursive: true });
      writeEnabledSet(WS_ID, kind, new Set());
      // also write an empty index.json
      fs.writeFileSync(path.join(dir, kind, "index.json"), JSON.stringify({ folders: [], order: [] }), "utf-8");
    }

    const { setRemote } = await import("../../src/sync/syncManager");
    const result = await setRemote(WS_ID, nonEmptyRemote, "main");

    // Should clone (not fail with "Remote is not empty")
    expect(result.ok).toBe(true);
    expect(result.cloned).toBe(true);
  }, 30_000);

  it("setRemote: returns error when connecting non-empty workspace to non-empty remote", async () => {
    const nonEmptyRemote = await makeNonEmptyRemote();

    // Add a file to our workspace so it is non-empty
    const { wsDir } = await import("../../src/store/workspaceFs");
    const dir = wsDir(WS_ID);
    fs.mkdirSync(path.join(dir, "mappings"), { recursive: true });
    fs.writeFileSync(path.join(dir, "mappings", "m1.json"), JSON.stringify({ id: "m1", domain: "test.localhost", target: "127.0.0.1:3000", enabled: true, workspaceId: WS_ID }), "utf-8");
    const { getGit } = await import("../../src/store/gitStore");
    await getGit(WS_ID).add(".");
    await getGit(WS_ID).commit("add mapping");

    const { setRemote } = await import("../../src/sync/syncManager");
    const result = await setRemote(WS_ID, nonEmptyRemote, "main");
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/not empty/i);
  }, 30_000);

  it("disconnect: removes syncConfig and remote", async () => {
    const bare = await makeBareRemote();
    const { setRemote, disconnect, getSyncConfig } = await import("../../src/sync/syncManager");
    await setRemote(WS_ID, bare, "main");
    await disconnect(WS_ID);
    const cfg = getSyncConfig(WS_ID);
    expect(cfg).toBeNull();
  }, 30_000);

  it("syncPush: returns error when no remote configured", async () => {
    const { syncPush } = await import("../../src/sync/syncManager");
    const result = await syncPush(WS_ID);
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/no remote/i);
  });

  it("syncPull: returns error when no remote configured", async () => {
    const { syncPull } = await import("../../src/sync/syncManager");
    const result = await syncPull(WS_ID);
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/no remote/i);
  });

  it("syncPush: succeeds after connecting to empty remote", async () => {
    const bare = await makeBareRemote();
    const { setRemote, syncPush } = await import("../../src/sync/syncManager");
    await setRemote(WS_ID, bare, "main");
    const result = await syncPush(WS_ID);
    expect(result.ok).toBe(true);
  }, 30_000);

  it("syncPull: reports updated=false when already up to date", async () => {
    const bare = await makeBareRemote();
    const { setRemote, syncPull, syncPush } = await import("../../src/sync/syncManager");
    await setRemote(WS_ID, bare, "main");
    await syncPush(WS_ID);
    const result = await syncPull(WS_ID);
    expect(result.ok).toBe(true);
    expect(result.updated).toBe(false);
  }, 30_000);

  it("setAutoSync: updates autoSync flag in syncConfig", async () => {
    const bare = await makeBareRemote();
    const { setRemote, setAutoSync, getSyncConfig } = await import("../../src/sync/syncManager");
    await setRemote(WS_ID, bare, "main");
    await setAutoSync(WS_ID, true);
    const cfg = getSyncConfig(WS_ID);
    expect(cfg?.autoSync).toBe(true);
  }, 30_000);

  it("getSyncState: returns idle state initially", async () => {
    const { getSyncState } = await import("../../src/sync/syncManager");
    const state = getSyncState(WS_ID);
    expect(state.status).toBe("idle");
    expect(state.error).toBeNull();
  });
});

// ── randomNames ───────────────────────────────────────────────────────────────

describe("generateRandomWorkspaceName()", () => {
  it("returns adjective-noun format", async () => {
    const { generateRandomWorkspaceName } = await import("../../src/lib/randomNames");
    const name = generateRandomWorkspaceName();
    expect(name).toMatch(/^[a-z]+-[a-z]+$/);
  });

  it("returns different names on repeated calls (probabilistic)", async () => {
    const { generateRandomWorkspaceName } = await import("../../src/lib/randomNames");
    const names = new Set(Array.from({ length: 10 }, () => generateRandomWorkspaceName()));
    expect(names.size).toBeGreaterThan(1);
  });
});
