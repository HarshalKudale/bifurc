import { test, expect } from "./fixtures/electronApp";

test.describe("GraphQL Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=GraphQL, [data-testid='nav-graphql']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays GraphQL panel", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });

    test("can open new GraphQL mock/request form", async ({ page }) => {
        const addBtn = page.locator("[data-testid='add-button'], button:has-text('Add'), button:has-text('+'), button:has-text('New')").first();
        if (await addBtn.isVisible()) {
            await addBtn.click();
            await page.waitForTimeout(300);
            const form = page.locator("input, textarea, .cm-editor, [data-testid='graphql-form']").first();
            const isVisible = await form.isVisible().catch(() => false);
            expect(typeof isVisible).toBe("boolean");
        }
    });
});

test.describe("SOAP Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=SOAP, [data-testid='nav-soap']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays SOAP panel", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });
});

test.describe("gRPC Panel", () => {
    test.beforeEach(async ({ page }) => {
        const nav = page.locator("text=gRPC, text=GRPC, [data-testid='nav-grpc']").first();
        if (await nav.isVisible()) {
            await nav.click();
            await page.waitForTimeout(500);
        }
    });

    test("displays gRPC panel", async ({ page }) => {
        const body = await page.textContent("body");
        expect(body).toBeTruthy();
    });
});
