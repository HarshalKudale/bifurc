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
    /** Exact visible label / aria-label */
    label: string;
    /** Where to find the panel trigger */
    trigger: "sidebar" | "titlebar" | "bottom";
    /** For duplicated labels like REST/SOAP/gRPC, which visible match to use */
    occurrence?: number;
    /** Opens a new draft tab after navigating (for tab-based panels) */
    openNewTab?: boolean;
}

const PANELS: PanelDef[] = [
    { name: "01-services", label: "Services", trigger: "sidebar" },
    { name: "02-health-bar", label: "Health Bar", trigger: "sidebar" },
    { name: "03-mappings", label: "Mappings", trigger: "sidebar" },
    { name: "04-proxy-rules", label: "Proxy Rules", trigger: "sidebar" },
    { name: "05-capture", label: "Capture", trigger: "sidebar" },
    { name: "06-requests", label: "Requests", trigger: "sidebar", openNewTab: true },
    { name: "07-mocks", label: "Mocks", trigger: "sidebar", openNewTab: true },
    { name: "08-websocket", label: "WebSocket", trigger: "sidebar", openNewTab: true },
    { name: "09-webhooks", label: "Webhooks", trigger: "sidebar", openNewTab: true },
    { name: "10-workspace", label: "Workspace", trigger: "titlebar" },
    { name: "11-environments", label: "Manage Environments", trigger: "titlebar" },
    { name: "12-settings", label: "Settings", trigger: "bottom" },
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
    const btn = page.getByRole("button", { name: new RegExp(`^${escapeRegex(label)}(?:\\s|$)`, "i") }).nth(occurrence);
    await btn.waitFor({ state: "attached", timeout: 5000 });
    await btn.evaluate((el: Element) => {
        (el as HTMLElement).scrollIntoView({ block: "center" });
        (el as HTMLElement).click();
    });
    await page.waitForTimeout(600);
}

async function openPanel(page: import("@playwright/test").Page, panel: PanelDef) {
    if (panel.trigger === "titlebar") {
        const btn = page.getByRole("button", { name: new RegExp(escapeRegex(panel.label), "i") }).nth(panel.occurrence ?? 0);
        await btn.waitFor({ state: "visible", timeout: 5000 });
        await btn.evaluate((el: Element) => {
            (el as HTMLElement).scrollIntoView({ block: "center" });
            (el as HTMLElement).click();
        });
        await page.waitForTimeout(600);
        return;
    }

    if (panel.trigger === "bottom") {
        const btn = page.getByRole("button", { name: new RegExp(`^${escapeRegex(panel.label)}$`, "i") }).last();
        await btn.waitFor({ state: "visible", timeout: 5000 });
        await btn.evaluate((el: Element) => {
            (el as HTMLElement).scrollIntoView({ block: "center" });
            (el as HTMLElement).click();
        });
        await page.waitForTimeout(600);
        return;
    }

    await clickNavItem(page, panel.label, panel.occurrence ?? 0);
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

        await openPanel(page, panel);

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
