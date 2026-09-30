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
import { openEnvironmentsManager } from "./helpers";
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
    /**
     * Where to find the panel trigger.
     *
     * `environments` is separate from `titlebar` because its label is **not on screen at rest**: the
     * titlebar renders the environment *selector* (`EnvSelector.tsx`), and "Manage Environments…" is
     * rendered only while that dropdown is open — `{open && (...)}` at `EnvSelector.tsx:46`. A locator
     * for the label therefore matches nothing until the dropdown is opened, which is exactly how this
     * tour failed at panel 11. `openEnvironmentsManager` in `./helpers` performs the two real steps.
     */
    trigger: "sidebar" | "titlebar" | "bottom" | "environments";
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
    { name: "10-workspace", label: "Workspace", trigger: "titlebar" },
    { name: "11-environments", label: "Manage Environments", trigger: "environments" },
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
    /**
     * Guard: dismiss any dropdown the previous panel left open.
     *
     * Both titlebar dropdowns render a full-screen backdrop — `<div class="fixed inset-0 z-40">`
     * (`WorkspaceDropdown.tsx:76`, `EnvSelector.tsx:48`) — whose whole job is to swallow the next click.
     * Because this tour navigates with `evaluate(el.click())`, which bypasses hit-testing, a leftover
     * backdrop is invisible to every step except the one real click (panel 11's
     * `openEnvironmentsManager`) — which is how the 30 s "intercepts pointer events" timeout happened.
     *
     * Scope, stated honestly: Escape closes the workspace dropdown (`WorkspaceSelector.tsx:75`) and is a
     * no-op when nothing is open. `EnvSelector` has no Escape handler, but its dropdown cannot be left
     * open — `onManage` closes it (`TitleBar.tsx:163-166`) — so Escape covers the reachable case. A
     * future panel whose dropdown closes only on a backdrop click will need that click here instead.
     */
    await page.keyboard.press("Escape");

    if (panel.trigger === "environments") {
        await openEnvironmentsManager(page);
        return;
    }

    if (panel.trigger === "titlebar") {
        /**
         * **Anchored, and the anchoring is load-bearing.** Unanchored, `/Workspace/i` matched the
         * workspace *switcher* first — its accessible name is `"SW Sample Workspace"` — so `.nth(0)`
         * clicked that instead of the Workspace *panel* button (`aria-label` is exactly `"Workspace"`,
         * `TitleBar.tsx:113` → `setPanel("workspace")`, `App.tsx:292`). Two consequences, the second
         * fatal:
         *
         *   1. `10-workspace.png` was a screenshot of the switcher dropdown, not the Workspace panel.
         *   2. That dropdown renders a full-screen backdrop — `<div class="fixed inset-0 z-40">`
         *      (`WorkspaceDropdown.tsx:76`) — whose whole job is to swallow the next click. Every other
         *      navigation click in this tour runs inside `evaluate(el.click())`, which bypasses
         *      hit-testing, so the backdrop was invisible to all of them **except the one real click in
         *      the tour**: panel 11's `openEnvironmentsManager`. That click then timed out for 30 s with
         *      `<div class="fixed inset-0 z-40"></div> ... intercepts pointer events`, and the failure
         *      snapshot showed `button "SW Sample Workspace" [expanded]` with its "Search workspaces…"
         *      box open.
         *
         * `app.spec.ts`'s environments test passes because it goes straight there with nothing else
         * open — which is why "the helper works" was the right clue and not the whole answer.
         */
        const btn = page.getByRole("button", { name: new RegExp(`^${escapeRegex(panel.label)}$`, "i") }).nth(panel.occurrence ?? 0);
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
