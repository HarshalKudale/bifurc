import { test, expect } from "./fixtures/electronApp";
import { openPanel, chooseProtocol, uniqueName, pause, renameTab, awaitAutosave } from "./helpers";

/**
 * REST Requests panel.
 *
 * Replaces a version whose every assertion was guarded by `if (await locator.isVisible())`
 * — it asserted `expect(body).toBeTruthy()` and nothing else, so it never caught a
 * regression.
 *
 * **Create is a button; every later change is not.** A draft has no server id, so it is written by
 * the `Create Request` button. Once it exists the editor autosaves one second after the last edit,
 * so the update test waits that out with `awaitAutosave` instead of clicking a Save button that no
 * longer exists.
 *
 * Both tests also assert the **persisted** entity via `window.api.getConfig()`. A bare
 * `toContainText(name)` cannot tell a real write from a renamed tab label, and the tab renames
 * locally the instant you press Enter — so that assertion alone passes even when nothing was saved.
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

            await renameTab(page, name);
            await page
                .getByPlaceholder("https://example.localhost/endpoint")
                .fill("https://example.localhost/api/e2e-request");
            await page.getByRole("button", { name: /Create Request/i }).click();

            // The draft gained an id, so the Create control is gone — that is the create landing.
            await expect(page.getByRole("button", { name: /Create Request/i })).toHaveCount(0);
            await expect(page.locator("body")).toContainText(name);

            const persisted = await page.evaluate(async (n) => {
                const cfg = await window.api.getConfig();
                return (cfg.requests ?? []).some((r) => r.name === n);
            }, name);
            expect(persisted).toBe(true);
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

            await renameTab(page, name);
            await page
                .getByPlaceholder("https://example.localhost/endpoint")
                .fill("https://example.localhost/api/e2e-request-update");
            await page.getByRole("button", { name: /Create Request/i }).click();
            await expect(page.locator("body")).toContainText(name);

            // There is no Update button any more: renaming a saved request autosaves a second later.
            await renameTab(page, updated);
            await awaitAutosave(page);

            const persisted = await page.evaluate(async (n) => {
                const cfg = await window.api.getConfig();
                return (cfg.requests ?? []).some((r) => r.name === n);
            }, updated);
            expect(persisted).toBe(true);
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
