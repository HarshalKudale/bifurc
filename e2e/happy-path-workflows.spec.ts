import { test, expect } from "./fixtures/electronApp";
import type { Page } from "@playwright/test";
import { awaitAutosave, openEnvironmentsManager, renameTab } from "./helpers";

test.setTimeout(180_000);

function uniqueName(prefix: string): string {
    return `E2E ${prefix} ${Date.now().toString(36)}`;
}

function escapeRegex(input: string): string {
    return input.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

async function pause(page: Page, ms = 400): Promise<void> {
    await page.waitForTimeout(ms);
}

async function clickSidebar(page: Page, label: string): Promise<void> {
    await page.getByRole("button", { name: new RegExp(`^${escapeRegex(label)}$`, "i") }).first().click();
    await pause(page, 700);
}

async function clickBottomSettings(page: Page): Promise<void> {
    await page.getByRole("button", { name: /^Settings$/i }).last().click();
    await pause(page, 700);
}

async function clickTitlebarButton(page: Page, label: RegExp): Promise<void> {
    await page.getByRole("button", { name: label }).first().click();
    await pause(page, 700);
}

async function clickNewTab(page: Page, title: string, fallbackText?: RegExp): Promise<void> {
    const byTitle = page.locator(`button[title="${title}"]`).first();
    if (await byTitle.isVisible().catch(() => false)) {
        await byTitle.click();
        await pause(page, 500);
        return;
    }

    if (fallbackText) {
        const byText = page.getByRole("button", { name: fallbackText }).first();
        await byText.click();
        await pause(page, 500);
        return;
    }

    throw new Error(`Unable to find new-tab button: ${title}`);
}

async function chooseProtocol(page: Page, label: RegExp): Promise<void> {
    await page.getByRole("button", { name: label }).click();
    await pause(page, 700);
}

async function fillVisibleCodeEditor(page: Page, value: string): Promise<void> {
    const editors = page.locator(".cm-content[contenteditable='true']");
    const count = await editors.count();
    for (let i = count - 1; i >= 0; i--) {
        const editor = editors.nth(i);
        if (await editor.isVisible().catch(() => false)) {
            await editor.click();
            await page.keyboard.press("ControlOrMeta+A");
            await page.keyboard.type(value);
            return;
        }
    }
    throw new Error("No visible editable code editor found");
}

/**
 * Click a **Create** control and let the write land.
 *
 * This replaced a `clickSave` that drove a Save / Update button pair no longer in the app. Create is
 * the only explicit write left — everything after it autosaves — so every call site below is a
 * create, and the updates use `awaitAutosave` instead of clicking anything.
 */
async function clickCreate(page: Page, label: RegExp): Promise<void> {
    await page.getByRole("button", { name: label }).last().click();
    await pause(page, 900);
}

/**
 * Find a Mappings row by one of its field values.
 *
 * A mapping row is an editor now, not a read-out: domain / target / label all render as `<Input>`,
 * so the row carries no text and the old `tr:has-text(...)` handle cannot find it. React writes the
 * `value` **attribute** as well as the property — the same reason the environment block below can
 * locate a renamed input by its new value.
 */
function mappingRow(page: Page, value: string) {
    return page.locator(`tr:has(input[value="${value}"])`).first();
}

async function expectRow(page: Page, text: string): Promise<void> {
    await expect(page.locator(`tr:has-text("${text}")`).first()).toBeVisible();
}

async function cleanupCreatedEntities(page: Page, names: Record<string, string>): Promise<void> {
    await page.evaluate(async (created: Record<string, string>) => {
        const cfg = await window.api.getConfig();

        for (const mapping of cfg.mappings.filter((m) => (m.label ?? "") === created.mapping || m.domain === created.mappingDomain)) {
            await window.api.deleteMapping(mapping.id);
        }

        for (const rule of (cfg.proxyRules ?? []).filter((r) => r.name === created.rule || r.name === created.ruleUpdated)) {
            await window.api.deleteRule(rule.id);
        }

        for (const env of (cfg.environments ?? []).filter((e) => e.name === created.environment || e.name === created.environmentUpdated)) {
            await window.api.deleteEnvironment(env.id);
        }

        for (const conn of (cfg.wsConnections ?? []).filter((w) => w.name === created.socket || w.name === created.socketUpdated)) {
            await window.api.deleteWsConnection(conn.id);
        }


        for (const req of (cfg.requests ?? []).filter((r) => r.name === created.restRequest || r.name === created.restRequestUpdated)) {
            await window.api.deleteRequest(req.id);
        }
        for (const req of (cfg.graphqlRequests ?? []).filter((r) => r.name === created.graphqlRequest || r.name === created.graphqlRequestUpdated)) {
            await window.api.deleteGraphQLRequest(req.id);
        }
        for (const req of (cfg.soapRequests ?? []).filter((r) => r.name === created.soapRequest || r.name === created.soapRequestUpdated)) {
            await window.api.deleteSoapRequest(req.id);
        }
        for (const req of (cfg.grpcRequests ?? []).filter((r) => r.name === created.grpcRequest || r.name === created.grpcRequestUpdated)) {
            await window.api.deleteGrpcRequest(req.id);
        }

        // REST mocks only. `plan/17` work item 8 removed the GraphQL / SOAP / gRPC mock kinds, so the
        // three loops that used to follow this one — and the `deleteGraphQLMock` / `deleteSoapMock` /
        // `deleteGrpcMock` methods they called — are gone with them.
        for (const mock of (cfg.mocks ?? []).filter((m) => m.name === created.restMock || m.name === created.restMockUpdated)) {
            await window.api.deleteMock(mock.id);
        }

        const healthServices = await window.api.healthbarGetServices(cfg.activeWorkspaceId);
        await window.api.healthbarSaveServices(
            cfg.activeWorkspaceId,
            healthServices.filter((svc) => svc.name !== created.healthService && svc.name !== created.healthServiceUpdated),
        );
    }, names);
}

test("visits all screens and runs core happy-path CRUD workflows", async ({ page }) => {
    await page.waitForLoadState("domcontentloaded");
    await pause(page, 2000);

    const created = {
        mapping: uniqueName("Mapping"),
        mappingDomain: `e2e-${Date.now().toString(36)}.localhost`,
        rule: uniqueName("Rule"),
        ruleUpdated: uniqueName("Rule Updated"),
        environment: uniqueName("Environment"),
        environmentUpdated: uniqueName("Environment Updated"),
        socket: uniqueName("Socket"),
        socketUpdated: uniqueName("Socket Updated"),
        restRequest: uniqueName("REST Request"),
        restRequestUpdated: uniqueName("REST Request Updated"),
        graphqlRequest: uniqueName("GraphQL Request"),
        graphqlRequestUpdated: uniqueName("GraphQL Request Updated"),
        soapRequest: uniqueName("SOAP Request"),
        soapRequestUpdated: uniqueName("SOAP Request Updated"),
        grpcRequest: uniqueName("gRPC Request"),
        grpcRequestUpdated: uniqueName("gRPC Request Updated"),
        restMock: uniqueName("REST Mock"),
        restMockUpdated: uniqueName("REST Mock Updated"),
        healthService: uniqueName("Health Service"),
        healthServiceUpdated: uniqueName("Health Service Updated"),
    };

    try {
        // Services
        await clickSidebar(page, "Services");
        await expect(page.locator("body")).toContainText("Processes currently listening on localhost ports");

        // Health Bar
        //
        // The card's name is an `<input>` now, not a heading, so the assertion is on the field's
        // value rather than on page text.
        await clickSidebar(page, "Health Bar");
        await page.getByRole("button", { name: /Add Service/i }).first().click();
        await page.getByPlaceholder("e.g. Auth Service").fill(created.healthService);
        await page.getByPlaceholder("http://localhost:3000/health or http://{{HOST}}/health").fill("http://localhost:3000/health");
        await clickCreate(page, /^Create$/i);
        await expect(page.locator(`input[value="${created.healthService}"]`).first()).toBeVisible();

        // Mappings — create in the modal, edit in the row.
        await clickSidebar(page, "Mappings");
        await page.getByRole("button", { name: /\+ Add Mapping/i }).first().click();
        await page.getByPlaceholder("example or client.example").fill(created.mappingDomain.replace(/\.localhost$/, ""));
        await page.getByPlaceholder("127.0.0.1:3000").fill("127.0.0.1:3010");
        await page.getByPlaceholder("My App").fill(created.mapping);
        await clickCreate(page, /^Create$/i);
        await expect(mappingRow(page, created.mapping)).toBeVisible();

        // The row is the editor: no Edit button, no Save button, just a 1 s autosave.
        await mappingRow(page, created.mapping)
            .getByLabel("Label", { exact: true })
            .fill(`${created.mapping} Updated`);
        await awaitAutosave(page);
        await expect(mappingRow(page, `${created.mapping} Updated`)).toBeVisible();

        // Proxy Rules CRUD
        await clickSidebar(page, "Proxy Rules");
        await page.getByRole("button", { name: /Add Rule/i }).first().click();
        await page.getByPlaceholder("Rule name (optional)").fill(created.rule);
        await page.getByPlaceholder("^https?://api\\.example\\.com/.*").fill("https://api.example.com/v1/orders");
        await page.getByRole("radio", { name: /External Host/i }).click();
        await page.getByPlaceholder("api.example.com:8080 or 127.0.0.1:3000").fill("127.0.0.1:3010");
        await clickCreate(page, /Create Rule/i);
        await expectRow(page, created.rule);
        await page.getByPlaceholder("Rule name (optional)").fill(created.ruleUpdated);
        await awaitAutosave(page);
        await expectRow(page, created.ruleUpdated);

        // Capture
        await clickSidebar(page, "Capture");
        await expect(page.locator("body")).toContainText("Captured requests");

        // Requests CRUD across protocols
        await clickSidebar(page, "Requests");

        await clickNewTab(page, "New request", /^New Request$/i);
        await chooseProtocol(page, /REST Request/i);
        await renameTab(page, created.restRequest);
        await page.getByPlaceholder("https://example.localhost/endpoint").fill("https://example.localhost/api/e2e");
        await clickCreate(page, /Create Request/i);
        await expect(page.locator("body")).toContainText(created.restRequest);
        await renameTab(page, created.restRequestUpdated);
        await awaitAutosave(page);
        await expect(page.locator("body")).toContainText(created.restRequestUpdated);

        await clickNewTab(page, "New request", /^New Request$/i);
        await chooseProtocol(page, /GraphQL Operation/i);
        await page.getByPlaceholder("Request name…").last().fill(created.graphqlRequest);
        await page.getByPlaceholder("https://api.example.com/graphql").fill("https://example.localhost/graphql");
        await clickCreate(page, /^Create$/i);
        await page.getByPlaceholder("Request name…").last().fill(created.graphqlRequestUpdated);
        await awaitAutosave(page);

        await clickNewTab(page, "New request", /^New Request$/i);
        await chooseProtocol(page, /SOAP Request/i);
        await page.getByPlaceholder("Untitled Request").fill(created.soapRequest);
        await page.getByPlaceholder("https://example.com/ws/service").fill("https://example.localhost/ws/orders");
        await clickCreate(page, /^Create$/i);
        await page.getByPlaceholder("Untitled Request").fill(created.soapRequestUpdated);
        await awaitAutosave(page);

        await clickNewTab(page, "New request", /^New Request$/i);
        await chooseProtocol(page, /gRPC Call/i);
        await page.getByPlaceholder("Request name…").last().fill(created.grpcRequest);
        await page.getByPlaceholder("localhost:50051").last().fill("localhost:50051");
        await page.getByPlaceholder("ServiceName").last().fill("bifurc.E2E");
        await page.getByPlaceholder("MethodName").last().fill("Ping");
        await clickCreate(page, /^Create$/i);
        await page.getByPlaceholder("ServiceName").last().fill("bifurc.E2E.Updated");
        await awaitAutosave(page);

        // Mocks CRUD — REST only.
        //
        // This block used to run four times, once per protocol. `plan/17` work item 8 made mocks
        // REST-only, and `ProtocolSelectorTab` now renders the REST card alone when `mode === "mock"`,
        // so the GraphQL / SOAP / gRPC sub-workflows had no card left to click. Their *request* halves
        // are still covered above, where the selector is unchanged.
        await clickSidebar(page, "Mocks");

        await clickNewTab(page, "New mock", /^New Mock$/i);
        await chooseProtocol(page, /REST Mock/i);
        await page.getByPlaceholder("Mock name (optional)").fill(created.restMock);
        await page.locator("input[placeholder='http://example.localhost/endpoint']").last().fill("http://example.localhost/api/mock-e2e");
        await fillVisibleCodeEditor(page, "{\"ok\":true}");
        await clickCreate(page, /Create Mock/i);
        await expect(page.locator("body")).toContainText(created.restMock);
        await page.getByPlaceholder("Mock name (optional)").fill(created.restMockUpdated);
        await awaitAutosave(page);

        // WebSocket CRUD
        await clickSidebar(page, "WebSocket");
        await clickNewTab(page, "New socket", /^New Socket$/i);
        await page.getByPlaceholder("Socket name (optional)").last().fill(created.socket);
        await page.getByPlaceholder("ws://localhost:8080 or wss://…").last().fill("ws://localhost:12345/e2e");
        await clickCreate(page, /Create Socket/i);
        await expect(page.locator("body")).toContainText(created.socket);
        await page.evaluate(async ({ name, updatedName }) => {
            const cfg = await window.api.getConfig();
            const conn = (cfg.wsConnections ?? []).find((item) => item.name === name);
            if (!conn) throw new Error(`Socket not found: ${name}`);
            await window.api.updateWsConnection({ ...conn, name: updatedName });
        }, { name: created.socket, updatedName: created.socketUpdated });


        // Workspace
        await clickTitlebarButton(page, /^Workspace$/i);
        await expect(page.locator("body")).toContainText("Configure workspace settings and remote sync");

        // Environments CRUD
        //
        // **Not `clickTitlebarButton(page, /^Manage Environments/i)`.** That label exists only inside
        // the environment dropdown, so a role-locator for it matches nothing at rest and the click times
        // out — the exact bug `openEnvironmentsManager`'s docblock in `e2e/helpers/index.ts` records,
        // and which was already fixed in `app.spec.ts`. This file kept its own copy of the wrong
        // approach, so the fix never reached it.
        //
        // "New Environment" is the **create**: `EnvironmentsPanel.handleAdd` calls `addEnvironment`
        // up front, so the environment exists the moment the button is clicked. Everything after —
        // the name, the variables — autosaves, which is why the two `Save` clicks that used to sit in
        // this block are gone.
        await openEnvironmentsManager(page);
        await page.getByRole("button", { name: /New Environment/i }).first().click();
        const envNameInput = page.locator('input[value="New Environment"]').first();
        await expect(envNameInput).toBeVisible();
        await envNameInput.fill(created.environment);
        await awaitAutosave(page);
        await expect(page.locator("body")).toContainText(created.environment);

        await page.getByRole("button", { name: /Add Variable/i }).first().click();
        const envInputs = page.locator("input[placeholder='VARIABLE_NAME']");
        await envInputs.last().fill("BASE_URL");
        await page.locator("input[placeholder='value']").last().fill("https://example.localhost");
        await page.locator(`input[value="${created.environment}"]`).first().fill(created.environmentUpdated);
        await awaitAutosave(page);

        // Assert the **persisted** environment, not the input: `environmentUpdated` contains
        // `environment` as a prefix, so a text assertion cannot tell the rename from the original.
        const envPersisted = await page.evaluate(async (n) => {
            const cfg = await window.api.getConfig();
            return (cfg.environments ?? []).some((e) => e.name === n);
        }, created.environmentUpdated);
        expect(envPersisted).toBe(true);

        // Settings
        await clickBottomSettings(page);
        await expect(page.locator("body")).toContainText("Server configuration and data management");
        await expect(page.getByRole("button", { name: /^Restart$/i }).first()).toBeVisible();
    } finally {
        await cleanupCreatedEntities(page, created);
    }
});
