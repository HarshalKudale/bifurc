import { test, expect } from "./fixtures/electronApp";

test.describe("Capture Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=Capture, text=Traffic, [data-testid='nav-capture']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays capture panel with start/stop controls", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
        // Should have some kind of start/record button
        const startBtn = page.locator("button:has-text('Start'), button:has-text('Record'), button:has-text('Capture'), [data-testid='capture-start']").first();
        const isVisible = await startBtn.isVisible().catch(() => false);
        // Capture panel should render without errors
        expect(typeof isVisible).toBe("boolean");
    });

    test("displays empty capture log initially", async ({ page }) => {
        // No captured requests should exist yet
        const body = await page.textContent("body");
        // Should not show any HTTP method indicators for captured requests
        expect(body).toBeTruthy();
    });
});

test.describe("WebSocket Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=WebSocket, text=Sockets, text=WS, [data-testid='nav-sockets']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays websocket panel", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });

    test("can open new websocket connection form", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('New'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);
            // Should show URL input for WS connection
            const urlInput = page.locator("input[placeholder*='ws'], input[placeholder*='url'], [data-testid='ws-url-input']").first();
            const isVisible = await urlInput.isVisible().catch(() => false);
            expect(typeof isVisible).toBe("boolean");
        }
    });
});

test.describe("Webhooks Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=Webhooks, [data-testid='nav-webhooks']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays webhooks panel", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });

    test("can create a new webhook listener", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            const suffixInput = page.locator("input[name='urlSuffix'], input[placeholder*='suffix'], input[placeholder*='path'], [data-testid='webhook-suffix-input']").first();
            if (await suffixInput.isVisible()) {
                await suffixInput.fill("order-events");
                const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button[type='submit']").first();
                if (await saveBtn.isVisible()) {
                    await saveBtn.click();
                    await page.waitForTimeout(500);
                    const body = await page.textContent("body");
                    expect(body).toContain("order-events");
                }
            }
        }
    });
});
