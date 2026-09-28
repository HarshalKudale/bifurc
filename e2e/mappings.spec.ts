import { test, expect } from "./fixtures/electronApp";
import type { Page } from "@playwright/test";
import { openPanel, uniqueName, pause, awaitAutosave } from "./helpers";

/**
 * Mappings panel — real CRUD assertions.
 *
 * These replace an earlier version where every step was wrapped in
 * `if (await locator.isVisible())`. That pattern meant the test passed even when the
 * panel, the Add button, or the Save button did not exist at all.
 *
 * **The panel changed shape, and so did these locators.** The modal is create-only now (its confirm
 * button reads `Create`, not `Save`), and an existing mapping is edited **in its own table row** —
 * domain, target and label are `<Input>`s. That removes the Edit button *and* the `tr:has-text(...)`
 * handle: a row's domain is a field value, not text, and the only text it still carries is the
 * `.localhost` suffix. `rowWithValue` below is the replacement.
 */
test.describe("Mappings panel", () => {
    test.beforeEach(async ({ page }) => {
        await openPanel(page, "Mappings");
    });

    /** Match a row by one of its field values — rows are editors, not read-outs. */
    function rowWithValue(page: Page, value: string) {
        return page.locator(`tr:has(input[value="${value}"])`).first();
    }

    test("renders the panel with its add control and seeded mappings", async ({ page }) => {
        await expect(page.getByRole("button", { name: /\+ Add Mapping/i }).first()).toBeVisible();
        // The e2e fixture seeds api.localhost / docs.localhost / admin.localhost. The subdomain
        // column holds `api`, not `api.localhost` — the suffix is a static adornment beside it.
        await expect(rowWithValue(page, "api")).toBeVisible();
    });

    test("creates a mapping and shows it in the list", async ({ page }) => {
        const label = uniqueName("Mapping");
        const domain = `e2e-${Date.now().toString(36)}`;

        try {
            await page.getByRole("button", { name: /\+ Add Mapping/i }).first().click();
            await page.getByPlaceholder("example or client.example").fill(domain);
            await page.getByPlaceholder("127.0.0.1:3000").fill("127.0.0.1:3010");
            await page.getByPlaceholder("My App").fill(label);
            await page.getByRole("button", { name: /^Create$/i }).last().click();

            await expect(rowWithValue(page, domain)).toBeVisible();
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
            await page.getByRole("button", { name: /^Create$/i }).last().click();
            await expect(rowWithValue(page, domain)).toBeVisible();

            // No Edit button and no second modal: the row *is* the form, and it autosaves.
            await rowWithValue(page, domain).getByLabel("Label", { exact: true }).fill(updated);
            await awaitAutosave(page);

            await expect(rowWithValue(page, updated)).toBeVisible();
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
        // Leave the required domain field empty and try to create.
        await page.getByPlaceholder("My App").fill(uniqueName("Invalid"));
        await page.getByRole("button", { name: /^Create$/i }).last().click();
        await pause(page, 400);

        // The dialog must stay open (nothing created, no row added).
        await expect(page.getByPlaceholder("example or client.example")).toBeVisible();
    });
});
