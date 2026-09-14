import { test, expect } from "./fixtures/electronApp";
import { openPanel, chooseProtocol, fillVisibleCodeEditor, uniqueName, pause } from "./helpers";

/**
 * Mocks panel — REST mocks.
 *
 * The earlier version of this file only asserted `expect(body).toBeTruthy()` and
 * wrapped every interaction in `if (await locator.isVisible())`, so it could not fail.
 * These tests drive the real create/save flow and assert on the saved entity.
 */
test.describe("Mocks panel — REST", () => {
    test.beforeEach(async ({ page }) => {
        await openPanel(page, "Mocks");
    });

    test("renders the panel with its new-mock control and seeded mocks", async ({ page }) => {
        await expect(page.getByRole("button", { name: /^New Mock$/i }).first()).toBeVisible();
        // The e2e fixture seeds a "GET Users List" REST mock.
        await expect(page.locator("body")).toContainText("GET Users List");
    });

    test("creates a REST mock with a body and status", async ({ page }) => {
        const name = uniqueName("REST Mock");

        try {
            await page.getByRole("button", { name: /^New Mock$/i }).first().click();
            await pause(page, 300);
            await chooseProtocol(page, /REST Mock/i);

            await page.getByPlaceholder("Mock name (optional)").fill(name);
            await page
                .locator("input[placeholder='http://example.localhost/endpoint']")
                .last()
                .fill("http://example.localhost/api/e2e-mock");
            await fillVisibleCodeEditor(page, '{"status":"ok"}');
            await page.getByRole("button", { name: /Save Mock/i }).click();

            await expect(page.locator("body")).toContainText(name);
        } finally {
            await page.evaluate(async (n) => {
                const cfg = await window.api.getConfig();
                for (const m of (cfg.mocks ?? []).filter((x) => x.name === n)) {
                    await window.api.deleteMock(m.id);
                }
            }, name);
        }
    });

    test("updates an existing REST mock", async ({ page }) => {
        const name = uniqueName("REST Mock");
        const updated = `${name} Updated`;

        try {
            await page.getByRole("button", { name: /^New Mock$/i }).first().click();
            await pause(page, 300);
            await chooseProtocol(page, /REST Mock/i);

            await page.getByPlaceholder("Mock name (optional)").fill(name);
            await page
                .locator("input[placeholder='http://example.localhost/endpoint']")
                .last()
                .fill("http://example.localhost/api/e2e-mock-update");
            await page.getByRole("button", { name: /Save Mock/i }).click();
            await expect(page.locator("body")).toContainText(name);

            await page.getByPlaceholder("Mock name (optional)").fill(updated);
            await page.getByRole("button", { name: /Update Mock/i }).click();

            await expect(page.locator("body")).toContainText(updated);
        } finally {
            await page.evaluate(async ({ n, u }) => {
                const cfg = await window.api.getConfig();
                for (const m of (cfg.mocks ?? []).filter((x) => x.name === n || x.name === u)) {
                    await window.api.deleteMock(m.id);
                }
            }, { n: name, u: updated });
        }
    });
});
