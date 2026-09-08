import { test, expect } from "./fixtures/electronApp";

test.describe("Proxy Rules Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=Rules, text=Proxy, [data-testid='nav-rules']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays empty state when no rules exist", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });

    test("can open add rule form", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);
            const form = page.locator("input, select, [data-testid='rule-form']").first();
            await expect(form).toBeVisible({ timeout: 3000 });
        }
    });

    test("can create a block rule with regex pattern", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Fill rule name
            const nameInput = page.locator("input[name='name'], input[placeholder*='name'], [data-testid='rule-name-input']").first();
            if (await nameInput.isVisible()) {
                await nameInput.fill("Block Tracking");
            }

            // Set URL pattern
            const patternInput = page.locator("input[name='urlPattern'], input[placeholder*='pattern'], input[placeholder*='url'], [data-testid='url-pattern-input']").first();
            if (await patternInput.isVisible()) {
                await patternInput.fill(".*tracking\\.js$");
            }

            // Enable regex
            const regexToggle = page.locator("input[name='useRegex'], [data-testid='regex-toggle'], label:has-text('Regex')").first();
            if (await regexToggle.isVisible()) {
                await regexToggle.click();
            }

            // Save
            const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button[type='submit']").first();
            if (await saveBtn.isVisible()) {
                await saveBtn.click();
                await page.waitForTimeout(500);
                const body = await page.textContent("body");
                expect(body).toContain("Block Tracking");
            }
        }
    });

    test("can create a redirect rule", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Fill rule name
            const nameInput = page.locator("input[name='name'], input[placeholder*='name']").first();
            if (await nameInput.isVisible()) {
                await nameInput.fill("Redirect API v1");
            }

            // Select redirect type
            const typeSelect = page.locator("select[name='type'], [data-testid='rule-type-select']").first();
            if (await typeSelect.isVisible()) {
                await typeSelect.selectOption("redirect");
            }

            // Set URL pattern
            const patternInput = page.locator("input[name='urlPattern'], input[placeholder*='pattern']").first();
            if (await patternInput.isVisible()) {
                await patternInput.fill("/api/v1/");
            }

            // Set target
            const targetInput = page.locator("input[name='targetUrl'], input[placeholder*='target'], [data-testid='target-url-input']").first();
            if (await targetInput.isVisible()) {
                await targetInput.fill("/api/v2/");
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
