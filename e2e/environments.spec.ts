import { test, expect } from "./fixtures/electronApp";
import { openEnvironmentsManager } from "./helpers";

/**
 * The Environments manager is reached through the titlebar's environment **dropdown**
 * (`openEnvironmentsManager`).
 *
 * There is no sidebar entry for it, and the reason is easy to get wrong: `environments` **is** in
 * `PANEL_REGISTRY` (`renderer/lib/panelRegistry.tsx:160`) but carries `showInSidebar: false`, and
 * `enabledPanels` (`:207`) filters exactly those out. No `NavItem` is rendered, so no
 * `nav-environments` testid exists in the DOM. (`settings`, `workspace` and `audit` are the same.)
 *
 * This spec used to wrap every step in `if (await locator.isVisible())`, which is the anti-pattern
 * `e2e/helpers/index.ts`'s header calls out: a renamed or missing control produced a *passing* test.
 * Its `beforeEach` looked for that never-rendered nav item, so it silently skipped navigation and
 * all four tests passed without the panel ever being open.
 *
 * Every test below now asserts on something only reachable through the panel.
 *
 * ## Two locator traps this panel has
 *
 * 1. **The active environment's name is ambiguous.** `EnvSelector`'s titlebar toggle takes its
 *    accessible name from its *text content*, which is the active environment's name — so
 *    `getByRole("button", { name: /^Development$/ })` matches the titlebar toggle **and** the
 *    panel's sidebar item. The toggle precedes the panel in DOM order, so `.last()` is the sidebar
 *    item. (The dropdown's own menu entries also carry env names, but `openEnvironmentsManager`
 *    closes the menu again, so there are exactly two matches.)
 * 2. **The panel opens on its empty state.** `selectedEnvId` defaults to `GLOBAL_ENV_ID`
 *    (`__global__`), which `writeSampleWorkspace` does not define, so no environment is selected
 *    until one is clicked — `EnvVariableTable` is not rendered before then.
 */
test.describe("Environments Panel", () => {
    test.beforeEach(async ({ page }) => {
        await openEnvironmentsManager(page);
    });

    test("displays environment list", async ({ page }) => {
        await page.getByRole("button", { name: /^Development$/ }).last().click();

        // The seeded environment's own data is what proves the list resolved to the right entry:
        // `writeSampleWorkspace` seeds `env-dev` as "Development" with baseUrl/apiKey/timeout.
        await expect(page.getByPlaceholder("Environment name").first()).toHaveValue("Development");
        await expect(page.getByPlaceholder("VARIABLE_NAME").first()).toHaveValue("baseUrl");
    });

    test("can create a new environment", async ({ page }) => {
        const newEnv = page.getByRole("button", { name: /^New Environment$/ });

        // One match at rest: the sidebar-footer button. (`EnvironmentsPanel.handleAdd` asks the
        // engine for an environment *named* "New Environment", so the created entry deliberately
        // shares the button's label — which is what makes the count assertion below meaningful.)
        await expect(newEnv).toHaveCount(1);

        await newEnv.first().click();

        // Two now: the footer button plus the new entry in the sidebar list.
        await expect(newEnv).toHaveCount(2);
    });

    test("can add variables to an environment", async ({ page }) => {
        await page.getByRole("button", { name: /^Development$/ }).last().click();

        const addVar = page.getByRole("button", { name: /Add Variable/i }).first();
        await expect(addVar).toBeVisible();
        await addVar.click();

        // The seeded environment has exactly three variables, so a fourth row is the new one.
        await expect(page.getByPlaceholder("VARIABLE_NAME")).toHaveCount(4);
    });

    test("cannot create environment with empty name", async ({ page }) => {
        // There is no empty-name path to begin with: `EnvironmentsPanel.handleAdd` asks the engine
        // for a pre-named environment, and `EnvVariableTable.handleSave` falls back to the existing
        // name when the field is cleared (`name.trim() || env.name`). This asserts that fallback.
        await page.getByRole("button", { name: /^Development$/ }).last().click();

        const nameInput = page.getByPlaceholder("Environment name").first();
        await expect(nameInput).toHaveValue("Development");

        await nameInput.fill("");

        // The Save button only renders once the form is dirty.
        await page.getByRole("button", { name: /^Save$/ }).first().click();

        // The titlebar toggle re-reads the active environment from the reloaded config, so an
        // empty name reaching disk would show up here as a blank toggle.
        await expect(page.getByTitle("Switch environment")).toContainText("Development");
    });
});
