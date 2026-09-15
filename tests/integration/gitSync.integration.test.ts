/**
 * Happy-path workflow test for **Git sync / publish — the local half**.
 *
 * The publish-on-demand model is core to Bifurc's data story: every create/update/delete
 * is written straight to disk, and the user then *publishes* it, which turns the change
 * into a git commit that the Audit Log and the History sidebar read back. A regression in
 * the commit **subject format** — the string the Audit Log parses into rows — would make
 * the whole Audit Log silently empty, and nothing asserted it before.
 *
 * Before this suite the path was covered only in mocked-unit form:
 * `src/sync/publishService.ts` sat at 54%, `src/sync/gitOps.ts` at 74% (branches 32%),
 * and `src/ipc/handlers/syncHandlers.ts` at 25%. These drive the REAL registered IPC
 * handlers against a REAL git repository in a temp workspace and assert the commits,
 * diffs and files that actually land on disk. Only Electron itself is mocked.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as path from "path";
import { createWorkspace, TEST_WS, TEST_WS_NAME, type WorkspaceFixture } from "./proxyHarness";
import { initWorkspaceDir } from "@bifurc/engine/store/workspaceFs";
import { initWorkspaceRepo, setDataDirOverride, queryLog, getGit } from "@bifurc/engine/store/gitStore";

// ── Electron: capture the real handlers instead of registering them ───────────

const { handlers } = vi.hoisted(() => ({
    handlers: new Map<string, (...args: any[]) => any>(),
}));

vi.mock("electron", () => ({
    ipcMain: {
        handle: (channel: string, fn: (...args: any[]) => any) => { handlers.set(channel, fn); },
        on: vi.fn(),
    },
    BrowserWindow: { getAllWindows: vi.fn(() => []) },
    dialog: { showSaveDialog: vi.fn(async () => ({ canceled: true, filePath: undefined })) },
    app: {
        getPath: vi.fn(() => "/tmp/test-user-data"),
        on: vi.fn(),
        whenReady: vi.fn(() => Promise.resolve()),
        commandLine: { appendSwitch: vi.fn() },
        quit: vi.fn(),
    },
}));

import { registerSyncHandlers } from "@/ipc/handlers/syncHandlers";

async function invoke<T = any>(channel: string, ...args: any[]): Promise<T> {
    const fn = handlers.get(channel);
    if (!fn) throw new Error(`No handler registered for "${channel}"`);
    return await fn({}, ...args);
}

// ── Fixture helpers ──────────────────────────────────────────────────────────

let ws: WorkspaceFixture;

function wsPath(...parts: string[]): string {
    return path.join(ws.dataRoot, TEST_WS, ...parts);
}

function writeJson(relPath: string, data: unknown): void {
    const abs = wsPath(...relPath.split("/"));
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, JSON.stringify(data, null, 2), "utf-8");
}

function readJson(relPath: string): any {
    return JSON.parse(fs.readFileSync(wsPath(...relPath.split("/")), "utf-8"));
}

function mockObj(id: string, overrides: Record<string, any> = {}): any {
    return {
        id,
        name: `Sync Mock ${id}`,
        method: "GET",
        urlPattern: "/sync",
        useRegex: false,
        capturedHeaders: {},
        capturedBody: "",
        responseStatus: 200,
        responseHeaders: {},
        responseBody: "{}",
        responseDelay: 0,
        createdAt: 1,
        folderId: null,
        workspaceId: TEST_WS,
        ...overrides,
    };
}

/** Write a mock at `mocks/<id>.json`. */
function writeMock(id: string, overrides: Record<string, any> = {}): void {
    writeJson(`mocks/${id}.json`, mockObj(id, overrides));
}

/** Write a mock inside a folder: `mocks/<folder>/<id>.json`. */
function writeFolderMock(folder: string, id: string, overrides: Record<string, any> = {}): void {
    writeJson(`mocks/${folder}/${id}.json`, mockObj(id, { folderId: folder, ...overrides }));
}

const git = () => getGit(TEST_WS);

/** Commit subjects, newest first. */
async function subjects(): Promise<string[]> {
    const raw = await git().raw(["log", "--format=%s"]);
    return raw.trim().split("\n").filter(Boolean);
}

const head = async (): Promise<string> => (await git().revparse(["HEAD"])).trim();

function exists(relPath: string): boolean {
    return fs.existsSync(wsPath(...relPath.split("/")));
}

beforeAll(async () => {
    ws = createWorkspace();
    // Clear gitStore's per-workspace git cache so it points at this temp dir.
    setDataDirOverride(ws.dataRoot);

    // The real app initialises the workspace directory (which writes .gitignore) before
    // creating the repo — mirror that so the repo layout matches production.
    initWorkspaceDir(TEST_WS, TEST_WS_NAME);
    await initWorkspaceRepo(TEST_WS);

    registerSyncHandlers();
});

afterAll(() => {
    setDataDirOverride("");
    ws?.cleanup();
});

// ─────────────────────────────────────────────────────────────────────────────

describe("git publish — turning a change into a commit", () => {
    it("publishes a brand-new entity as a `create` commit", async () => {
        writeMock("mock-pub-create");

        const res = await invoke("entity:publish", TEST_WS, ["mocks/mock-pub-create.json"]);

        expect(res).toEqual({ ok: true });
        expect((await subjects())[0]).toBe("create mock Sync Mock mock-pub-create");
    });

    it("writes the entity id, workspace id and actor into the commit body", async () => {
        const raw = await git().raw(["log", "-1", "--format=%B"]);

        expect(raw).toContain("entity-id: mock-pub-create");
        expect(raw).toContain(`workspace-id: ${TEST_WS}`);
        expect(raw).toMatch(/actor: .+/);
    });

    it("makes the published change visible in the Audit Log", async () => {
        const { entries, total } = await queryLog({ workspaceId: TEST_WS, entityId: "mock-pub-create" });

        expect(total).toBe(1);
        expect(entries[0]).toMatchObject({
            action: "create",
            entity: "mock",
            entityName: "Sync Mock mock-pub-create",
            entityId: "mock-pub-create",
            workspaceId: TEST_WS,
        });
    });

    it("publishes an edit as an `update` commit named after the entity", async () => {
        writeMock("mock-pub-create", { name: "Sync Mock Renamed" });

        await invoke("entity:publish", TEST_WS, ["mocks/mock-pub-create.json"]);

        expect((await subjects())[0]).toBe("update mock Sync Mock Renamed");
    });

    it("does not create an empty commit when nothing changed", async () => {
        const before = await head();

        const res = await invoke("entity:publish", TEST_WS, ["mocks/mock-pub-create.json"]);

        expect(res).toEqual({ ok: true });
        expect(await head()).toBe(before);
    });

    it("honours an explicit commit message", async () => {
        writeMock("mock-pub-create", { name: "Sync Mock Custom" });

        await invoke("entity:publish", TEST_WS, ["mocks/mock-pub-create.json"], "chore: manual message");

        expect((await subjects())[0]).toBe("chore: manual message");
    });

    it("publishes a deletion as a `delete` commit named after the entity id", async () => {
        fs.unlinkSync(wsPath("mocks", "mock-pub-create.json"));

        await invoke("entity:publish", TEST_WS, ["mocks/mock-pub-create.json"]);

        expect((await subjects())[0]).toBe("delete mock mock-pub-create");
    });

    it("publishes several paths as a single commit", async () => {
        writeMock("mock-pub-multi");
        writeJson("mocks/index.json", { folders: [], order: ["mock-pub-multi"] });

        const res = await invoke("entity:publish", TEST_WS, [
            "mocks/mock-pub-multi.json",
            "mocks/index.json",
        ]);

        expect(res).toEqual({ ok: true });
        expect((await subjects())[0]).toBe("update 2 files");
    });

    it("bundles a folder with several entities into one commit", async () => {
        writeFolderMock("Bundle Folder", "mf-1", { name: "Bundle One" });
        writeFolderMock("Bundle Folder", "mf-2", { name: "Bundle Two" });

        const res = await invoke("entity:publish", TEST_WS, ["mocks/Bundle Folder/"]);

        expect(res).toEqual({ ok: true });
        expect((await subjects())[0]).toBe('update mock folder "Bundle Folder"');

        // The bundled body lists every entity id so the History sidebar can still link them.
        const raw = await git().raw(["log", "-1", "--format=%B"]);
        expect(raw).toContain("entity-ids: ");
        expect(raw).toContain("mf-1");
        expect(raw).toContain("mf-2");
    });

    it("treats a folder holding exactly one entity as a single-entity commit", async () => {
        writeFolderMock("Solo Folder", "sf-1", { name: "Solo One" });

        await invoke("entity:publish", TEST_WS, ["mocks/Solo Folder/"]);

        expect((await subjects())[0]).toBe("create mock Solo One");
        const raw = await git().raw(["log", "-1", "--format=%B"]);
        expect(raw).toContain("entity-id: sf-1");
    });

    it("publishes a named folder through the folder:publish handler", async () => {
        writeFolderMock("Handler Folder", "hf-1", { name: "Handler One" });

        const res = await invoke("folder:publish", TEST_WS, "mocks", "Handler Folder");

        expect(res).toEqual({ ok: true });
        expect((await subjects())[0]).toBe("create mock Handler One");
    });

    it("publishes an entire entity kind when the folder name is null", async () => {
        writeFolderMock("Whole Kind", "wk-1", { name: "Whole One" });

        const res = await invoke("folder:publish", TEST_WS, "mocks", null);

        expect(res).toEqual({ ok: true });
        expect((await subjects())[0]).toMatch(/^(create|update|delete) mock/);
    });

    it("publishes through git:sync (the generic file-centric entry point)", async () => {
        writeMock("mock-pub-generic", { name: "Generic Publish" });

        const res = await invoke("git:sync", TEST_WS, ["mocks/mock-pub-generic.json"]);

        expect(res).toEqual({ ok: true });
        expect((await subjects())[0]).toBe("create mock Generic Publish");
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("git:diff — what changed relative to HEAD", () => {
    it("reports a committed, unmodified file as clean", async () => {
        writeMock("mock-diff-clean");
        await invoke("entity:publish", TEST_WS, ["mocks/mock-diff-clean.json"]);

        const res = await invoke("git:diff", TEST_WS, "mocks/mock-diff-clean.json");

        expect(res.hasDiff).toBe(false);
        expect(res.status).toBe("clean");
        expect(res.diff).toBe("");
    });

    it("returns the unified diff for a modified file", async () => {
        writeMock("mock-diff-mod", { name: "Before Edit" });
        await invoke("entity:publish", TEST_WS, ["mocks/mock-diff-mod.json"]);
        writeMock("mock-diff-mod", { name: "After Edit" });

        const res = await invoke("git:diff", TEST_WS, "mocks/mock-diff-mod.json");

        expect(res.hasDiff).toBe(true);
        expect(res.status).toBe("modified");
        expect(res.diff).toMatch(/^-.*Before Edit/m);
        expect(res.diff).toMatch(/^\+.*After Edit/m);
    });

    it("returns the file contents for a brand-new untracked file", async () => {
        writeMock("mock-diff-new", { name: "Untracked Entity" });

        const res = await invoke("git:diff", TEST_WS, "mocks/mock-diff-new.json");

        expect(res.hasDiff).toBe(true);
        expect(res.status).toBe("new");
        expect(JSON.parse(res.current).name).toBe("Untracked Entity");
    });

    it("reports a file deleted from disk but still in HEAD", async () => {
        writeMock("mock-diff-del");
        await invoke("entity:publish", TEST_WS, ["mocks/mock-diff-del.json"]);
        fs.unlinkSync(wsPath("mocks", "mock-diff-del.json"));

        const res = await invoke("git:diff", TEST_WS, "mocks/mock-diff-del.json");

        expect(res.hasDiff).toBe(true);
        expect(res.status).toBe("deleted");
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("git:discard — throwing away uncommitted work", () => {
    it("reverts a modified file back to its committed contents", async () => {
        writeMock("mock-discard-mod", { name: "Committed Name" });
        await invoke("entity:publish", TEST_WS, ["mocks/mock-discard-mod.json"]);
        writeMock("mock-discard-mod", { name: "Unsaved Name" });

        const res = await invoke("git:discard", TEST_WS, "mocks/mock-discard-mod.json");

        expect(res).toEqual({ ok: true });
        expect(readJson("mocks/mock-discard-mod.json").name).toBe("Committed Name");
    });

    it("leaves the file clean once the change is discarded", async () => {
        const res = await invoke("git:diff", TEST_WS, "mocks/mock-discard-mod.json");

        expect(res.status).toBe("clean");
        expect(res.hasDiff).toBe(false);
    });

    it("deletes an untracked file from disk", async () => {
        writeMock("mock-discard-new");

        const res = await invoke("git:discard", TEST_WS, "mocks/mock-discard-new.json");

        expect(res).toEqual({ ok: true });
        expect(exists("mocks/mock-discard-new.json")).toBe(false);
    });

    it("re-syncs names.json so the sidebar shows the restored name", async () => {
        writeMock("mock-discard-names", { name: "Original Sidebar Name" });
        await invoke("entity:publish", TEST_WS, ["mocks/mock-discard-names.json"]);
        writeMock("mock-discard-names", { name: "Draft Sidebar Name" });

        await invoke("git:discard", TEST_WS, "mocks/mock-discard-names.json");

        const names = readJson("mocks/names.json");
        expect(names["mock-discard-names"].name).toBe("Original Sidebar Name");
    });

    it("restores through entity:restore (the backward-compatible alias)", async () => {
        writeMock("mock-restore", { name: "Restore Original" });
        await invoke("entity:publish", TEST_WS, ["mocks/mock-restore.json"]);
        writeMock("mock-restore", { name: "Restore Draft" });

        const res = await invoke("entity:restore", TEST_WS, "mocks/mock-restore.json");

        expect(res).toEqual({ ok: true });
        expect(readJson("mocks/mock-restore.json").name).toBe("Restore Original");
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("git:history — the History sidebar", () => {
    it("returns only the commits that touched a given file", async () => {
        writeMock("mock-hist-a", { name: "History A" });
        await invoke("entity:publish", TEST_WS, ["mocks/mock-hist-a.json"]);
        writeMock("mock-hist-a", { name: "History A v2" });
        await invoke("entity:publish", TEST_WS, ["mocks/mock-hist-a.json"]);

        // An unrelated publish that must NOT appear in this file's history.
        writeMock("mock-hist-b", { name: "History B" });
        await invoke("entity:publish", TEST_WS, ["mocks/mock-hist-b.json"]);

        const res = await invoke("git:history", TEST_WS, "mocks/mock-hist-a.json");

        expect(res.total).toBe(2);
        expect(res.entries.map((e: any) => e.entityName)).toEqual([
            "History A v2",
            "History A",
        ]);
    });

    it("normalises a Windows-style path before filtering", async () => {
        const res = await invoke("history:list", {
            workspaceId: TEST_WS,
            filePath: "mocks\\mock-hist-a.json",
        });

        expect(res.total).toBe(2);
    });

    it("returns the before/after pair for a commit in the file's history", async () => {
        const { entries } = await invoke("git:history", TEST_WS, "mocks/mock-hist-a.json");
        const latest = entries[0].commitHash;

        const res = await invoke("history:diff", latest, "mocks/mock-hist-a.json", TEST_WS);

        expect(res.after.name).toBe("History A v2");
        expect(res.before.name).toBe("History A");
    });

    it("caps results at the requested limit", async () => {
        const res = await invoke("git:history", TEST_WS, "mocks/mock-hist-a.json", { limit: 1 });

        expect(res.entries).toHaveLength(1);
        expect(res.total).toBe(2);
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("sync:getEntityStatus — the per-entity dirty markers", () => {
    it("classifies clean, modified, new and deleted entities", async () => {
        // clean
        writeMock("mock-status-clean");
        await invoke("entity:publish", TEST_WS, ["mocks/mock-status-clean.json"]);

        // modified
        writeMock("mock-status-mod", { name: "Status Mod" });
        await invoke("entity:publish", TEST_WS, ["mocks/mock-status-mod.json"]);
        writeMock("mock-status-mod", { name: "Status Mod Edited" });

        // new (never published)
        writeMock("mock-status-new");

        // deleted
        writeMock("mock-status-del");
        await invoke("entity:publish", TEST_WS, ["mocks/mock-status-del.json"]);
        fs.unlinkSync(wsPath("mocks", "mock-status-del.json"));

        const status = await invoke("sync:getEntityStatus", TEST_WS);

        expect(status["mocks/mock-status-clean.json"]).toBe("clean");
        expect(status["mocks/mock-status-mod.json"]).toBe("modified");
        expect(status["mocks/mock-status-new.json"]).toBe("new");
        expect(status["mocks/mock-status-del.json"]).toBe("deleted");
    });

    it("clears the dirty marker once the change is published", async () => {
        await invoke("entity:publish", TEST_WS, ["mocks/mock-status-mod.json"]);

        const status = await invoke("sync:getEntityStatus", TEST_WS);

        expect(status["mocks/mock-status-mod.json"]).toBe("clean");
    });

    it("never reports bookkeeping files as entities", async () => {
        const status = await invoke("sync:getEntityStatus", TEST_WS);

        expect(status["mocks/index.json"]).toBeUndefined();
        expect(status["mocks/enabled.json"]).toBeUndefined();
        expect(status["mocks/names.json"]).toBeUndefined();
    });
});
