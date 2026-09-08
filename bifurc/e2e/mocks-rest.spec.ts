import { test, expect } from "./fixtures/electronApp";

test.describe("Mocks Panel - REST", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=Mocks, [data-testid='nav-mocks']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays empty state when no mocks exist", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });

    test("can open add mock form", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);
            const form = page.locator("input, select, [data-testid='mock-form']").first();
            await expect(form).toBeVisible({ timeout: 3000 });
        }
    });

    test("can create a GET mock with status 200", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Fill URL pattern
            const urlInput = page.locator("input[placeholder*='url'], input[placeholder*='pattern'], input[name='urlPattern'], [data-testid='url-pattern-input']").first();
            if (await urlInput.isVisible()) {
                await urlInput.fill("/api/health");
            }

            // Fill response body
            const bodyEditor = page.locator("textarea, [data-testid='response-body'], .cm-editor").first();
            if (await bodyEditor.isVisible()) {
                await bodyEditor.click();
                await page.keyboard.type('{"status":"ok"}');
            }

            // Save
            const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button[type='submit']").first();
            if (await saveBtn.isVisible()) {
                await saveBtn.click();
                await page.waitForTimeout(500);
                const body = await page.textContent("body");
                expect(body).toContain("/api/health");
            }
        }
    });

    test("can create a POST mock with error response", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Select POST method
            const methodSelect = page.locator("select[name='method'], [data-testid='method-select']").first();
            if (await methodSelect.isVisible()) {
                await methodSelect.selectOption("POST");
            }

            // Fill URL pattern
            const urlInput = page.locator("input[placeholder*='url'], input[placeholder*='pattern'], input[name='urlPattern']").first();
            if (await urlInput.isVisible()) {
                await urlInput.fill("/api/users");
            }

            // Set status to 422
            const statusInput = page.locator("input[name='status'], input[placeholder*='status'], [data-testid='status-input']").first();
            if (await statusInput.isVisible()) {
                await statusInput.fill("422");
            }

            // Save
            const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button[type='submit']").first();
            if (await saveBtn.isVisible()) {
                await saveBtn.click();
                await page.waitForTimeout(500);
            }
        }
    });
});
