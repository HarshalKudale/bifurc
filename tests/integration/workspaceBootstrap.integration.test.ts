import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import simpleGit from "simple-git";

// Only `startAutoSync` is stubbed — it would otherwise leave a real 30s poller timer running for
// the duration of the suite. Everything else runs for real: real temp data root, real directories,
// real `git init`, real settings persistence. Mocking the store layer here would test nothing,
// because the whole point of the extraction is that the *same* on-disk effects still happen.
vi.mock("@/sync/autoSync", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/sync/autoSync")>()),
  startAutoSync: vi.fn(),
}));

import { bootstrapWorkspaces } from "@/startup";
import { startAutoSync } from "@/sync/autoSync";
import { setDataDirOverride } from "@bifurc/engine/store/gitStore";
import { loadSettings, setSettingsPathOverride } from "@bifurc/engine/store/appSettings";
import type { AppSettings } from "@bifurc/engine/store/appSettings";

type WorkspaceMeta = AppSettings["workspaces"][number];

/** A workspace entry with sync enabled — `syncConfig` is read through a cast in `getSyncConfig`,
 * so it is not part of `WorkspaceMeta`'s declared shape. */
function autoSyncWorkspace(id: string, name: string): WorkspaceMeta {
  return {
    id,
    name,
    activeEnvironmentId: null,
    syncConfig: { remote: "", branch: "main", autoSync: true },
  } as unknown as WorkspaceMeta;
}

describe("startup/bootstrapWorkspaces (integration)", () => {
  let root: string;
  let settingsPath: string;

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-bootstrap-"));
    // Sets the data-root override *and* clears the per-workspace git cache — without the cache
    // clear, a workspace id reused across tests would keep a SimpleGit bound to a deleted dir.
    setDataDirOverride(root);
    settingsPath = path.join(root, "app.json");
    setSettingsPathOverride(settingsPath);
    vi.mocked(startAutoSync).mockClear();
  });

  afterEach(() => {
    setSettingsPathOverride(null);
    setDataDirOverride("");
    fs.rmSync(root, { recursive: true, force: true });
    vi.clearAllMocks();
  });

  /** Write a settings file to the override path and return the object that was written. */
  function seedSettings(over: Partial<AppSettings>): AppSettings {
    const seeded: AppSettings = { ...loadSettings(), ...over };
    fs.writeFileSync(settingsPath, JSON.stringify(seeded, null, 2), "utf-8");
    return seeded;
  }

  it("creates and initialises a default workspace when settings list none", async () => {
    // A *listed* workspace always has its directory created by the bootstrap loop before the
    // existence check runs, so this branch is only reachable with an empty list (fresh install) or
    // an active id that is not in the list at all. Pre-existing behaviour, preserved verbatim by
    // the extraction — see the quirk note in `src/startup.ts`.
    const seeded = seedSettings({
      workspaces: [],
      activeWorkspaceId: "gone-a",
    });

    const res = await bootstrapWorkspaces(seeded);

    expect(res.createdDefaultWorkspace).toBe(true);
    const id = res.settings.activeWorkspaceId;
    expect(id).not.toBe("gone-a");
    expect(res.settings.workspaces).toHaveLength(1);
    expect(res.settings.workspaces[0].id).toBe(id);
    expect(res.settings.workspaces[0].name).toBe("Workspace 1");

    // Real directories, real identity file, real git repo — not just a settings entry.
    expect(fs.statSync(path.join(root, id, "mappings")).isDirectory()).toBe(true);
    expect(fs.existsSync(path.join(root, id, "workspace.json"))).toBe(true);
    expect(fs.existsSync(path.join(root, id, ".git"))).toBe(true);

    // Persisted, so the next launch adopts the same workspace instead of making another.
    expect(loadSettings().activeWorkspaceId).toBe(id);
    expect(loadSettings().workspaces).toHaveLength(1);
  });

  it("repairs an active workspace id that is not in the workspace list", async () => {
    // An `activeWorkspaceId` absent from `workspaces` is the reachable form of "the active
    // workspace is gone" — see the quirk note in `src/startup.ts`.
    const seeded = seedSettings({
      workspaces: [
        { id: "ws-real", name: "Real", activeEnvironmentId: null },
        { id: "ws-other", name: "Other", activeEnvironmentId: null },
      ],
      activeWorkspaceId: "ghost-id",
    });

    const res = await bootstrapWorkspaces(seeded);

    expect(res.createdDefaultWorkspace).toBe(false);
    expect(res.settings.activeWorkspaceId).toBe("ws-real");
    // Every listed workspace is still known — repair must not prune the list.
    expect(res.settings.workspaces).toHaveLength(2);
    expect(loadSettings().activeWorkspaceId).toBe("ws-real");
    expect(fs.existsSync(path.join(root, "ws-real", ".git"))).toBe(true);
  });

  it("leaves settings untouched when the active workspace is healthy", async () => {
    fs.mkdirSync(path.join(root, "ws-ok"), { recursive: true });
    const seeded = seedSettings({
      workspaces: [{ id: "ws-ok", name: "Ok", activeEnvironmentId: null }],
      activeWorkspaceId: "ws-ok",
    });
    const before = fs.readFileSync(settingsPath, "utf-8");

    const res = await bootstrapWorkspaces(seeded);

    expect(res.createdDefaultWorkspace).toBe(false);
    expect(res.settings.activeWorkspaceId).toBe("ws-ok");
    // Byte-identical: no pointless rewrite on every launch.
    expect(fs.readFileSync(settingsPath, "utf-8")).toBe(before);
  });

  it("does not mutate the settings object it was given", async () => {
    fs.mkdirSync(path.join(root, "ws-x"), { recursive: true });
    const seeded = seedSettings({
      workspaces: [{ id: "ws-x", name: "X", activeEnvironmentId: null }],
      activeWorkspaceId: "missing",
    });

    await bootstrapWorkspaces(seeded);

    // The caller keeps its own reference; the repaired settings come back as the return value.
    expect(seeded.activeWorkspaceId).toBe("missing");
  });

  it("is safe to run twice — the second run initialises nothing new", async () => {
    fs.mkdirSync(path.join(root, "ws-twice"), { recursive: true });
    const seeded = seedSettings({
      workspaces: [{ id: "ws-twice", name: "Twice", activeEnvironmentId: null }],
      activeWorkspaceId: "ws-twice",
    });

    const first = await bootstrapWorkspaces(seeded);
    const second = await bootstrapWorkspaces(first.settings);

    expect(second.createdDefaultWorkspace).toBe(false);
    expect(second.settings.activeWorkspaceId).toBe("ws-twice");
    expect(second.settings.workspaces).toHaveLength(1);

    // `initWorkspaceRepo` commits only when it actually creates the repo, so a second bootstrap
    // must not add a second "chore: init workspace repo" commit.
    const log = await simpleGit(path.join(root, "ws-twice")).log();
    expect(log.total).toBe(1);
  });

  it("starts auto-sync only for workspaces configured for it", async () => {
    fs.mkdirSync(path.join(root, "ws-sync"), { recursive: true });
    fs.mkdirSync(path.join(root, "ws-quiet"), { recursive: true });
    const seeded = seedSettings({
      workspaces: [
        autoSyncWorkspace("ws-sync", "Sync"),
        { id: "ws-quiet", name: "Quiet", activeEnvironmentId: null },
      ],
      activeWorkspaceId: "ws-sync",
    });

    const res = await bootstrapWorkspaces(seeded);

    expect(res.autoSyncStarted).toEqual(["ws-sync"]);
    expect(vi.mocked(startAutoSync)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(startAutoSync)).toHaveBeenCalledWith("ws-sync");
  });

  it("initialises every known workspace, not only the active one", async () => {
    const seeded = seedSettings({
      workspaces: [
        { id: "ws-one", name: "One", activeEnvironmentId: null },
        { id: "ws-two", name: "Two", activeEnvironmentId: null },
      ],
      activeWorkspaceId: "ws-one",
    });

    await bootstrapWorkspaces(seeded);

    for (const id of ["ws-one", "ws-two"]) {
      expect(fs.existsSync(path.join(root, id, ".git"))).toBe(true);
      expect(fs.existsSync(path.join(root, id, "workspace.json"))).toBe(true);
    }
  });
});
