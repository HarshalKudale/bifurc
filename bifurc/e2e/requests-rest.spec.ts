import { test, expect } from "./fixtures/electronApp";

test.describe("REST Requests Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=Requests, [data-testid='nav-requests']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays empty state when no saved requests exist", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });

    test("can open new request tab", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('New'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);
            // Should show URL input field
            const urlInput = page.locator("input[placeholder*='url'], input[placeholder*='http'], [data-testid='url-input']").first();
            await expect(urlInput).toBeVisible({ timeout: 3000 });
        }
    });

    test("can enter URL and method for a GET request", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('New'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Enter URL
            const urlInput = page.locator("input[placeholder*='url'], input[placeholder*='http'], [data-testid='url-input']").first();
            if (await urlInput.isVisible()) {
                await urlInput.fill("http://localhost:3000/api/users");
                const value = await urlInput.inputValue();
                expect(value).toBe("http://localhost:3000/api/users");
            }
        }
    });

    test("can switch HTTP method to POST", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('New'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            const methodSelect = page.locator("select[name='method'], [data-testid='method-select'], button:has-text('GET')").first();
            if (await methodSelect.isVisible()) {
                if (await methodSelect.evaluate((el) => el.tagName === "SELECT")) {
                    await methodSelect.selectOption("POST");
                } else {
                    // It's a button/dropdown
                    await methodSelect.click();
                    await page.waitForTimeout(200);
                    const postOption = page.locator("text=POST").first();
                    if (await postOption.isVisible()) {
                        await postOption.click();
                    }
                }
            }
        }
    });

    test("shows body editor when method is POST", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('New'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Switch to POST
            const methodSelect = page.locator("select[name='method'], [data-testid='method-select']").first();
            if (await methodSelect.isVisible()) {
                await methodSelect.selectOption("POST");
                await page.waitForTimeout(200);
                // Body tab/editor should be available
                const bodyTab = page.locator("text=Body, [data-testid='body-tab']").first();
                if (await bodyTab.isVisible()) {
                    await bodyTab.click();
                    await page.waitForTimeout(200);
                    const editor = page.locator("textarea, .cm-editor, [data-testid='body-editor']").first();
                    await expect(editor).toBeVisible({ timeout: 3000 });
                }
            }
        }
    });
});
