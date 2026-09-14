import { test, expect } from "./fixtures/electronApp";
import { openPanel, chooseProtocol, uniqueName, pause } from "./helpers";

/**
 * REST Requests panel.
 *
 * Replaces a version whose every assertion was guarded by `if (await locator.isVisible())`
 * — it asserted `expect(body).toBeTruthy()` and nothing else, so it never caught a
 * regression. These tests save a request and assert the saved entity is persisted.
 */
test.describe("REST Requests panel", () => {
    test.beforeEach(async ({ page }) => {
        await openPanel(page, "Requests");
    });

    test("renders the panel with its new-request control", async ({ page }) => {
        await expect(page.getByRole("button", { name: /^New Request$/i }).first()).toBeVisible();
    });

    test("creates a REST request and shows it in the list", async ({ page }) => {
        const name = uniqueName("REST Request");

        try {
            await page.getByRole("button", { name: /^New Request$/i }).first().click();
            await pause(page, 300);
            await chooseProtocol(page, /REST Request/i);

            await page.getByPlaceholder("Request name (optional)").fill(name);
            await page
                .getByPlaceholder("https://example.localhost/endpoint")
                .fill("https://example.localhost/api/e2e-request");
            await page.getByRole("button", { name: /Save Request/i }).click();

            await expect(page.locator("body")).toContainText(name);
        } finally {
            await page.evaluate(async (n) => {
                const cfg = await window.api.getConfig();
                for (const r of (cfg.requests ?? []).filter((x) => x.name === n)) {
                    await window.api.deleteRequest(r.id);
                }
            }, name);
        }
    });

    test("updates a saved REST request", async ({ page }) => {
        const name = uniqueName("REST Request");
        const updated = `${name} Updated`;

        try {
            await page.getByRole("button", { name: /^New Request$/i }).first().click();
            await pause(page, 300);
            await chooseProtocol(page, /REST Request/i);

            await page.getByPlaceholder("Request name (optional)").fill(name);
            await page
                .getByPlaceholder("https://example.localhost/endpoint")
                .fill("https://example.localhost/api/e2e-request-update");
            await page.getByRole("button", { name: /Save Request/i }).click();
            await expect(page.locator("body")).toContainText(name);

            await page.getByPlaceholder("Request name (optional)").fill(updated);
            await page.getByRole("button", { name: /Update Request/i }).click();

            await expect(page.locator("body")).toContainText(updated);
        } finally {
            await page.evaluate(async ({ n, u }) => {
                const cfg = await window.api.getConfig();
                for (const r of (cfg.requests ?? []).filter((x) => x.name === n || x.name === u)) {
                    await window.api.deleteRequest(r.id);
                }
            }, { n: name, u: updated });
        }
    });
});
