import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

vi.mock("electron", () => ({
    app: { getPath: () => "/mock/userData" },
}));

import {
    setDataRootOverride,
    initWorkspaceDir,
    writeEntity,
    deleteEntityFile,
    readAllEntities,
    sanitizeDirName,
    wsDir,
    readEnabledSet,
    writeEnabledSet,
    bootstrapEnabledSet,
} from "@/store/workspaceFs";

describe("workspaceFs", () => {
    let tmpDir: string;

    beforeEach(() => {
        tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "wsfs-test-"));
        setDataRootOverride(tmpDir);
    });

    afterEach(() => {
        setDataRootOverride(null);
        fs.rmSync(tmpDir, { recursive: true, force: true });
    });

    describe("sanitizeDirName()", () => {
        it("removes invalid filesystem characters", () => {
            expect(sanitizeDirName('file<>:"/\\|?*name')).toBe("filename");
        });

        it("returns 'unnamed' for empty string after sanitization", () => {
            expect(sanitizeDirName("???")).toBe("unnamed");
        });

        it("trims whitespace", () => {
            expect(sanitizeDirName("  hello  ")).toBe("hello");
        });

        it("passes through valid names unchanged", () => {
            expect(sanitizeDirName("my-folder_123")).toBe("my-folder_123");
        });
    });

    describe("initWorkspaceDir()", () => {
        it("creates workspace directory structure", () => {
            initWorkspaceDir("ws-test", "Test Workspace");
            const wsPath = wsDir("ws-test");
            expect(fs.existsSync(wsPath)).toBe(true);
            expect(fs.existsSync(path.join(wsPath, "mappings"))).toBe(true);
            expect(fs.existsSync(path.join(wsPath, "rules"))).toBe(true);
            expect(fs.existsSync(path.join(wsPath, "environments"))).toBe(true);
            expect(fs.existsSync(path.join(wsPath, "mocks"))).toBe(true);
            expect(fs.existsSync(path.join(wsPath, "requests"))).toBe(true);
            expect(fs.existsSync(path.join(wsPath, "sockets"))).toBe(true);
            expect(fs.existsSync(path.join(wsPath, "webhooks"))).toBe(true);
        });

        it("creates workspace.json file", () => {
            initWorkspaceDir("ws-test2", "My WS");
            const wsFile = path.join(wsDir("ws-test2"), "workspace.json");
            expect(fs.existsSync(wsFile)).toBe(true);
            const data = JSON.parse(fs.readFileSync(wsFile, "utf-8"));
            expect(data.name).toBe("My WS");
            expect(data.id).toBe("ws-test2");
        });

        it("creates .gitignore file", () => {
            initWorkspaceDir("ws-test3", "WS");
            const gitignore = path.join(wsDir("ws-test3"), ".gitignore");
            expect(fs.existsSync(gitignore)).toBe(true);
            const content = fs.readFileSync(gitignore, "utf-8");
            expect(content).toContain("capture/");
        });

        it("is idempotent (does not overwrite existing workspace.json)", () => {
            initWorkspaceDir("ws-idem", "First");
            initWorkspaceDir("ws-idem", "Second");
            const data = JSON.parse(fs.readFileSync(path.join(wsDir("ws-idem"), "workspace.json"), "utf-8"));
            expect(data.name).toBe("First");
        });
    });

    describe("writeEntity() and readAllEntities()", () => {
        beforeEach(() => {
            initWorkspaceDir("ws-ent", "Entity Test");
        });

        it("writes and reads back an entity", () => {
            const entity = { id: "e1", name: "Test Entity", extra: "data" };
            writeEntity("ws-ent", "mappings", "e1", entity);
            const all = readAllEntities("ws-ent", "mappings");
            expect(all).toHaveLength(1);
            expect(all[0].id).toBe("e1");
            expect(all[0].name).toBe("Test Entity");
        });

        it("writes multiple entities", () => {
            writeEntity("ws-ent", "mappings", "a", { id: "a", name: "A" });
            writeEntity("ws-ent", "mappings", "b", { id: "b", name: "B" });
            const all = readAllEntities("ws-ent", "mappings");
            expect(all).toHaveLength(2);
        });

        it("overwrites entity with same ID", () => {
            writeEntity("ws-ent", "mocks", "x", { id: "x", name: "Original" });
            writeEntity("ws-ent", "mocks", "x", { id: "x", name: "Updated" });
            const all = readAllEntities("ws-ent", "mocks");
            expect(all).toHaveLength(1);
            expect(all[0].name).toBe("Updated");
        });
    });

    describe("deleteEntityFile()", () => {
        beforeEach(() => {
            initWorkspaceDir("ws-del", "Delete Test");
        });

        it("removes an entity file", () => {
            writeEntity("ws-del", "rules", "r1", { id: "r1", name: "Rule" });
            deleteEntityFile("ws-del", "rules", "r1");
            const all = readAllEntities("ws-del", "rules");
            expect(all).toHaveLength(0);
        });

        it("does not throw when entity does not exist", () => {
            expect(() => deleteEntityFile("ws-del", "rules", "nonexistent")).not.toThrow();
        });
    });

    describe("enabledSet operations", () => {
        beforeEach(() => {
            initWorkspaceDir("ws-en", "Enabled Test");
        });

        it("bootstrapEnabledSet returns a Set of existing entity IDs", () => {
            writeEntity("ws-en", "mappings", "m1", { id: "m1", name: "M1" });
            writeEntity("ws-en", "mappings", "m2", { id: "m2", name: "M2" });
            const set = bootstrapEnabledSet("ws-en", "mappings");
            expect(set.has("m1")).toBe(true);
            expect(set.has("m2")).toBe(true);
        });

        it("writeEnabledSet and readEnabledSet round-trip", () => {
            const set = new Set(["a", "b", "c"]);
            writeEnabledSet("ws-en", "mocks", set);
            const read = readEnabledSet("ws-en", "mocks");
            expect(read).not.toBeNull();
            expect(read!.has("a")).toBe(true);
            expect(read!.has("b")).toBe(true);
            expect(read!.has("c")).toBe(true);
            expect(read!.size).toBe(3);
        });

        it("readEnabledSet returns null when file does not exist", () => {
            const result = readEnabledSet("ws-en", "nonexistent-kind");
            expect(result).toBeNull();
        });
    });
});
