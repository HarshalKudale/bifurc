import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

vi.mock("electron", () => ({
    app: { getPath: () => "/mock/userData" },
}));

import {
    setSettingsPathOverride,
    loadSettings,
    saveSettings,
    appDataDir,
} from "@/store/appSettings";

describe("appSettings", () => {
    let tmpDir: string;
    let settingsFile: string;

    beforeEach(() => {
        tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "appsettings-test-"));
        settingsFile = path.join(tmpDir, "app.json");
        setSettingsPathOverride(settingsFile);
    });

    afterEach(() => {
        setSettingsPathOverride(null);
        fs.rmSync(tmpDir, { recursive: true, force: true });
    });

    describe("loadSettings()", () => {
        it("returns default settings when file does not exist", () => {
            const s = loadSettings();
            expect(s.port).toBe(80);
            expect(s.webhookPort).toBe(9101);
            expect(s.companionPort).toBe(9271);
            expect(s.minimizeToTray).toBe(true);
            expect(s.workspaces).toHaveLength(1);
            expect(s.activeWorkspaceId).toBeTruthy();
        });

        it("creates the file on first load", () => {
            loadSettings();
            expect(fs.existsSync(settingsFile)).toBe(true);
        });

        it("reads existing settings from file", () => {
            const custom = {
                port: 8080,
                webhookPort: 9200,
                companionPort: 9300,
                minimizeToTray: false,
                tlsEnabled: true,
                tlsCaCertPath: null,
                tlsCaKeyPath: null,
                workspaces: [{ id: "ws1", name: "Test", activeEnvironmentId: null }],
                activeWorkspaceId: "ws1",
                hasSeenWelcome: true,
            };
            fs.writeFileSync(settingsFile, JSON.stringify(custom));
            const s = loadSettings();
            expect(s.port).toBe(8080);
            expect(s.minimizeToTray).toBe(false);
            expect(s.tlsEnabled).toBe(true);
        });

        it("merges partial settings with defaults", () => {
            fs.writeFileSync(settingsFile, JSON.stringify({ port: 9090 }));
            const s = loadSettings();
            expect(s.port).toBe(9090);
            expect(s.webhookPort).toBe(9101); // default
        });
    });

    describe("saveSettings()", () => {
        it("writes settings to file", () => {
            const s = loadSettings();
            s.port = 3000;
            saveSettings(s);
            const raw = JSON.parse(fs.readFileSync(settingsFile, "utf-8"));
            expect(raw.port).toBe(3000);
        });

        it("creates parent directory if it does not exist", () => {
            const nestedPath = path.join(tmpDir, "nested", "dir", "app.json");
            setSettingsPathOverride(nestedPath);
            const s = loadSettings();
            saveSettings(s);
            expect(fs.existsSync(nestedPath)).toBe(true);
        });
    });

    describe("appDataDir()", () => {
        it("returns the directory containing the settings file", () => {
            const dir = appDataDir();
            expect(dir).toBe(tmpDir);
        });
    });
});
