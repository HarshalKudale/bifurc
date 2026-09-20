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
 * True when `candidate` is `root` itself or lives underneath it. `path.relative` compares
 * case-insensitively on win32, which is what we want for drive-letter paths.
 */
function isInside(root: string, candidate: string): boolean {
    const rel = path.relative(path.resolve(root), path.resolve(candidate));
    return rel === "" || (!rel.startsWith("..") && !path.isAbsolute(rel));
}

/**
 * The directories the engine resolves its data dir *from*, read inside the running app.
 *
 * On the Electron path there are exactly two, and the shell supplies the first:
 *
 *   - `app.getPath("userData")` — handed to `setDataRoot()` at `src/main.ts:256`.
 *   - `process.env.LOCALAPPDATA` — `store/workspaceFs.ts#dataRoot()` and
 *     `store/appSettings.ts#settingsPath()` both short-circuit on it on win32, **before** consulting
 *     the data root. That ordering is what makes the assertion below subtle.
 *
 * `main.ts` consults neither `--data-dir` nor `BIFURC_DATA_DIR` (`store/paths.ts#resolveDataDir` is
 * for the CLI/Docker entrypoints, P8/P9), so those two are not on this path.
 */
interface EngineDataDirInputs {
    userData: string;
    localAppData: string | null;
    platform: string;
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

        // ── P6 work item 5: the engine's data dir must be inside this test's temp dir ─────────
        //
        // If it is not, every e2e test shares one data dir and overwrites the previous test's
        // state — which surfaces as intermittent, order-dependent failures that read as flakiness.
        //
        // The check is *containment of the inputs*, deliberately not a re-derivation of the
        // engine's formula. Both consumers above only ever `path.join` relative segments
        // ("Bifurc", "data", "app.json", `<wsId>`, `<kind>`) onto one of the two inputs, so if both
        // inputs are inside the temp dir then every path derived from them is too. Re-deriving the
        // formula here would duplicate it, and a copy can drift from the original silently.
        //
        // Scope: this asserts the *shell wiring* — that the overrides reach the app process. The
        // engine's own resolution order is covered by `tests/store/paths.test.ts` (all four
        // `resolveDataDir` branches) and `tests/store/workspaceFs.test.ts`.
        const inputs: EngineDataDirInputs = await app.evaluate(({ app: electronApp }) => ({
            userData: electronApp.getPath("userData"),
            localAppData: process.env.LOCALAPPDATA ?? null,
            platform: process.platform,
        }));

        // Non-vacuity: an empty probe would make the assertion below pass for the wrong reason.
        expect(inputs.userData, "the main-process probe returned no userData").toBeTruthy();

        const candidates: Array<[string, string | null]> = [
            [`app.getPath("userData")`, inputs.userData],
            ["process.env.LOCALAPPDATA", inputs.localAppData],
        ];
        const escaped = candidates
            .filter((entry): entry is [string, string] => !!entry[1])
            .filter(([, value]) => !isInside(userData, value))
            .map(([label, value]) => `${label} → ${value}`);

        // On win32 an unset LOCALAPPDATA is a failure, not a skip: the engine would fall back to the
        // developer's real profile, which is exactly the contamination this guards against.
        if (inputs.platform === "win32" && !inputs.localAppData) {
            escaped.push(
                "win32 but process.env.LOCALAPPDATA is unset — the engine would read the developer's real profile",
            );
        }

        expect(
            escaped,
            `the engine would resolve its data dir outside this test's temp dir (${userData}).\n` +
                `Every e2e test would then share one data dir and contaminate the next:\n  ${escaped.join("\n  ")}`,
        ).toEqual([]);

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
