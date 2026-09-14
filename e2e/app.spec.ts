import { test, expect } from "./fixtures/electronApp";
import { openPanel } from "./helpers";

test.describe("Application Launch", () => {
    test("window opens with correct title", async ({ page }) => {
        const title = await page.title();
        expect(title).toContain("Bifurc");
    });

    test("main layout renders", async ({ page }) => {
        // The app should show a sidebar navigation
        const sidebar = page.locator("[data-testid='sidebar'], nav, [class*='sidebar']");
        await expect(sidebar.first()).toBeVisible({ timeout: 10_000 });
    });

    test("window has minimum dimensions", async ({ electronApp }) => {
        const win = await electronApp.firstWindow();
        const { width, height } = await win.evaluate(() => ({
            width: window.innerWidth,
            height: window.innerHeight,
        }));
        expect(width).toBeGreaterThanOrEqual(800);
        expect(height).toBeGreaterThanOrEqual(500);
    });
});

test.describe("Navigation", () => {
    /**
     * These tests used to be wrapped in `if (await nav.isVisible())`, so a missing or
     * renamed panel produced a passing test. Each one now asserts on a control that
     * only exists inside the target panel.
     */
    test("can navigate to mappings panel", async ({ page }) => {
        await openPanel(page, "Mappings");
        await expect(page.getByRole("button", { name: /\+ Add Mapping/i }).first()).toBeVisible();
    });

    test("can navigate to mocks panel", async ({ page }) => {
        await openPanel(page, "Mocks");
        await expect(page.getByRole("button", { name: /^New Mock$/i }).first()).toBeVisible();
    });

    test("can navigate to requests panel", async ({ page }) => {
        await openPanel(page, "Requests");
        await expect(page.getByRole("button", { name: /^New Request$/i }).first()).toBeVisible();
    });

    test("can navigate to proxy rules panel", async ({ page }) => {
        await openPanel(page, "Proxy Rules");
        await expect(page.getByRole("button", { name: /Add Rule/i }).first()).toBeVisible();
    });

    test("can open the environments manager", async ({ page }) => {
        await page.getByRole("button", { name: /^Manage Environments/i }).first().click();
        await expect(page.getByRole("button", { name: /New Environment/i }).first()).toBeVisible({
            timeout: 10_000,
        });
    });

    test("common tab keybinds work on tabbed panels", async ({ page }) => {
        await page.getByRole("button", { name: /^REST 3$/i }).nth(1).click();
        await page.waitForTimeout(400);

        await page.keyboard.press("ControlOrMeta+T");
        await expect(page.getByText("New Request").first()).toBeVisible();

        await page.getByRole("textbox", { name: /Request name \(optional\)/i }).fill("Keyboard Save Test");
        await page.getByPlaceholder("https://example.localhost/endpoint").fill("https://example.localhost/keybind-save-test");
        await page.keyboard.press("ControlOrMeta+S");
        await expect(page.getByRole("button", { name: /Update Request/i })).toBeVisible();

        await page.keyboard.press("ControlOrMeta+W");
        await expect(page.getByText("No requests open")).toBeVisible();
    });
});
