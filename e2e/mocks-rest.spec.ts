import { test, expect } from "./fixtures/electronApp";
import { openPanel, chooseProtocol, fillVisibleCodeEditor, uniqueName, pause, awaitAutosave } from "./helpers";

/**
 * Mocks panel — REST mocks.
 *
 * The earlier version of this file only asserted `expect(body).toBeTruthy()` and
 * wrapped every interaction in `if (await locator.isVisible())`, so it could not fail.
 * These tests drive the real create flow and assert on the persisted entity.
 *
 * A draft is written by the `Create Mock` button; a saved mock autosaves one second after the last
 * edit, so the update test waits that out with `awaitAutosave`. The persistence check goes through
 * `window.api.getConfig()` because the mock *name* also appears in the tab label, which renames
 * locally — a text assertion alone cannot distinguish the two.
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
            await page.getByRole("button", { name: /Create Mock/i }).click();

            await expect(page.locator("body")).toContainText(name);

            const persisted = await page.evaluate(async (n) => {
                const cfg = await window.api.getConfig();
                return (cfg.mocks ?? []).some((m) => m.name === n);
            }, name);
            expect(persisted).toBe(true);
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
            /**
             * **The response body is required, and this test used to omit it.**
             *
             * `RestTab.tsx`'s `canSaveBase` gates the create button on
             * `resMode === "none" || resBody.trim()`, and a new REST mock starts at `resMode: "json"`
             * with `resBody: ""` (`restTabReducer.ts`), so the button stays **disabled** and the click
             * times out after 30 s — which is exactly what the failure showed:
             * `locator resolved to <button disabled …>Create Mock</button>`.
             *
             * The sibling test above is named "creates a REST mock **with a body** and status" and
             * fills one; this test was written without that step. Filling the body is what makes the
             * create path reachable at all, and it is what a user must do too.
             *
             * The gate is on the **button**, not on the write: Ctrl+S routes through `handleSave`
             * directly and skips it. Filling the body is still the honest way to drive this, because
             * it is what a user does.
             */
            await fillVisibleCodeEditor(page, '{"status":"ok"}');
            await page.getByRole("button", { name: /Create Mock/i }).click();
            await expect(page.locator("body")).toContainText(name);

            // No Update button any more: a saved mock autosaves a second after the last edit.
            await page.getByPlaceholder("Mock name (optional)").fill(updated);
            await awaitAutosave(page);

            const persisted = await page.evaluate(async (n) => {
                const cfg = await window.api.getConfig();
                return (cfg.mocks ?? []).some((m) => m.name === n);
            }, updated);
            expect(persisted).toBe(true);
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
