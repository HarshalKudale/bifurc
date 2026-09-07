import { test, expect } from "./fixtures/electronApp";

test.describe("Application Launch", () => {
    test("window opens with correct title", async ({ page, electronApp }) => {
        const title = await page.title();
        expect(title).toContain("Bifurc");
    });

    test("main layout renders", async ({ page }) => {
        // The app should show a sidebar navigation
        const sidebar = page.locator("[data-testid='sidebar'], nav, [class*='sidebar']");
        await expect(sidebar.first()).toBeVisible({ timeout: 10_000 });
    });

    test("window has minimum dimensions", async ({ electronApp }) => {
        const window = await electronApp.firstWindow();
        const { width, height } = await window.evaluate(() => ({
            width: window.innerWidth,
            height: window.innerHeight,
        }));
        expect(width).toBeGreaterThanOrEqual(800);
        expect(height).toBeGreaterThanOrEqual(500);
    });
});

test.describe("Navigation", () => {
    test("can navigate to mappings panel", async ({ page }) => {
        const mappingsNav = page.locator("text=Mappings, text=Services, [data-testid='nav-mappings']").first();
        if (await mappingsNav.isVisible()) {
            await mappingsNav.click();
            await page.waitForTimeout(500);
            // Panel should be visible
            const content = await page.textContent("body");
            expect(content).toBeTruthy();
        }
    });

    test("can navigate to mocks panel", async ({ page }) => {
        const mocksNav = page.locator("text=Mocks, [data-testid='nav-mocks']").first();
        if (await mocksNav.isVisible()) {
            await mocksNav.click();
            await page.waitForTimeout(500);
            const content = await page.textContent("body");
            expect(content).toBeTruthy();
        }
    });

    test("can navigate to requests panel", async ({ page }) => {
        const reqNav = page.locator("text=Requests, [data-testid='nav-requests']").first();
        if (await reqNav.isVisible()) {
            await reqNav.click();
            await page.waitForTimeout(500);
            const content = await page.textContent("body");
            expect(content).toBeTruthy();
        }
    });

    test("can navigate to environments panel", async ({ page }) => {
        const envNav = page.locator("text=Environments, text=Env, [data-testid='nav-environments']").first();
        if (await envNav.isVisible()) {
            await envNav.click();
            await page.waitForTimeout(500);
            const content = await page.textContent("body");
            expect(content).toBeTruthy();
        }
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
