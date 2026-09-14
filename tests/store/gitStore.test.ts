import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import simpleGit from "simple-git";

// We bypass the electron mock for gitStore tests by using setDataDirOverride.
// electron is mocked globally in tests/setup.ts.

let tmpDir: string;
const WS_ID = "ws1";

beforeEach(async () => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lp-git-test-"));

  // Override data root so workspaceFs.wsDir() points at tmpDir
  const { setDataDirOverride } = await import("../../src/store/gitStore");
  setDataDirOverride(tmpDir);

  // Initialize the workspace directory and git repo
  const { initWorkspaceDir } = await import("../../src/store/workspaceFs");
  initWorkspaceDir(WS_ID, "Test Workspace");

  const { initWorkspaceRepo } = await import("../../src/store/gitStore");
  await initWorkspaceRepo(WS_ID);
});

afterEach(() => {
  fs.rmSync(tmpDir, { recursive: true, force: true });
  // Clear overrides and git cache
  import("../../src/store/gitStore").then(({ setDataDirOverride }) => {
    setDataDirOverride("");
  });
});

// ── helpers ───────────────────────────────────────────────────────────────────

async function writeMockFile(id: string, data: object, folderName?: string): Promise<string> {
  const { writeEntity, entityRelPath } = await import("../../src/store/workspaceFs");
  writeEntity(WS_ID, "mocks", id, data, folderName);
  return entityRelPath("mocks", id, folderName);
}

async function writeMappingFile(id: string, data: object): Promise<string> {
  const { writeFlatEntity, flatEntityRelPath } = await import("../../src/store/workspaceFs");
  writeFlatEntity(WS_ID, "mappings", id, data);
  return flatEntityRelPath("mappings", id);
}

async function freshCommit(relPath: string, opts: {
  action: import("../../src/store/types").AuditAction;
  entity: import("../../src/store/types").AuditEntity;
  entityId: string;
  entityName: string;
}): Promise<string> {
  const { commitMutation } = await import("../../src/store/gitStore");
  return commitMutation({ ...opts, workspaceId: WS_ID, relPath });
}

// ── commitMutation ────────────────────────────────────────────────────────────

describe("commitMutation()", () => {
  it("creates a git commit with the correct subject format", async () => {
    const { commitMutation } = await import("../../src/store/gitStore");
    const relPath = await writeMockFile("mock_abc", { id: "mock_abc", name: "POST /api/users" });
    const hash = await commitMutation({
      action: "create",
      entity: "mock",
      entityId: "mock_abc",
      entityName: "POST /api/users",
      workspaceId: WS_ID,
      relPath,
    });

    expect(hash).toBeTruthy();
    const g = simpleGit(path.join(tmpDir, WS_ID));
    const raw = await g.raw(["log", "-1", "--format=%s"]);
    expect(raw.trim()).toBe("create mock POST /api/users");
  });

  it("embeds entity-id, workspace-id, and actor in commit body", async () => {
    const { commitMutation } = await import("../../src/store/gitStore");
    const relPath = await writeMappingFile("map_xyz", { id: "map_xyz", domain: "api.localhost" });
    await commitMutation({
      action: "update",
      entity: "mapping",
      entityId: "map_xyz",
      entityName: "api.localhost",
      workspaceId: WS_ID,
      relPath,
      actor: "dev@example.com",
    });

    const g = simpleGit(path.join(tmpDir, WS_ID));
    const raw = await g.raw(["log", "-1", "--format=%b"]);
    expect(raw).toContain("entity-id: map_xyz");
    expect(raw).toContain(`workspace-id: ${WS_ID}`);
    expect(raw).toContain("actor: dev@example.com");
  });

  it("uses 'local' as the default actor", async () => {
    const { commitMutation } = await import("../../src/store/gitStore");
    const relPath = await writeMappingFile("env_1", { id: "env_1", name: "Production" });
    await commitMutation({
      action: "create",
      entity: "environment",
      entityId: "env_1",
      entityName: "Production",
      workspaceId: WS_ID,
      relPath,
    });

    const g = simpleGit(path.join(tmpDir, WS_ID));
    const raw = await g.raw(["log", "-1", "--format=%b"]);
    expect(raw).toContain(`actor: ${os.hostname()}`);
  });

  it("embeds changed-fields in the subject for update commits", async () => {
    const { commitMutation } = await import("../../src/store/gitStore");
    const relPath = await writeMockFile("m1", { id: "m1", name: "GET /api" });
    await commitMutation({
      action: "update",
      entity: "mock",
      entityId: "m1",
      entityName: "GET /api",
      workspaceId: WS_ID,
      relPath,
      changedFields: ["responseBody", "responseStatus"],
    });

    const g = simpleGit(path.join(tmpDir, WS_ID));
    const raw = await g.raw(["log", "-1", "--format=%s"]);
    expect(raw.trim()).toBe("update mock [responseBody,responseStatus] GET /api");
  });

  it("skips commit when file content is unchanged (spurious commit prevention)", async () => {
    const { commitMutation } = await import("../../src/store/gitStore");
    const data = { id: "m1", name: "GET /api" };
    const relPath = await writeMockFile("m1", data);
    const hash1 = await commitMutation({ action: "create", entity: "mock", entityId: "m1", entityName: "GET /api", workspaceId: WS_ID, relPath });

    // Write identical content again
    await writeMockFile("m1", data);
    const hash2 = await commitMutation({ action: "update", entity: "mock", entityId: "m1", entityName: "GET /api", workspaceId: WS_ID, relPath });

    expect(hash1).toBeTruthy();
    expect(hash2).toBe("");  // No-op commit returns empty string
  });
});

// ── queryLog ──────────────────────────────────────────────────────────────────

describe("queryLog()", () => {
  it("returns entries in descending timestamp order", async () => {
    const { queryLog } = await import("../../src/store/gitStore");

    const r1 = await writeMockFile("m1", { id: "m1", name: "Mock A" });
    await freshCommit(r1, { action: "create", entity: "mock", entityId: "m1", entityName: "Mock A" });

    const r2 = await writeMockFile("m2", { id: "m2", name: "Mock B" });
    await freshCommit(r2, { action: "create", entity: "mock", entityId: "m2", entityName: "Mock B" });

    // Delete m1
    const { deleteEntityFile } = await import("../../src/store/workspaceFs");
    deleteEntityFile(WS_ID, "mocks", "m1");
    await freshCommit(r1, { action: "delete", entity: "mock", entityId: "m1", entityName: "Mock A" });

    const { entries } = await queryLog({ workspaceId: WS_ID });
    // +1 for the init commit, but we filter by audit format so only 3 match
    expect(entries.length).toBeGreaterThanOrEqual(3);
    expect(entries[0].action).toBe("delete");
    expect(entries[1].action).toBe("create");
    expect(entries[2].action).toBe("create");
  });

  it("filters by entity type", async () => {
    const { queryLog } = await import("../../src/store/gitStore");

    const r1 = await writeMockFile("m1", { id: "m1", name: "My Mock" });
    await freshCommit(r1, { action: "create", entity: "mock", entityId: "m1", entityName: "My Mock" });

    const r2 = await writeMappingFile("map1", { id: "map1", domain: "api.localhost" });
    await freshCommit(r2, { action: "create", entity: "mapping", entityId: "map1", entityName: "api.localhost" });

    const r3 = await writeMockFile("m1", { id: "m1", name: "My Mock Updated" });
    await freshCommit(r3, { action: "update", entity: "mock", entityId: "m1", entityName: "My Mock" });

    const { entries, total } = await queryLog({ workspaceId: WS_ID, entity: "mock" });
    expect(total).toBe(2);
    expect(entries.every((e) => e.entity === "mock")).toBe(true);
  });

  it("filters by action", async () => {
    const { queryLog } = await import("../../src/store/gitStore");

    const r1 = await writeMockFile("m1", { id: "m1", name: "A", v: 1 });
    await freshCommit(r1, { action: "create", entity: "mock", entityId: "m1", entityName: "A" });

    const r2 = await writeMockFile("m1", { id: "m1", name: "A", v: 2 });
    await freshCommit(r2, { action: "update", entity: "mock", entityId: "m1", entityName: "A" });

    const { deleteEntityFile } = await import("../../src/store/workspaceFs");
    deleteEntityFile(WS_ID, "mocks", "m1");
    await freshCommit(r1, { action: "delete", entity: "mock", entityId: "m1", entityName: "A" });

    const { entries } = await queryLog({ workspaceId: WS_ID, action: "update" });
    expect(entries.length).toBe(1);
    expect(entries[0].action).toBe("update");
  });

  it("filters by entityId", async () => {
    const { queryLog } = await import("../../src/store/gitStore");

    const r1 = await writeMockFile("m1", { id: "m1", name: "Mock 1" });
    await freshCommit(r1, { action: "create", entity: "mock", entityId: "m1", entityName: "Mock 1" });

    const r2 = await writeMockFile("m2", { id: "m2", name: "Mock 2" });
    await freshCommit(r2, { action: "create", entity: "mock", entityId: "m2", entityName: "Mock 2" });

    const r3 = await writeMockFile("m1", { id: "m1", name: "Mock 1 Updated" });
    await freshCommit(r3, { action: "update", entity: "mock", entityId: "m1", entityName: "Mock 1" });

    const { entries } = await queryLog({ workspaceId: WS_ID, entityId: "m1" });
    expect(entries.length).toBe(2);
    expect(entries.every((e) => e.entityId === "m1")).toBe(true);
  });

  it("filters by search (case-insensitive entity name match)", async () => {
    const { queryLog } = await import("../../src/store/gitStore");

    const r1 = await writeMockFile("m1", { id: "m1", name: "foo endpoint" });
    await freshCommit(r1, { action: "create", entity: "mock", entityId: "m1", entityName: "foo endpoint" });

    const r2 = await writeMockFile("m2", { id: "m2", name: "bar endpoint" });
    await freshCommit(r2, { action: "create", entity: "mock", entityId: "m2", entityName: "bar endpoint" });

    const r3 = await writeMockFile("m3", { id: "m3", name: "FOO special" });
    await freshCommit(r3, { action: "create", entity: "mock", entityId: "m3", entityName: "FOO special" });

    const { entries } = await queryLog({ workspaceId: WS_ID, search: "foo" });
    expect(entries.length).toBe(2);
    expect(entries.every((e) => e.entityName.toLowerCase().includes("foo"))).toBe(true);
  });

  it("paginates with limit and offset", async () => {
    const { queryLog } = await import("../../src/store/gitStore");

    for (let i = 0; i < 5; i++) {
      const r = await writeMockFile(`m${i}`, { id: `m${i}`, name: `Mock ${i}`, v: i });
      await freshCommit(r, { action: "create", entity: "mock", entityId: `m${i}`, entityName: `Mock ${i}` });
    }

    const page1 = await queryLog({ workspaceId: WS_ID, limit: 2, offset: 0 });
    const page2 = await queryLog({ workspaceId: WS_ID, limit: 2, offset: 2 });

    expect(page1.entries.length).toBe(2);
    expect(page2.entries.length).toBe(2);
    expect(page1.total).toBe(5);
    expect(page1.entries[0].entityId).not.toBe(page2.entries[0].entityId);
  });

  it("returns all entries when limit is 0", async () => {
    const { queryLog } = await import("../../src/store/gitStore");

    for (let i = 0; i < 5; i++) {
      const r = await writeMappingFile(`r${i}`, { id: `r${i}`, name: `Rule ${i}`, v: i });
      await freshCommit(r, { action: "create", entity: "rule", entityId: `r${i}`, entityName: `Rule ${i}` });
    }

    const { entries } = await queryLog({ workspaceId: WS_ID, limit: 0 });
    expect(entries.length).toBe(5);
  });

  it("returns empty array when the workspace has no audit commits", async () => {
    const { queryLog } = await import("../../src/store/gitStore");
    const { entries, total } = await queryLog({ workspaceId: WS_ID });
    // init commit doesn't match audit format → 0 entries
    expect(entries).toEqual([]);
    expect(total).toBe(0);
  });

  it("returns empty array for a non-existent workspace", async () => {
    const { queryLog } = await import("../../src/store/gitStore");
    const { entries, total } = await queryLog({ workspaceId: "nonexistent" });
    expect(entries).toEqual([]);
    expect(total).toBe(0);
  });

  it("filters by specific filePath", async () => {
    const { queryLog } = await import("../../src/store/gitStore");

    const r1 = await writeMockFile("m1", { id: "m1", name: "Mock A", v: 1 });
    await freshCommit(r1, { action: "create", entity: "mock", entityId: "m1", entityName: "Mock A" });

    const r2 = await writeMockFile("m2", { id: "m2", name: "Mock B" });
    await freshCommit(r2, { action: "create", entity: "mock", entityId: "m2", entityName: "Mock B" });

    const r3 = await writeMockFile("m1", { id: "m1", name: "Mock A updated", v: 2 });
    await freshCommit(r3, { action: "update", entity: "mock", entityId: "m1", entityName: "Mock A" });

    // Filter to only commits that touched m1's file
    const { entries } = await queryLog({ workspaceId: WS_ID, filePath: "mocks/m1.json" });
    expect(entries.length).toBe(2);
    expect(entries.every((e) => e.entityId === "m1")).toBe(true);
  });
});

// ── getEntityAtCommit ─────────────────────────────────────────────────────────

describe("getEntityAtCommit()", () => {
  it("returns the entity state as it was at the given commit", async () => {
    const { commitMutation, getEntityAtCommit } = await import("../../src/store/gitStore");

    const mockV1 = { id: "m1", name: "Version 1", method: "GET", responseStatus: 200 };
    const mockV2 = { id: "m1", name: "Version 2", method: "POST", responseStatus: 201 };

    const r1 = await writeMockFile("m1", mockV1);
    const hash1 = await commitMutation({ action: "create", entity: "mock", entityId: "m1", entityName: "Version 1", workspaceId: WS_ID, relPath: r1 });

    const r2 = await writeMockFile("m1", mockV2);
    await commitMutation({ action: "update", entity: "mock", entityId: "m1", entityName: "Version 2", workspaceId: WS_ID, relPath: r2 });

    const atV1 = await getEntityAtCommit(hash1, WS_ID, "mocks/m1.json");
    expect((atV1 as any)?.name).toBe("Version 1");
    expect((atV1 as any)?.method).toBe("GET");
  });

  it("returns null when the entity does not exist at that commit", async () => {
    const { commitMutation, getEntityAtCommit } = await import("../../src/store/gitStore");

    const r1 = await writeMockFile("m1", { id: "m1", name: "A" });
    const hash = await commitMutation({ action: "create", entity: "mock", entityId: "m1", entityName: "A", workspaceId: WS_ID, relPath: r1 });

    // m999 never existed
    const result = await getEntityAtCommit(hash, WS_ID, "mocks/m999.json");
    expect(result).toBeNull();
  });

  it("returns null for a bad commit reference", async () => {
    const { getEntityAtCommit } = await import("../../src/store/gitStore");
    const result = await getEntityAtCommit("0000000000000000000000000000000000000000", WS_ID, "mocks/m1.json");
    expect(result).toBeNull();
  });

  it("returns null for a non-existent workspace id", async () => {
    const { getEntityAtCommit } = await import("../../src/store/gitStore");
    const result = await getEntityAtCommit("abc1234", "nonexistent-ws", "mocks/m1.json");
    expect(result).toBeNull();
  });

  it("reconstructs before/after for an update by reading parent commit", async () => {
    const { commitMutation, getEntityAtCommit } = await import("../../src/store/gitStore");

    const v1 = { id: "env1", name: "Dev", variables: [] };
    const v2 = { id: "env1", name: "Development", variables: [{ id: "v1", key: "URL", value: "http://localhost" }] };

    const { writeFlatEntity } = await import("../../src/store/workspaceFs");
    writeFlatEntity(WS_ID, "environments", "env1", v1);
    const r1 = "environments/env1.json";
    await commitMutation({ action: "create", entity: "environment", entityId: "env1", entityName: "Dev", workspaceId: WS_ID, relPath: r1 });

    writeFlatEntity(WS_ID, "environments", "env1", v2);
    const hash2 = await commitMutation({ action: "update", entity: "environment", entityId: "env1", entityName: "Development", workspaceId: WS_ID, relPath: r1 });

    const after  = await getEntityAtCommit(hash2,        WS_ID, r1);
    const before = await getEntityAtCommit(`${hash2}~1`, WS_ID, r1);

    expect((after  as any)?.name).toBe("Development");
    expect((before as any)?.name).toBe("Dev");
  });
});

// ── getCommitChangedFiles ─────────────────────────────────────────────────────

describe("getCommitChangedFiles()", () => {
  it("returns the relative file paths changed in a commit", async () => {
    const { commitMutation, getCommitChangedFiles } = await import("../../src/store/gitStore");

    const relPath = await writeMockFile("m1", { id: "m1", name: "A" });
    const hash = await commitMutation({ action: "create", entity: "mock", entityId: "m1", entityName: "A", workspaceId: WS_ID, relPath });

    const files = await getCommitChangedFiles(hash, WS_ID);

    expect(files).toContain("mocks/m1.json");
  });

  it("returns empty array for a non-existent commit reference", async () => {
    const { getCommitChangedFiles } = await import("../../src/store/gitStore");

    const files = await getCommitChangedFiles("0000000000000000000000000000000000000000", WS_ID);

    expect(files).toEqual([]);
  });

  it("returns empty array for a non-existent workspace", async () => {
    const { getCommitChangedFiles } = await import("../../src/store/gitStore");

    const files = await getCommitChangedFiles("HEAD", "nonexistent-ws");

    expect(files).toEqual([]);
  });

  it("lists multiple files when a commit touches more than one file", async () => {
    const { commitMutation, getCommitChangedFiles } = await import("../../src/store/gitStore");

    const r1 = await writeMockFile("m1", { id: "m1", name: "A" });
    const r2 = await writeMockFile("m2", { id: "m2", name: "B" });

    // Commit both files in a single commitMutation call by committing r2 after r1 is staged
    // Since commitMutation handles one file at a time, we manually stage both via simple-git
    const simpleGitModule = await import("simple-git");
    const path = await import("path");
    const g = simpleGitModule.default(path.join(tmpDir, WS_ID));
    await g.add([r1, r2]);
    const result = await g.commit("create(mock): batch", { "--no-gpg-sign": null } as any);
    const hash = result.commit;

    const files = await getCommitChangedFiles(hash, WS_ID);

    expect(files).toContain("mocks/m1.json");
    expect(files).toContain("mocks/m2.json");
    expect(files.length).toBe(2);
  });
});
