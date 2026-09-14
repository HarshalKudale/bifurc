/**
 * Happy-path workflow test for the **Audit Log** feature.
 *
 * The Audit Log panel (`audit:list` / `audit:diff` / `audit:export`) was another
 * completely untested screen: it renders git history for a workspace, and the whole
 * pipeline — commit a mutation, read the log back, diff a commit, resolve the
 * changed files — had no coverage. A regression in the commit *subject format* (the
 * thing the panel actually parses into rows) would have been invisible.
 *
 * This drives real git commits in a real temp workspace.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import * as fs from "fs";
import * as path from "path";
import { createWorkspace, TEST_WS, type WorkspaceFixture } from "./proxyHarness";
import { initWorkspaceDir } from "@/store/workspaceFs";
import {
    initWorkspaceRepo,
    commitMutation,
    queryLog,
    getCommitChangedFiles,
    getEntityAtCommit,
    setDataDirOverride,
} from "@/store/gitStore";

let ws: WorkspaceFixture;
let createCommit: string;
let updateCommit: string;

const MOCK_ID = "mock-audit";
const REL_PATH = `mocks/${MOCK_ID}.json`;

beforeAll(async () => {
    ws = createWorkspace();
    // Clear gitStore's per-workspace git cache so it points at this temp dir.
    setDataDirOverride(ws.dataRoot);

    // The real app initialises the workspace directory (which also writes
    // .gitignore) before creating the repo — mirror that so the repo layout matches.
    initWorkspaceDir(TEST_WS, "Audit Workspace");
    await initWorkspaceRepo(TEST_WS);

    // ── A "create" mutation ───────────────────────────────────────────────────
    const mock = {
        id: MOCK_ID, name: "Audit Mock", method: "GET", urlPattern: "/audit",
        useRegex: false, capturedHeaders: {}, capturedBody: "",
        responseStatus: 200, responseHeaders: {}, responseBody: "{}",
        responseDelay: 0, createdAt: 1, folderId: null, workspaceId: TEST_WS,
    };
    fs.writeFileSync(path.join(ws.dataRoot, TEST_WS, REL_PATH), JSON.stringify(mock, null, 2), "utf-8");
    createCommit = await commitMutation({
        action: "create", entity: "mock", entityId: MOCK_ID, entityName: "Audit Mock",
        workspaceId: TEST_WS, relPath: REL_PATH, actor: "tester@bifurc",
    });

    // ── An "update" mutation ──────────────────────────────────────────────────
    fs.writeFileSync(
        path.join(ws.dataRoot, TEST_WS, REL_PATH),
        JSON.stringify({ ...mock, name: "Audit Mock v2", responseStatus: 201 }, null, 2),
        "utf-8",
    );
    updateCommit = await commitMutation({
        action: "update", entity: "mock", entityId: MOCK_ID, entityName: "Audit Mock v2",
        workspaceId: TEST_WS, relPath: REL_PATH, actor: "tester@bifurc",
        changedFields: ["name", "responseStatus"],
    });
});

afterAll(() => {
    setDataDirOverride("");
    ws?.cleanup();
});

describe("Audit Log — happy path", () => {
    it("commits a mutation and returns a commit hash", () => {
        expect(createCommit).toMatch(/^[0-9a-f]{7,40}$/);
        expect(updateCommit).toMatch(/^[0-9a-f]{7,40}$/);
        expect(updateCommit).not.toBe(createCommit);
    });

    it("lists the audit entries for the workspace, newest first", async () => {
        const { entries, total } = await queryLog({ workspaceId: TEST_WS });

        expect(total).toBe(2);
        expect(entries).toHaveLength(2);

        const [newest, oldest] = entries;
        expect(newest.commitHash).toBe(updateCommit);
        expect(newest.action).toBe("update");
        expect(newest.entity).toBe("mock");
        expect(newest.entityName).toBe("Audit Mock v2");
        expect(newest.entityId).toBe(MOCK_ID);
        expect(newest.workspaceId).toBe(TEST_WS);
        expect(newest.actor).toBe("tester@bifurc");
        expect(newest.changedFields).toEqual(["name", "responseStatus"]);
        expect(typeof newest.ts).toBe("number");
        expect(newest.ts).toBeGreaterThan(0);

        expect(oldest.commitHash).toBe(createCommit);
        expect(oldest.action).toBe("create");
        expect(oldest.entityName).toBe("Audit Mock");
        expect(oldest.changedFields).toBeUndefined();
    });

    it("filters by entity kind", async () => {
        const { entries, total } = await queryLog({ workspaceId: TEST_WS, entity: "mock" });
        expect(total).toBe(2);
        expect(entries.every((e) => e.entity === "mock")).toBe(true);

        const none = await queryLog({ workspaceId: TEST_WS, entity: "webhook" });
        expect(none.total).toBe(0);
        expect(none.entries).toEqual([]);
    });

    it("filters by action", async () => {
        const created = await queryLog({ workspaceId: TEST_WS, action: "create" });
        expect(created.total).toBe(1);
        expect(created.entries[0].action).toBe("create");

        const updated = await queryLog({ workspaceId: TEST_WS, action: "update" });
        expect(updated.total).toBe(1);
        expect(updated.entries[0].action).toBe("update");
    });

    it("filters by entity id", async () => {
        const mine = await queryLog({ workspaceId: TEST_WS, entityId: MOCK_ID });
        expect(mine.total).toBe(2);

        const other = await queryLog({ workspaceId: TEST_WS, entityId: "nope" });
        expect(other.total).toBe(0);
    });

    it("searches by entity name", async () => {
        const { entries, total } = await queryLog({ workspaceId: TEST_WS, search: "Audit Mock" });
        expect(total).toBe(2);
        expect(entries.every((e) => e.entityName.includes("Audit Mock"))).toBe(true);

        const v2 = await queryLog({ workspaceId: TEST_WS, search: "v2" });
        expect(v2.total).toBe(1);
        expect(v2.entries[0].entityName).toBe("Audit Mock v2");
    });

    it("paginates with limit and offset", async () => {
        const page1 = await queryLog({ workspaceId: TEST_WS, limit: 1 });
        expect(page1.entries).toHaveLength(1);
        expect(page1.total).toBe(2); // total is the unpaged count
        expect(page1.entries[0].commitHash).toBe(updateCommit);

        const page2 = await queryLog({ workspaceId: TEST_WS, limit: 1, offset: 1 });
        expect(page2.entries).toHaveLength(1);
        expect(page2.entries[0].commitHash).toBe(createCommit);

        const beyond = await queryLog({ workspaceId: TEST_WS, limit: 10, offset: 10 });
        expect(beyond.entries).toEqual([]);
    });

    it("filters by entity file path", async () => {
        const { total } = await queryLog({ workspaceId: TEST_WS, filePath: REL_PATH });
        expect(total).toBe(2);

        const other = await queryLog({ workspaceId: TEST_WS, filePath: "mocks/some-other.json" });
        expect(other.total).toBe(0);
    });

    it("resolves the files changed by a commit", async () => {
        const changed = await getCommitChangedFiles(createCommit, TEST_WS);
        expect(changed.some((f) => f.includes(MOCK_ID))).toBe(true);

        // The oldest audit entry must also resolve its files — this is what the
        // `--root` flag on diff-tree guarantees (without it, a workspace whose first
        // commit is an entity commit would render an empty diff view).
        const oldest = (await queryLog({ workspaceId: TEST_WS })).entries.at(-1)!;
        expect(await getCommitChangedFiles(oldest.commitHash, TEST_WS)).not.toHaveLength(0);
    });

    it("reads an entity's content at a commit and its parent", async () => {
        // getEntityAtCommit returns the PARSED entity (or null), not raw JSON text.
        const after = (await getEntityAtCommit(updateCommit, TEST_WS, REL_PATH)) as any;
        expect(after).toBeTruthy();
        expect(after.name).toBe("Audit Mock v2");
        expect(after.responseStatus).toBe(201);

        const before = (await getEntityAtCommit(`${updateCommit}~1`, TEST_WS, REL_PATH)) as any;
        expect(before).toBeTruthy();
        expect(before.name).toBe("Audit Mock");
        expect(before.responseStatus).toBe(200);
    });

    it("returns null for a path that does not exist at that commit", async () => {
        expect(await getEntityAtCommit(createCommit, TEST_WS, "mocks/not-there.json")).toBeNull();
    });

    it("returns an empty log for a workspace that is not a git repo", async () => {
        const { entries, total } = await queryLog({ workspaceId: "ws-not-a-repo" });
        expect(entries).toEqual([]);
        expect(total).toBe(0);
    });

    it("skips the commit when nothing is staged", async () => {
        // Committing the same unchanged file twice must not create a duplicate row.
        const hash = await commitMutation({
            action: "update", entity: "mock", entityId: MOCK_ID, entityName: "Audit Mock v2",
            workspaceId: TEST_WS, relPath: REL_PATH,
        });
        expect(hash).toBe("");
        expect((await queryLog({ workspaceId: TEST_WS })).total).toBe(2);
    });
});
