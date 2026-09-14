/**
 * Happy-path workflow test for **Git sync / publish — the remote half**.
 *
 * Bifurc's data story is "publish on demand, then sync": a workspace can be wired to a
 * git remote, pushed to, and cloned from on another machine. That entire surface —
 * `src/sync/syncManager.ts`, `src/sync/gitSyncOps.ts` — was previously covered only by
 * mocked-unit tests, and `gitSyncOps.ts` sat at **19%**. `setRemote()` in particular has
 * three materially different branches (empty→empty push, empty→non-empty clone, and the
 * refusal when both sides have data) and none of them was exercised.
 *
 * These drive the REAL registered IPC handlers against REAL git repositories: a throwaway
 * workspace on disk and a real **bare repo** standing in for the remote. No network access
 * is involved — a filesystem path is a perfectly valid git remote.
 *
 * Only Electron itself is mocked.
 *
 * Runs in the integration project (shared process-global data-root override ⇒
 * `fileParallelism: false`).
 */

import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { execFileSync } from "child_process";
import { setDataDirOverride, getGit, initWorkspaceRepo } from "@/store/gitStore";
import { initWorkspaceDir, wsDir } from "@/store/workspaceFs";
import { setSettingsPathOverride, loadSettings } from "@/store/appSettings";
import { getSyncConfig, getRemoteHead } from "@/sync/syncManager";

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

// ── Fixtures ─────────────────────────────────────────────────────────────────

const WS = "ws-sync";
const WS_NAME = "Sync Origin";
const CLONE_WS = "ws-clone";
const BAD_WS = "ws-bad";
const BRANCH = "main";
const ENTITY_ID = "mock-remote-1";

interface RootFixture {
    root: string;
    dataRoot: string;
    /** Point the stores at this root (clears the git cache). */
    activate: () => void;
    cleanup: () => void;
}

/** Git on Windows is happiest with forward slashes in remote URLs. */
function toUrl(p: string): string {
    return p.replace(/\\/g, "/");
}

/** Create a real bare repository to act as the remote. */
function makeBareRemote(): string {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-remote-"));
    execFileSync("git", ["init", "--bare", dir], { stdio: "ignore" });
    execFileSync("git", ["--git-dir", dir, "symbolic-ref", "HEAD", `refs/heads/${BRANCH}`], { stdio: "ignore" });
    return dir;
}

/** The remote's branch tip, or null when the remote has no commits yet. */
function remoteHead(dir: string): string | null {
    try {
        const out = execFileSync("git", ["--git-dir", dir, "rev-parse", `refs/heads/${BRANCH}`], {
            encoding: "utf-8",
        }).trim();
        return out || null;
    } catch {
        return null;
    }
}

/** A throwaway data root containing one workspace, ready for a git repo. */
function makeRoot(wsId: string, name: string): RootFixture {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-sync-"));
    const dataRoot = path.join(root, "data");
    fs.mkdirSync(path.join(dataRoot, wsId), { recursive: true });

    fs.writeFileSync(
        path.join(root, "app.json"),
        JSON.stringify(
            {
                port: 8080,
                webhookPort: 9101,
                companionPort: 9271,
                minimizeToTray: true,
                tlsEnabled: false,
                tlsCaCertPath: null,
                tlsCaKeyPath: null,
                workspaces: [{ id: wsId, name }],
                activeWorkspaceId: wsId,
                hasSeenWelcome: true,
                zoomLevel: 0,
                zoomLevelSetByUser: false,
                themeId: null,
            },
            null,
            2,
        ),
        "utf-8",
    );

    const fixture: RootFixture = {
        root,
        dataRoot,
        activate: () => {
            setDataDirOverride(dataRoot);
            setSettingsPathOverride(path.join(root, "app.json"));
        },
        cleanup: () => {
            try { fs.rmSync(root, { recursive: true, force: true }); } catch { /* best effort */ }
        },
    };

    fixture.activate();
    // Real app flow: the workspace dir (and its .gitignore) exists before the repo is created,
    // so the first commit always has something to commit.
    initWorkspaceDir(wsId, name);
    return fixture;
}

function writeMock(id: string, overrides: Record<string, any> = {}): void {
    const dir = path.join(wsDir(WS), "mocks");
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
        path.join(dir, `${id}.json`),
        JSON.stringify(
            {
                id,
                name: `Remote Mock ${id}`,
                method: "GET",
                urlPattern: "/remote",
                useRegex: false,
                capturedHeaders: {},
                capturedBody: "",
                responseStatus: 200,
                responseHeaders: {},
                responseBody: "{}",
                responseDelay: 0,
                createdAt: 1,
                folderId: null,
                workspaceId: WS,
                ...overrides,
            },
            null,
            2,
        ),
        "utf-8",
    );
}

function readMockAt(root: RootFixture, wsId: string, id: string): any {
    const p = path.join(root.dataRoot, wsId, "mocks", `${id}.json`);
    return JSON.parse(fs.readFileSync(p, "utf-8"));
}

let rootA: RootFixture;
let rootB: RootFixture;
let rootC: RootFixture;
let remoteDir: string;
let remoteUrl: string;

beforeAll(async () => {
    rootA = makeRoot(WS, WS_NAME);
    remoteDir = makeBareRemote();
    remoteUrl = toUrl(remoteDir);
    await initWorkspaceRepo(WS);
    registerSyncHandlers();
});

afterAll(() => {
    setDataDirOverride("");
    setSettingsPathOverride(null);
    rootA?.cleanup();
    rootB?.cleanup();
    rootC?.cleanup();
    try { fs.rmSync(remoteDir, { recursive: true, force: true }); } catch { /* best effort */ }
});

// ─────────────────────────────────────────────────────────────────────────────

describe("sync:setRemote — connecting an empty workspace to an empty remote", () => {
    it("wires up the remote and pushes the initial commit", async () => {
        const res = await invoke("sync:setRemote", WS, remoteUrl, BRANCH);

        expect(res.ok).toBe(true);
        expect(res.cloned).toBe(false);
    });

    it("leaves a branch tip on the remote", async () => {
        expect(remoteHead(remoteDir)).toMatch(/^[0-9a-f]{40}$/);
    });

    it("persists the sync config so the UI can show the connection", async () => {
        expect(getSyncConfig(WS)).toMatchObject({
            remote: remoteUrl,
            branch: BRANCH,
            autoSync: false,
        });
    });

    it("reports an idle state with no error and a push timestamp", async () => {
        const st = await invoke("sync:getState", WS);

        expect(st.status).toBe("idle");
        expect(st.error).toBeNull();
        expect(st.lastPushedAt).toBeTypeOf("number");
    });

    it("exposes the remote head for the polling loop", async () => {
        expect(await getRemoteHead(WS)).toBe(remoteHead(remoteDir));
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("sync:push — publishing local commits to the remote", () => {
    it("pushes to the remote as part of publishing an entity", async () => {
        rootA.activate();
        writeMock(ENTITY_ID);
        const before = remoteHead(remoteDir);

        await invoke("entity:publish", WS, [`mocks/${ENTITY_ID}.json`]);

        // Publishing is not just a local commit: with a remote configured it pushes too,
        // which is what makes "publish" behave like a save-and-share.
        expect(remoteHead(remoteDir)).not.toBe(before);
    });

    it("advances the remote tip for a commit that publish did not make", async () => {
        rootA.activate();
        const g = getGit(WS);
        fs.writeFileSync(
            path.join(wsDir(WS), "mocks", "manual-push.json"),
            JSON.stringify({ id: "manual-push", name: "Manual Push" }, null, 2),
            "utf-8",
        );
        await g.add("mocks/manual-push.json");
        await g.commit("test: manual local commit");

        const before = remoteHead(remoteDir);
        const res = await invoke("sync:push", WS);

        expect(res).toEqual({ ok: true });
        expect(remoteHead(remoteDir)).not.toBe(before);
    });

    it("records the push time in the sync state", async () => {
        const st = await invoke("sync:getState", WS);

        expect(st.lastPushedAt).toBeGreaterThan(0);
        expect(st.status).toBe("idle");
    });

    it("is a no-op when there is nothing new to push", async () => {
        const before = remoteHead(remoteDir);

        const res = await invoke("sync:push", WS);

        expect(res).toEqual({ ok: true });
        expect(remoteHead(remoteDir)).toBe(before);
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("sync:setRemote — cloning an existing remote into an empty workspace", () => {
    it("clones the remote and adopts its workspace identity", async () => {
        rootB = makeRoot(CLONE_WS, "Sync Clone");

        const res = await invoke("sync:setRemote", CLONE_WS, remoteUrl, BRANCH);

        expect(res.ok).toBe(true);
        expect(res.cloned).toBe(true);
        expect(res.adoptedId).toBe(WS);
    });

    it("lands the origin's entities on disk", () => {
        const cloned = readMockAt(rootB, WS, ENTITY_ID);

        expect(cloned.name).toBe(`Remote Mock ${ENTITY_ID}`);
        expect(fs.existsSync(path.join(rootB.dataRoot, WS, "mocks", `${ENTITY_ID}.json`))).toBe(true);
    });

    it("rewrites the local workspace entry to the remote identity", () => {
        rootB.activate();
        const settings = loadSettings();

        expect(settings.workspaces.find((w) => w.id === CLONE_WS)).toBeUndefined();
        expect(settings.workspaces.find((w) => w.id === WS)).toMatchObject({ name: WS_NAME });
    });

    it("persists the sync config under the adopted id", () => {
        expect(getSyncConfig(WS)).toMatchObject({ remote: remoteUrl, branch: BRANCH });
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("sync:pull — bringing remote commits down", () => {
    it("reports updated:false when the remote has nothing new", async () => {
        rootB.activate();

        const res = await invoke("sync:pull", WS);

        expect(res.ok).toBe(true);
        expect(res.updated).toBe(false);
    });

    it("fast-forwards to a new remote commit and names the changed entities", async () => {
        // Publish a new revision on the origin and push it.
        rootA.activate();
        writeMock(ENTITY_ID, { name: "Remote Updated Name" });
        await invoke("entity:publish", WS, [`mocks/${ENTITY_ID}.json`]);
        await invoke("sync:push", WS);

        // Pull it into the clone.
        rootB.activate();
        const res = await invoke("sync:pull", WS);

        expect(res.ok).toBe(true);
        expect(res.updated).toBe(true);
        expect(res.updatedIds).toContain(ENTITY_ID);
        expect(readMockAt(rootB, WS, ENTITY_ID).name).toBe("Remote Updated Name");
    });

    it("reports updated:false once the clone is level with the remote", async () => {
        const res = await invoke("sync:pull", WS);

        expect(res.ok).toBe(true);
        expect(res.updated).toBe(false);
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("sync:setAutoSync — the polling toggle", () => {
    it("enables auto sync on the persisted config", async () => {
        rootA.activate();

        const res = await invoke("sync:setAutoSync", WS, true);

        expect(res).toEqual({ ok: true });
        expect(getSyncConfig(WS)?.autoSync).toBe(true);
    });

    it("disables it again", async () => {
        await invoke("sync:setAutoSync", WS, false);

        expect(getSyncConfig(WS)?.autoSync).toBe(false);
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("sync:disconnect", () => {
    it("removes the origin remote and clears the sync config", async () => {
        rootA.activate();

        const res = await invoke("sync:disconnect", WS);

        expect(res).toEqual({ ok: true });
        expect(getSyncConfig(WS)).toBeNull();
        const remotes = await getGit(WS).getRemotes();
        expect(remotes.find((r) => r.name === "origin")).toBeUndefined();
    });

    it("makes sync:push fail cleanly", async () => {
        const res = await invoke("sync:push", WS);

        expect(res).toEqual({ ok: false, error: "No remote configured" });
    });

    it("makes sync:pull fail cleanly", async () => {
        const res = await invoke("sync:pull", WS);

        expect(res).toEqual({ ok: false, error: "No remote configured" });
    });

    it("makes the auto-sync toggle refuse", async () => {
        const res = await invoke("sync:setAutoSync", WS, true);

        expect(res).toEqual({ ok: false });
    });
});

// ─────────────────────────────────────────────────────────────────────────────

describe("failure paths", () => {
    it("reports an error for an unreachable remote", async () => {
        rootC = makeRoot(BAD_WS, "Bad Remote");
        const bogus = toUrl(path.join(os.tmpdir(), "bifurc-no-such-remote"));

        const res = await invoke("sync:setRemote", BAD_WS, bogus, BRANCH);

        expect(res.ok).toBe(false);
        expect(res.error).toMatch(/Cannot access remote/);
        const st = await invoke("sync:getState", BAD_WS);
        expect(st.status).toBe("error");
    });

    it("refuses to connect a non-empty workspace to a non-empty remote", async () => {
        rootC.activate();
        // Give the workspace local data so it is no longer a valid clone target.
        fs.writeFileSync(
            path.join(wsDir(BAD_WS), "mocks", "local-only.json"),
            JSON.stringify({ id: "local-only", name: "Local Only" }, null, 2),
            "utf-8",
        );

        const res = await invoke("sync:setRemote", BAD_WS, remoteUrl, BRANCH);

        expect(res.ok).toBe(false);
        expect(res.error).toMatch(/Remote is not empty/);
    });
});
