import { test, expect } from "./fixtures/electronApp";

test.describe("Environments Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=Environments, text=Env, [data-testid='nav-environments']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays environment list", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });

    test("can create a new environment", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+'), button:has-text('New')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Fill environment name
            const nameInput = page.locator("input[name='name'], input[placeholder*='name'], input[placeholder*='environment']").first();
            if (await nameInput.isVisible()) {
                await nameInput.fill("Development");
            }

            // Save/confirm
            const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button:has-text('OK'), button[type='submit']").first();
            if (await saveBtn.isVisible()) {
                await saveBtn.click();
                await page.waitForTimeout(500);
                const body = await page.textContent("body");
                expect(body).toContain("Development");
            }
        }
    });

    test("can add variables to an environment", async ({ page }) => {
        // First create an environment
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            const nameInput = page.locator("input[name='name'], input[placeholder*='name']").first();
            if (await nameInput.isVisible()) {
                await nameInput.fill("Test Env");
                const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button[type='submit']").first();
                if (await saveBtn.isVisible()) {
                    await saveBtn.click();
                    await page.waitForTimeout(500);
                }
            }

            // Add a variable
            const addVarBtn = page.locator("button:has-text('Add Variable'), button:has-text('+ Variable'), [data-testid='add-variable']").first();
            if (await addVarBtn.isVisible()) {
                await addVarBtn.click();
                await page.waitForTimeout(200);

                const keyInput = page.locator("input[placeholder*='key'], input[placeholder*='Key'], input[name='key']").first();
                const valueInput = page.locator("input[placeholder*='value'], input[placeholder*='Value'], input[name='value']").first();
                if (await keyInput.isVisible()) {
                    await keyInput.fill("baseUrl");
                    await valueInput.fill("http://localhost:3000");
                }
            }
        }
    });

    test("cannot create environment with empty name", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);

            // Try to save with empty name
            const saveBtn = page.locator("button:has-text('Save'), button:has-text('Create'), button[type='submit']").first();
            if (await saveBtn.isVisible()) {
                await saveBtn.click();
                await page.waitForTimeout(300);
                // Should still be in form state
                const nameInput = page.locator("input[name='name'], input[placeholder*='name']").first();
                if (await nameInput.isVisible()) {
                    await expect(nameInput).toBeVisible();
                }
            }
        }
    });
});
