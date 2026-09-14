import { test, expect } from "./fixtures/electronApp";
import { openPanel, uniqueName, pause } from "./helpers";

/**
 * Mappings panel — real CRUD assertions.
 *
 * These replace an earlier version where every step was wrapped in
 * `if (await locator.isVisible())`. That pattern meant the test passed even when the
 * panel, the Add button, or the Save button did not exist at all.
 */
test.describe("Mappings panel", () => {
    test.beforeEach(async ({ page }) => {
        await openPanel(page, "Mappings");
    });

    test("renders the panel with its add control and seeded mappings", async ({ page }) => {
        await expect(page.getByRole("button", { name: /\+ Add Mapping/i }).first()).toBeVisible();
        // The e2e fixture seeds api.localhost / docs.localhost / admin.localhost.
        await expect(page.locator("tr:has-text('api.localhost')").first()).toBeVisible();
    });

    test("creates a mapping and shows it in the list", async ({ page }) => {
        const label = uniqueName("Mapping");
        const domain = `e2e-${Date.now().toString(36)}`;

        try {
            await page.getByRole("button", { name: /\+ Add Mapping/i }).first().click();
            await page.getByPlaceholder("example or client.example").fill(domain);
            await page.getByPlaceholder("127.0.0.1:3000").fill("127.0.0.1:3010");
            await page.getByPlaceholder("My App").fill(label);
            await page.getByRole("button", { name: /^Save$/i }).last().click();

            await expect(page.locator(`tr:has-text("${domain}.localhost")`).first()).toBeVisible();
        } finally {
            await page.evaluate(async (d) => {
                const cfg = await window.api.getConfig();
                for (const m of cfg.mappings.filter((x) => x.domain === `${d}.localhost`)) {
                    await window.api.deleteMapping(m.id);
                }
            }, domain);
        }
    });

    test("edits an existing mapping and persists the new label", async ({ page }) => {
        const label = uniqueName("Mapping");
        const updated = `${label} Updated`;
        const domain = `e2e-${Date.now().toString(36)}`;

        try {
            await page.getByRole("button", { name: /\+ Add Mapping/i }).first().click();
            await page.getByPlaceholder("example or client.example").fill(domain);
            await page.getByPlaceholder("127.0.0.1:3000").fill("127.0.0.1:3011");
            await page.getByPlaceholder("My App").fill(label);
            await page.getByRole("button", { name: /^Save$/i }).last().click();
            await expect(page.locator(`tr:has-text("${domain}.localhost")`).first()).toBeVisible();

            const row = page.locator(`tr:has-text("${domain}.localhost")`).first();
            await row.getByRole("button", { name: /Edit/i }).click();
            await pause(page, 300);
            await page.getByPlaceholder("My App").fill(updated);
            await page.getByRole("button", { name: /^Save$/i }).last().click();

            await expect(page.locator(`tr:has-text("${updated}")`).first()).toBeVisible();
        } finally {
            await page.evaluate(async (d) => {
                const cfg = await window.api.getConfig();
                for (const m of cfg.mappings.filter((x) => x.domain === `${d}.localhost`)) {
                    await window.api.deleteMapping(m.id);
                }
            }, domain);
        }
    });

    test("does not create a mapping when the domain is missing", async ({ page }) => {
        await page.getByRole("button", { name: /\+ Add Mapping/i }).first().click();
        // Leave the required domain field empty and try to save.
        await page.getByPlaceholder("My App").fill(uniqueName("Invalid"));
        await page.getByRole("button", { name: /^Save$/i }).last().click();
        await pause(page, 400);

        // The dialog must stay open (nothing saved, no row added).
        await expect(page.getByPlaceholder("example or client.example")).toBeVisible();
    });
});
