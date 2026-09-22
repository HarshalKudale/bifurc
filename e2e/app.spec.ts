import { test, expect } from "./fixtures/electronApp";
import { openPanel, openEnvironmentsManager, chooseProtocol } from "./helpers";

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

    /**
     * The manager is **not** a titlebar button: it lives inside the environment dropdown, so the
     * dropdown has to be opened first (`openEnvironmentsManager`). This test previously clicked the
     * "Manage Environments…" label directly, which matches nothing until the menu is open.
     *
     * The panel anchor is unchanged and still the real one: `EnvironmentsPanel`'s sidebar footer
     * button, `strings.environments.newEnvironment` = "New Environment".
     */
    test("can open the environments manager", async ({ page }) => {
        await openEnvironmentsManager(page);
        await expect(page.getByRole("button", { name: /New Environment/i }).first()).toBeVisible({
            timeout: 10_000,
        });
    });

    test("common tab keybinds work on tabbed panels", async ({ page }) => {
        /**
         * Every step of this test had to be re-derived from the code, because **nothing it did touched
         * the UI it claimed to**:
         *
         * 1. It began with `getByRole("button", { name: /^REST 3$/i }).nth(1)`. That label has never
         *    existed — `git log -S "REST 3" -- e2e/app.spec.ts` shows it was written here in bae3732
         *    (2026-09-14) and appears nowhere else in the tree — so it could only ever time out.
         * 2. The next attempt clicked the seeded request by name. **The tree row is not a button**:
         *    `FolderTreeItemNode.tsx:28` renders a plain `<div title={item.name} onClick={...}>`, so its
         *    role is `generic` and its accessible name comes from `title`. `getByRole("button", ...)`
         *    matched nothing. It was also the wrong idea even had it matched — a seeded row opens a
         *    *second* tab, and the last assertion is `No requests open`, which needs the panel to end
         *    with **zero** tabs.
         * 3. `Ctrl+T` was assumed to open the request form. It does not: `openNewTabInFolder`
         *    (`useRequestsPanelState.ts:94`) sets `draftProtocols[tabId] = null`, so a new tab opens on
         *    the **protocol chooser** and the form only exists once a protocol is picked. That missing
         *    `chooseProtocol` step is why the `fill` below had nothing to fill.
         *
         * What each keybind is asserted against, and why each assertion can actually fail:
         *
         *   Ctrl+T — the chooser's heading, `Select Request Protocol`, which exists nowhere else. The
         *            original asserted `getByText("New Request")`, which *also* matches the panel's own
         *            "New request" button, so it passed whether or not the keybind did anything.
         *   Ctrl+S — the save button's label. `RestTab.tsx:271` selects `Save Request` while
         *            `draftTabId` is set and `Update Request` once it is cleared, so the label flipping
         *            is precisely the evidence that a save happened.
         *   Ctrl+W — the panel's empty state, reachable only by closing the one tab this test opens.
         *
         * A request needs only a URL to be saveable: `RestTab.tsx:162-163` gates on `state.url.trim()`
         * for `tabType === "request"` — the response-body rule is the mock branch. The name is filled
         * because it is what the tree and the tab label display, not because it is required.
         */
        await openPanel(page, "Requests");

        await page.keyboard.press("ControlOrMeta+T");
        await expect(page.getByText("Select Request Protocol").first()).toBeVisible();
        await chooseProtocol(page, /REST Request/i);

        await page.getByPlaceholder("Request name (optional)").fill("Keyboard Save Test");
        await page
            .getByPlaceholder("https://example.localhost/endpoint")
            .fill("https://example.localhost/keybind-save-test");

        await page.keyboard.press("ControlOrMeta+S");
        await expect(page.getByRole("button", { name: /Update Request/i })).toBeVisible();

        await page.keyboard.press("ControlOrMeta+W");
        await expect(page.getByText("No requests open")).toBeVisible();
    });
});
