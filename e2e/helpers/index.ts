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
 * The editor's autosave debounce, in milliseconds.
 *
 * Mirrors `AUTOSAVE_DELAY_MS` in `packages/gui/src/hooks/useDebouncedAutosave.ts`. The GUI cannot
 * import this file and this file cannot import the GUI, so the two are kept in step by this comment
 * alone — if that constant moves, move this one, or every "edit a saved entity" spec starts
 * asserting before the write has landed.
 */
export const AUTOSAVE_DEBOUNCE_MS = 1000;

/**
 * Wait out the autosave debounce **and** the write round-trip.
 *
 * Editing a saved entity has no Save button any more: the write fires one second after the last
 * change. A spec that asserts straight after a `fill` is racing that timer — invisible on an idle
 * machine, a flake on a loaded one. Use this instead of a bare `pause`, and only for edits to
 * entities that already exist; a **create** is still an explicit click.
 */
export async function awaitAutosave(page: Page, extraMs = 700): Promise<void> {
    await pause(page, AUTOSAVE_DEBOUNCE_MS + extraMs);
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
 * Rename the n-th open editor tab via double-click (the tab bar's inline rename).
 * Tabs are named via the tab itself since the inline name field was removed from
 * the request editor header.
 */
export async function renameTab(page: Page, name: string, index = 0): Promise<void> {
    await page.locator("[data-tab-id]").nth(index).dblclick();
    const input = page.getByTestId("tab-rename-input");
    await expect(input).toBeVisible({ timeout: 5_000 });
    await input.fill(name);
    await input.press("Enter");
    await expect(input).toBeHidden({ timeout: 5_000 });
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
