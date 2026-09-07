import { test, expect, type Page } from "./fixtures/electronApp";

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

async function clickSave(page: Page, label: RegExp): Promise<void> {
    await page.getByRole("button", { name: label }).last().click();
    await pause(page, 900);
}

async function expectRow(page: Page, text: string): Promise<void> {
    await expect(page.locator(`tr:has-text("${text}")`).first()).toBeVisible();
}

async function cleanupCreatedEntities(page: Page, names: Record<string, string>): Promise<void> {
    await page.evaluate(async (created) => {
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

        for (const hook of (cfg.webhooks ?? []).filter((w) => w.name === created.webhook || w.name === created.webhookUpdated)) {
            await window.api.deleteWebhook(hook.id);
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

        for (const mock of (cfg.mocks ?? []).filter((m) => m.name === created.restMock || m.name === created.restMockUpdated)) {
            await window.api.deleteMock(mock.id);
        }
        for (const mock of (cfg.graphqlMocks ?? []).filter((m) => m.name === created.graphqlMock || m.name === created.graphqlMockUpdated)) {
            await window.api.deleteGraphQLMock(mock.id);
        }
        for (const mock of (cfg.soapMocks ?? []).filter((m) => m.name === created.soapMock || m.name === created.soapMockUpdated)) {
            await window.api.deleteSoapMock(mock.id);
        }
        for (const mock of (cfg.grpcMocks ?? []).filter((m) => m.name === created.grpcMock || m.name === created.grpcMockUpdated)) {
            await window.api.deleteGrpcMock(mock.id);
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
        webhook: uniqueName("Webhook"),
        webhookUpdated: uniqueName("Webhook Updated"),
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
        graphqlMock: uniqueName("GraphQL Mock"),
        graphqlMockUpdated: uniqueName("GraphQL Mock Updated"),
        soapMock: uniqueName("SOAP Mock"),
        soapMockUpdated: uniqueName("SOAP Mock Updated"),
        grpcMock: uniqueName("gRPC Mock"),
        grpcMockUpdated: uniqueName("gRPC Mock Updated"),
        healthService: uniqueName("Health Service"),
        healthServiceUpdated: uniqueName("Health Service Updated"),
    };

    try {
        // Services
        await clickSidebar(page, "Services");
        await expect(page.locator("body")).toContainText("Processes currently listening on localhost ports");

        // Health Bar
        await clickSidebar(page, "Health Bar");
        await page.getByRole("button", { name: /Add Service/i }).first().click();
        await page.getByPlaceholder("e.g. Auth Service").fill(created.healthService);
        await page.getByPlaceholder("http://localhost:3000/health or http://{{HOST}}/health").fill("http://localhost:3000/health");
        await clickSave(page, /Add Service/i);
        await expect(page.locator("body")).toContainText(created.healthService);

        // Mappings CRUD
        await clickSidebar(page, "Mappings");
        await page.getByRole("button", { name: /\+ Add Mapping/i }).first().click();
        await page.getByPlaceholder("example or client.example").fill(created.mappingDomain.replace(/\.localhost$/, ""));
        await page.getByPlaceholder("127.0.0.1:3000").fill("127.0.0.1:3010");
        await page.getByPlaceholder("My App").fill(created.mapping);
        await clickSave(page, /^Save$/i);
        await expectRow(page, created.mappingDomain);

        const mappingRow = page.locator(`tr:has-text("${created.mappingDomain}")`).first();
        await mappingRow.getByRole("button", { name: /Edit/i }).click();
        await page.getByPlaceholder("My App").fill(`${created.mapping} Updated`);
        await clickSave(page, /^Save$/i);
        await expectRow(page, `${created.mapping} Updated`);

        // Proxy Rules CRUD
        await clickSidebar(page, "Proxy Rules");
        await page.getByRole("button", { name: /Add Rule/i }).first().click();
        await page.getByPlaceholder("Rule name (optional)").fill(created.rule);
        await page.getByPlaceholder("^https?://api\\.example\\.com/.*").fill("https://api.example.com/v1/orders");
        await page.getByRole("radio", { name: /External Host/i }).click();
        await page.getByPlaceholder("api.example.com:8080 or 127.0.0.1:3000").fill("127.0.0.1:3010");
        await clickSave(page, /Save Rule/i);
        await expectRow(page, created.rule);
        await page.getByPlaceholder("Rule name (optional)").fill(created.ruleUpdated);
        await clickSave(page, /Update Rule/i);
        await expectRow(page, created.ruleUpdated);

        // Capture
        await clickSidebar(page, "Capture");
        await expect(page.locator("body")).toContainText("Captured requests");

        // Requests CRUD across protocols
        await clickSidebar(page, "Requests");

        await clickNewTab(page, "New request", /^New Request$/i);
        await chooseProtocol(page, /REST Request/i);
        await page.getByPlaceholder("Request name (optional)").fill(created.restRequest);
        await page.getByPlaceholder("https://example.localhost/endpoint").fill("https://example.localhost/api/e2e");
        await clickSave(page, /Save Request/i);
        await expect(page.locator("body")).toContainText(created.restRequest);
        await page.getByPlaceholder("Request name (optional)").fill(created.restRequestUpdated);
        await clickSave(page, /Update Request/i);
        await expect(page.locator("body")).toContainText(created.restRequestUpdated);

        await clickNewTab(page, "New request", /^New Request$/i);
        await chooseProtocol(page, /GraphQL Operation/i);
        await page.getByPlaceholder("Request name…").last().fill(created.graphqlRequest);
        await page.getByPlaceholder("https://api.example.com/graphql").fill("https://example.localhost/graphql");
        await clickSave(page, /^Save$/i);
        await page.getByPlaceholder("Request name…").last().fill(created.graphqlRequestUpdated);
        await clickSave(page, /Update/i);

        await clickNewTab(page, "New request", /^New Request$/i);
        await chooseProtocol(page, /SOAP Request/i);
        await page.getByPlaceholder("Untitled Request").fill(created.soapRequest);
        await page.getByPlaceholder("https://example.com/ws/service").fill("https://example.localhost/ws/orders");
        await clickSave(page, /^Save$/i);
        await page.getByPlaceholder("Untitled Request").fill(created.soapRequestUpdated);
        await clickSave(page, /Update/i);

        await clickNewTab(page, "New request", /^New Request$/i);
        await chooseProtocol(page, /gRPC Call/i);
        await page.getByPlaceholder("Request name…").last().fill(created.grpcRequest);
        await page.getByPlaceholder("localhost:50051").last().fill("localhost:50051");
        await page.getByPlaceholder("ServiceName").last().fill("localpanel.E2E");
        await page.getByPlaceholder("MethodName").last().fill("Ping");
        await clickSave(page, /^Save$/i);
        await page.getByPlaceholder("ServiceName").last().fill("localpanel.E2E.Updated");
        await clickSave(page, /Update/i);

        // Mocks CRUD across protocols
        await clickSidebar(page, "Mocks");

        await clickNewTab(page, "New mock", /^New Mock$/i);
        await chooseProtocol(page, /REST Mock/i);
        await page.getByPlaceholder("Mock name (optional)").fill(created.restMock);
        await page.locator("input[placeholder='http://example.localhost/endpoint']").last().fill("http://example.localhost/api/mock-e2e");
        await fillVisibleCodeEditor(page, "{\"ok\":true}");
        await clickSave(page, /Save Mock/i);
        await expect(page.locator("body")).toContainText(created.restMock);
        await page.getByPlaceholder("Mock name (optional)").fill(created.restMockUpdated);
        await clickSave(page, /Update Mock/i);

        await clickNewTab(page, "New mock", /^New Mock$/i);
        await chooseProtocol(page, /GraphQL Mock/i);
        await page.getByPlaceholder("Mock name…").last().fill(created.graphqlMock);
        await page.getByPlaceholder("/graphql or regex pattern…").fill("/graphql");
        await page.getByPlaceholder("Match operation name (leave empty to match all)").fill("GetDashboard");
        await clickSave(page, /^Save$/i);
        await page.getByPlaceholder("Mock name…").last().fill(created.graphqlMockUpdated);
        await clickSave(page, /Update/i);

        await clickNewTab(page, "New mock", /^New Mock$/i);
        await chooseProtocol(page, /SOAP Mock/i);
        await page.getByPlaceholder("Untitled Mock").fill(created.soapMock);
        await page.getByPlaceholder("/ws/service").fill("/soap/orders");
        await page.getByPlaceholder("*").fill("CreateOrder");
        await clickSave(page, /^Save$/i);
        await page.getByPlaceholder("Untitled Mock").fill(created.soapMockUpdated);
        await clickSave(page, /Update/i);

        await clickNewTab(page, "New mock", /^New Mock$/i);
        await chooseProtocol(page, /gRPC Mock/i);
        await page.getByPlaceholder("Mock name…").last().fill(created.grpcMock);
        await page.getByPlaceholder("ServiceName").last().fill("localpanel.E2E");
        await page.getByPlaceholder("MethodName").last().fill("MockPing");
        await clickSave(page, /^Save$/i);
        await page.getByPlaceholder("ServiceName").last().fill("localpanel.E2E.Updated");
        await clickSave(page, /Update/i);

        // WebSocket CRUD
        await clickSidebar(page, "WebSocket");
        await clickNewTab(page, "New socket", /^New Socket$/i);
        await page.getByPlaceholder("Socket name (optional)").last().fill(created.socket);
        await page.getByPlaceholder("ws://localhost:8080 or wss://…").last().fill("ws://localhost:12345/e2e");
        await clickSave(page, /Save Socket/i);
        await expect(page.locator("body")).toContainText(created.socket);
        await page.evaluate(async ({ name, updatedName }) => {
            const cfg = await window.api.getConfig();
            const conn = (cfg.wsConnections ?? []).find((item) => item.name === name);
            if (!conn) throw new Error(`Socket not found: ${name}`);
            await window.api.updateWsConnection({ ...conn, name: updatedName });
        }, { name: created.socket, updatedName: created.socketUpdated });

        // Webhooks CRUD
        await clickSidebar(page, "Webhooks");
        await clickNewTab(page, "New webhook", /^New Webhook$/i);
        await page.getByPlaceholder("Webhook name (optional)").last().fill(created.webhook);
        await page.getByPlaceholder("your-webhook-path").last().fill(`e2e-${Date.now().toString(36)}`);
        await clickSave(page, /Save Webhook/i);
        await expect(page.locator("body")).toContainText(created.webhook);
        await page.evaluate(async ({ name, updatedName }) => {
            const cfg = await window.api.getConfig();
            const hook = (cfg.webhooks ?? []).find((item) => item.name === name);
            if (!hook) throw new Error(`Webhook not found: ${name}`);
            await window.api.updateWebhook({ ...hook, name: updatedName });
        }, { name: created.webhook, updatedName: created.webhookUpdated });

        // Workspace
        await clickTitlebarButton(page, /^Workspace$/i);
        await expect(page.locator("body")).toContainText("Configure workspace settings and remote sync");

        // Environments CRUD
        await clickTitlebarButton(page, /^Manage Environments/i);
        await page.getByRole("button", { name: /New Environment/i }).first().click();
        const envNameInput = page.locator('input[value="New Environment"]').first();
        await expect(envNameInput).toBeVisible();
        await envNameInput.fill(created.environment);
        await page.getByRole("button", { name: /Save/i }).first().click();
        await expect(page.locator("body")).toContainText(created.environment);
        await page.getByRole("button", { name: /Add Variable/i }).first().click();
        const envInputs = page.locator("input[placeholder='VARIABLE_NAME']");
        await envInputs.last().fill("BASE_URL");
        await page.locator("input[placeholder='value']").last().fill("https://example.localhost");
        await page.locator(`input[value="${created.environment}"]`).first().fill(created.environmentUpdated);
        await page.getByRole("button", { name: /Save/i }).first().click();
        await expect(page.locator("body")).toContainText(created.environmentUpdated);

        // Settings
        await clickBottomSettings(page);
        await expect(page.locator("body")).toContainText("Server configuration and data management");
        await expect(page.getByRole("button", { name: /^Restart$/i }).first()).toBeVisible();
    } finally {
        await cleanupCreatedEntities(page, created);
    }
});
