/**
 * Screenshot tour of every panel in the left sidebar.
 *
 * Tab-based panels (Mock / Request sub-panels, WebSocket, Webhooks) get a
 * second screenshot taken with a fresh draft tab open so the tab bar is
 * visible.
 *
 * All screenshots are written to <workspace-root>/screenshots/
 */

import { test } from "./fixtures/electronApp";
import path from "path";
import fs from "fs";

// ── Output directory ───────────────────────────────────────────────────────

const SCREENSHOT_DIR = path.resolve(__dirname, "..", "..", "screenshots");
fs.mkdirSync(SCREENSHOT_DIR, { recursive: true });

// ── Panel definitions ──────────────────────────────────────────────────────

interface PanelDef {
    /** Human-readable name for the screenshot file */
    name: string;
    /** Exact visible nav label */
    navLabel: string;
    /** For duplicated labels like REST/SOAP/gRPC, which visible match to use */
    navOccurrence?: number;
    /** Opens a new draft tab after navigating (for tab-based panels) */
    openNewTab?: boolean;
}

const PANELS: PanelDef[] = [
    // ── Routing (flat) ─────────────────────────────────────────────────────
    { name: "01-mappings", navLabel: "Mappings" },
    { name: "02-proxy-rules", navLabel: "Proxy Rules" },
    { name: "03-capture", navLabel: "Capture" },

    // ── API (flat) ──────────────────────────────────────────────────────────
    { name: "04-requests", navLabel: "Requests", openNewTab: true },
    { name: "05-mocks", navLabel: "Mocks", openNewTab: true },

    // ── Realtime (flat) ─────────────────────────────────────────────────────
    { name: "06-websocket", navLabel: "WebSocket", openNewTab: true },
    { name: "07-webhooks", navLabel: "Webhooks", openNewTab: true },

    // ── Tools (flat) ───────────────────────────────────────────────────────
    { name: "14-environments", navLabel: "Envs & Vars" },

    // ── Discovery (flat) ───────────────────────────────────────────────────
    { name: "16-services", navLabel: "Services" },

    // ── Monitoring (flat) ──────────────────────────────────────────────────
    { name: "17-health-bar", navLabel: "Health Bar" },
];

// ── Helpers ────────────────────────────────────────────────────────────────

/**
 * Save a screenshot of the actual Electron window.
 * `page.screenshot({ fullPage: true })` only captures the web contents viewport,
 * which can miss the bottom edge of the app window in Electron.
 */
async function shot(
    page: import("@playwright/test").Page,
    electronApp: import("@playwright/test").ElectronApplication,
    name: string,
) {
    const file = path.join(SCREENSHOT_DIR, `${name}.png`);
    await page.waitForTimeout(150);
    const browserWindow = await electronApp.browserWindow(page);
    const pngBase64 = await browserWindow.evaluate(async (win) => {
        const image = await win.capturePage();
        return image.toPNG().toString("base64");
    });
    fs.writeFileSync(file, Buffer.from(pngBase64, "base64"));
    console.log(`  ✓ ${file}`);
}

function escapeRegex(input: string): string {
    return input.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

async function clickNavItem(page: import("@playwright/test").Page, label: string, occurrence = 0) {
    const sidebar = page.locator("nav").first();
    const btn = sidebar.getByRole("button", { name: new RegExp(`^${escapeRegex(label)}(?:\\s|$)`, "i") }).nth(occurrence);
    await btn.waitFor({ state: "attached", timeout: 5000 });
    await btn.evaluate((el: Element) => {
        (el as HTMLElement).scrollIntoView({ block: "center" });
        (el as HTMLElement).click();
    });
    await page.waitForTimeout(600);
}

// ── Test ───────────────────────────────────────────────────────────────────

test.setTimeout(120_000);

test("screenshot tour of all sidebar panels", async ({ page, electronApp }) => {
    // Wait for the app to fully load
    await page.waitForLoadState("domcontentloaded");
    await page.waitForTimeout(2000);

    // Screenshot the initial state
    await shot(page, electronApp, "00-initial");

    for (const panel of PANELS) {
        console.log(`\nNavigating to: ${panel.name}`);

        await clickNavItem(page, panel.navLabel, panel.navOccurrence ?? 0);

        // 2. For tab-based panels, open a new draft tab by clicking the + button
        if (panel.openNewTab) {
            // TabBar + buttons always have title="New <something>" (starts with "New ").
            // This avoids matching unrelated buttons like "Stop webhook server".
            // Panels where the TabBar only renders when tabs exist (e.g. Webhooks) need
            // Strategy B: click the empty-state "New …" button in the main content.

            let clicked = false;

            // Strategy A: TabBar + button whose title starts with "New "
            try {
                const plusBtn = page.locator("button[title^='New ']").first();
                if (await plusBtn.isVisible({ timeout: 1200 })) {
                    await plusBtn.click();
                    clicked = true;
                }
            } catch { /* continue */ }

            // Strategy B: empty-state "New …" visible button (e.g. Webhooks)
            if (!clicked) {
                try {
                    const allBtns = page.locator("button");
                    const count = await allBtns.count();
                    for (let i = 0; i < count; i++) {
                        const btn = allBtns.nth(i);
                        if (!(await btn.isVisible().catch(() => false))) continue;
                        const text = (await btn.textContent().catch(() => "") ?? "").trim();
                        if (/^New [A-Z]/i.test(text)) {
                            await btn.click();
                            clicked = true;
                            break;
                        }
                    }
                } catch { /* ignore */ }
            }

            if (clicked) await page.waitForTimeout(900);
        }

        // 3. Screenshot
        await shot(page, electronApp, panel.name);
    }

    console.log(`\nAll screenshots saved to: ${SCREENSHOT_DIR}`);
});
