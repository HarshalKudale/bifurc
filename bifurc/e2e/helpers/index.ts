import type { ElectronApplication, Page } from "@playwright/test";

/**
 * Helper to call IPC handlers from tests (seed data, trigger actions).
 * Uses electronApp.evaluate() to invoke ipcMain handlers directly.
 */
export async function ipcInvoke(app: ElectronApplication, channel: string, ...args: any[]): Promise<any> {
    return app.evaluate(async ({ ipcMain }, { channel, args }) => {
        // Access the registered handler via electron internals
        const event = { sender: { send: () => { } } } as any;
        const handler = (ipcMain as any)._invokeHandlers?.get(channel);
        if (handler) return handler(event, ...args);
        return undefined;
    }, { channel, args });
}

/**
 * Navigate to a specific panel in the sidebar.
 */
export async function navigateTo(page: Page, panelId: string): Promise<void> {
    await page.click(`[data-testid="nav-${panelId}"]`);
    await page.waitForTimeout(300); // Brief wait for panel transition
}

/**
 * Wait for the app to finish initial loading.
 */
export async function waitForAppReady(page: Page): Promise<void> {
    // Wait for the main layout to appear
    await page.waitForSelector("[data-testid='app-layout']", { timeout: 15_000 });
}

/**
 * Get all visible items in a list panel.
 */
export async function getListItems(page: Page, listSelector: string): Promise<string[]> {
    return page.$$eval(`${listSelector} [data-testid="list-item"]`, (items) =>
        items.map((el) => el.textContent?.trim() ?? "")
    );
}

/**
 * Click the "Add" / "+" button in a panel.
 */
export async function clickAddButton(page: Page): Promise<void> {
    await page.click("[data-testid='add-button']");
}

/**
 * Fill a form field by its label.
 */
export async function fillField(page: Page, label: string, value: string): Promise<void> {
    const input = page.locator(`label:has-text("${label}") + input, label:has-text("${label}") input, [aria-label="${label}"]`).first();
    await input.fill(value);
}

/**
 * Click a button by its text content.
 */
export async function clickButton(page: Page, text: string): Promise<void> {
    await page.click(`button:has-text("${text}")`);
}

/**
 * Verify a toast/notification message appears.
 */
export async function expectToast(page: Page, message: string): Promise<void> {
    await page.waitForSelector(`text=${message}`, { timeout: 5_000 });
}
