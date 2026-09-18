import { test as base, expect, type ElectronApplication, type Page } from "@playwright/test";
import { _electron as electron } from "playwright";
import path from "path";
import fs from "fs";
import os from "os";
import { writeSampleWorkspace, readRealThemeId } from "./sampleData";

export interface ElectronFixtures {
    electronApp: ElectronApplication;
    page: Page;
    userData: string;
}

/**
 * Custom Playwright fixture that launches the Electron app with an isolated userData dir.
 * Each test gets a fresh data directory to avoid state leakage.
 * The workspace is pre-populated with sample data for realistic screenshots and tests.
 */
export const test = base.extend<ElectronFixtures>({
    userData: async ({ }, use) => {
        const dir = fs.mkdtempSync(path.join(os.tmpdir(), "lp-e2e-"));

        // Pre-populate with sample workspace data
        // App expects: ${LOCALAPPDATA}/Bifurc/app.json and ${LOCALAPPDATA}/Bifurc/data/
        const appDir = path.join(dir, "Bifurc");
        const dataDir = path.join(appDir, "data");
        fs.mkdirSync(dataDir, { recursive: true });
        writeSampleWorkspace(dataDir, readRealThemeId());

        await use(dir);
        // Cleanup after test
        fs.rmSync(dir, { recursive: true, force: true });
    },

    electronApp: async ({ userData }, use) => {
        const appPath = path.resolve(__dirname, "..", "..");
        // Isolate Electron's own userData dir (localStorage/session partition, e.g. the
        // theme preference) — this is separate from the LOCALAPPDATA override below, which
        // only isolates our own app.json/workspace data files. Without this, every e2e run
        // reuses the developer's real profile's localStorage (e.g. a leftover "terminal"
        // theme selection) instead of starting with defaults.
        const electronUserDataDir = path.join(userData, "electron-userdata");
        const app = await electron.launch({
            args: [appPath, `--user-data-dir=${electronUserDataDir}`],
            env: {
                ...process.env,
                NODE_ENV: "test",
                LOCALAPPDATA: userData, // Override where settings/data are stored on Windows
                XDG_CONFIG_HOME: userData, // Linux
                HOME: userData, // macOS/Linux fallback
                LP_E2E: "1",
            },
        });
        await use(app);
        await app.close();
    },

    page: async ({ electronApp }, use) => {
        const window = await electronApp.firstWindow();

        /**
         * Accept the Terms of Service screen before the app renders.
         *
         * `renderer/App.tsx:23` gates the entire shell on
         * `usePersistedState<boolean>("app:tos-accepted", false)` — if that key is absent the app
         * returns `<TermsAcceptanceScreen />` and nothing else is reachable. `usePersistedState` is
         * backed by plain `localStorage` (`renderer/lib/storage.ts`), so this is **per-profile
         * renderer state, not `app.json`**.
         *
         * That distinction is the whole reason this lives here rather than in `sampleData.ts`:
         * `writeSampleWorkspace()` writes `app.json` (`hasSeenWelcome: true`, which drives
         * `app:isFirstLaunch`) and cannot reach `localStorage`. The fresh `--user-data-dir` created
         * above starts with empty storage, so without this every test lands on the TOS screen.
         *
         * `addInitScript` runs before any page script on the **next** navigation, so the reload
         * below is what makes it take effect — it is seeded before `App.tsx` reads the key. The
         * value is JSON-encoded because `readStorage` does `JSON.parse`, and `JSON.parse("true")`
         * is the boolean the hook expects.
         */
        await window.addInitScript(() => {
            try {
                localStorage.setItem("app:tos-accepted", "true");
            } catch {
                /* Storage unavailable: the TOS screen will show and the test will fail loudly. */
            }
        });
        await window.reload();

        // Wait for the app to be fully loaded
        await window.waitForLoadState("domcontentloaded");
        await use(window);
    },
});

export { expect };
