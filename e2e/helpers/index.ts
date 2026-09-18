import type { ElectronApplication, Page } from "@playwright/test";
import { expect } from "@playwright/test";

/**
 * Shared E2E helpers.
 *
 * IMPORTANT: these helpers assert. The previous generation of specs wrapped every
 * step in `if (await locator.isVisible())`, which meant a renamed button or a broken
 * panel produced a *green* test. A test that cannot fail is worse than no test, so
 * every helper here fails loudly when the UI it depends on is missing.
 */

/** Escape a string for safe use inside a RegExp. */
export function escapeRegex(input: string): string {
    return input.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** A unique, run-scoped name so tests never collide with seeded sample data. */
export function uniqueName(prefix: string): string {
    return `E2E ${prefix} ${Date.now().toString(36)}`;
}

export async function pause(page: Page, ms = 400): Promise<void> {
    await page.waitForTimeout(ms);
}

/**
 * Open a left-sidebar panel by its visible label and wait for the transition.
 * Fails if the panel entry point does not exist.
 */
export async function openPanel(page: Page, label: string): Promise<void> {
    const button = page.getByRole("button", { name: new RegExp(`^${escapeRegex(label)}$`, "i") }).first();
    await expect(button).toBeVisible({ timeout: 15_000 });
    await button.click();
    await pause(page, 700);
}

/**
 * Assert that a panel finished rendering by waiting for a stable anchor element
 * that only exists inside that panel.
 */
export async function expectPanelAnchor(page: Page, name: string | RegExp): Promise<void> {
    await expect(page.getByRole("button", { name }).first()).toBeVisible({ timeout: 10_000 });
}

/** Wait for the app shell to be interactive. */
export async function waitForAppReady(page: Page): Promise<void> {
    await page.waitForLoadState("domcontentloaded");
    await pause(page, 1500);
    await expect(page.locator("body")).toContainText(/\S/, { timeout: 15_000 });
}

/**
 * Call an IPC handler from the renderer. Useful for deterministic cleanup and for
 * seeding state that is awkward to drive through the UI.
 */
export async function ipcInvoke(app: ElectronApplication, channel: string, ...args: any[]): Promise<any> {
    return app.evaluate(async ({ ipcMain }, { channel, args }) => {
        const event = { sender: { send: () => { } } } as any;
        const handler = (ipcMain as any)._invokeHandlers?.get(channel);
        if (handler) return handler(event, ...args);
        return undefined;
    }, { channel, args });
}

/**
 * Navigate to a specific panel by its data-testid (`nav-<id>`).
 * Prefer `openPanel` — the sidebar exposes accessible names for every entry.
 */
export async function navigateTo(page: Page, panelId: string): Promise<void> {
    await page.click(`[data-testid="nav-${panelId}"]`);
    await pause(page, 300);
}

/** Get all visible items in a list panel. */
export async function getListItems(page: Page, listSelector: string): Promise<string[]> {
    return page.$$eval(`${listSelector} [data-testid="list-item"]`, (items) =>
        items.map((el) => el.textContent?.trim() ?? ""),
    );
}

/** Click a button by its accessible name. Fails if it is not present. */
export async function clickButton(page: Page, name: string | RegExp): Promise<void> {
    const button = page.getByRole("button", { name }).first();
    await expect(button).toBeVisible({ timeout: 10_000 });
    await button.click();
}

/** Verify a toast/notification message appears. */
export async function expectToast(page: Page, message: string): Promise<void> {
    await page.waitForSelector(`text=${message}`, { timeout: 5_000 });
}

/** Pick a protocol in the new-tab / new-mock chooser (e.g. "REST Mock"). */
export async function chooseProtocol(page: Page, label: RegExp): Promise<void> {
    await page.getByRole("button", { name: label }).click();
    await pause(page, 700);
}

/**
 * Open the Environments manager.
 *
 * It is reached through the titlebar's environment **dropdown**, not by a button of its own.
 * `EnvSelector` renders the toggle (titled "Switch environment"), and the "Manage Environments…"
 * item exists **only inside the menu that opens on click**. A locator for that label therefore
 * matches nothing until the dropdown is open — which is exactly how the previous version of this
 * test failed: it asserted a control that is not on screen at rest.
 *
 * `getByTitle` rather than `getByRole(..., { name })` is deliberate: the toggle's accessible name
 * comes from its *text content* (the active environment's name, e.g. "Development"), which changes
 * with the seeded data, while `title` is the stable, intention-revealing attribute.
 *
 * Written as the two steps a user actually performs, so a renamed menu item fails here with a
 * readable assertion instead of surfacing as a mysterious timeout in whichever test called it.
 */
export async function openEnvironmentsManager(page: Page): Promise<void> {
    const toggle = page.getByTitle("Switch environment").first();
    await expect(toggle).toBeVisible({ timeout: 15_000 });
    await toggle.click();

    const manage = page.getByRole("button", { name: /Manage Environments/i }).first();
    await expect(manage).toBeVisible({ timeout: 10_000 });
    await manage.click();
    await pause(page, 500);
}

/**
 * Type into the last visible CodeMirror editor. The protocol editors render several
 * editors per tab, so we walk backwards and use the first one that is visible.
 */
export async function fillVisibleCodeEditor(page: Page, value: string): Promise<void> {
    const editors = page.locator(".cm-content[contenteditable='true']");
    const count = await editors.count();
    for (let i = count - 1; i >= 0; i--) {
        const editor = editors.nth(i);
        if (await editor.isVisible().catch(() => false)) {
            await editor.click();
            await page.keyboard.press("ControlOrMeta+A");
            await page.keyboard.type(value);
            return;
        }
    }
    throw new Error("No visible editable code editor found");
}
