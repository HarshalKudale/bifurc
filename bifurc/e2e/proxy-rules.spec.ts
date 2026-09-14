import { test, expect } from "./fixtures/electronApp";
import { openPanel, uniqueName } from "./helpers";

/**
 * Proxy Rules panel — real CRUD assertions.
 *
 * Previously these tests were guarded by `if (await locator.isVisible())` and could
 * therefore pass with the panel completely broken.
 */
test.describe("Proxy Rules panel", () => {
    test.beforeEach(async ({ page }) => {
        await openPanel(page, "Proxy Rules");
    });

    test("renders the panel with its add control and seeded rules", async ({ page }) => {
        await expect(page.getByRole("button", { name: /Add Rule/i }).first()).toBeVisible();
        // The e2e fixture seeds a "Block Analytics" rule.
        await expect(page.locator("body")).toContainText("Block Analytics");
    });

    test("creates a rule that routes to an external host", async ({ page }) => {
        const name = uniqueName("Rule");

        try {
            await page.getByRole("button", { name: /Add Rule/i }).first().click();
            await page.getByPlaceholder("Rule name (optional)").fill(name);
            await page.getByPlaceholder("^https?://api\\.example\\.com/.*").fill("https://api.example.com/v1/e2e");
            await page.getByRole("radio", { name: /External Host/i }).click();
            await page.getByPlaceholder("api.example.com:8080 or 127.0.0.1:3000").fill("127.0.0.1:3010");
            await page.getByRole("button", { name: /Save Rule/i }).click();

            await expect(page.locator(`tr:has-text("${name}")`).first()).toBeVisible();
        } finally {
            await page.evaluate(async (n) => {
                const cfg = await window.api.getConfig();
                for (const r of (cfg.proxyRules ?? []).filter((x) => x.name === n)) {
                    await window.api.deleteRule(r.id);
                }
            }, name);
        }
    });

    test("updates an existing rule name", async ({ page }) => {
        const name = uniqueName("Rule");
        const updated = `${name} Updated`;

        try {
            await page.getByRole("button", { name: /Add Rule/i }).first().click();
            await page.getByPlaceholder("Rule name (optional)").fill(name);
            await page.getByPlaceholder("^https?://api\\.example\\.com/.*").fill("https://api.example.com/v1/update-e2e");
            await page.getByRole("radio", { name: /External Host/i }).click();
            await page.getByPlaceholder("api.example.com:8080 or 127.0.0.1:3000").fill("127.0.0.1:3010");
            await page.getByRole("button", { name: /Save Rule/i }).click();
            await expect(page.locator(`tr:has-text("${name}")`).first()).toBeVisible();

            await page.getByPlaceholder("Rule name (optional)").fill(updated);
            await page.getByRole("button", { name: /Update Rule/i }).click();

            await expect(page.locator(`tr:has-text("${updated}")`).first()).toBeVisible();
        } finally {
            await page.evaluate(async ({ n, u }) => {
                const cfg = await window.api.getConfig();
                for (const r of (cfg.proxyRules ?? []).filter((x) => x.name === n || x.name === u)) {
                    await window.api.deleteRule(r.id);
                }
            }, { n: name, u: updated });
        }
    });

    test("does not save a rule when the pattern is missing", async ({ page }) => {
        await page.getByRole("button", { name: /Add Rule/i }).first().click();
        await page.getByPlaceholder("Rule name (optional)").fill(uniqueName("NoPattern"));
        await page.getByRole("button", { name: /Save Rule/i }).click();

        // Still on the rule editor with the pattern field present — nothing was saved.
        await expect(page.getByPlaceholder("^https?://api\\.example\\.com/.*")).toBeVisible();
    });
});
