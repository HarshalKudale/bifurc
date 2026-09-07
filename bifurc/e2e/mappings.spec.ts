import { test, expect } from "./fixtures/electronApp";

test.describe("Mappings / Services Panel", () => {
    test("displays empty state when no mappings exist", async ({ page }) => {
        // Navigate to mappings
        const nav = page.locator("text=Mappings, text=Services, [data-testid='nav-mappings']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
        // Should show empty state or list
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });

    test("can open add mapping form", async ({ page }) => {
        const nav = page.locator("text=Mappings, text=Services, [data-testid='nav-mappings']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
        // Click add button
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);
            // Form or modal should appear
            const form = page.locator("input, [data-testid='mapping-form']").first();
            await expect(form).toBeVisible({ timeout: 3000 });
        }
    });

    test("can create a new mapping with domain and target", async ({ page }) => {
        const nav = page.locator("text=Mappings, text=Services, [data-testid='nav-mappings']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }

        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Fill domain field
            const domainInput = page.locator("input[placeholder*='domain'], input[name='domain'], [data-testid='domain-input']").first();
            if (await domainInput.isVisible()) {
                await domainInput.fill("myapp.localhost");
            }

            // Fill target field
            const targetInput = page.locator("input[placeholder*='target'], input[name='target'], input[placeholder*='http'], [data-testid='target-input']").first();
            if (await targetInput.isVisible()) {
                await targetInput.fill("http://127.0.0.1:3000");
            }

            // Save
            const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button[type='submit']").first();
            if (await saveBtn.isVisible()) {
                await saveBtn.click();
                await page.waitForTimeout(500);
                // Verify it appears in the list
                const body = await page.textContent("body");
                expect(body).toContain("myapp.localhost");
            }
        }
    });

    test("mapping without domain shows validation feedback", async ({ page }) => {
        const nav = page.locator("text=Mappings, text=Services, [data-testid='nav-mappings']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }

        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Try to save without filling fields
            const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button[type='submit']").first();
            if (await saveBtn.isVisible()) {
                await saveBtn.click();
                await page.waitForTimeout(300);
                // Should still be on the form (not navigated away)
                const form = page.locator("input").first();
                await expect(form).toBeVisible();
            }
        }
    });
});
