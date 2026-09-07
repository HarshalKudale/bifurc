/**
 * Sample workspace data for E2E tests and screenshots.
 * Provides a complete, realistic workspace that's copied to the temp directory
 * before launching the Electron app, so tests run against a populated state.
 */

import * as fs from "fs";
import * as path from "path";

// ── Constants ──────────────────────────────────────────────────────────────

export const WORKSPACE_ID = "default";
export const WORKSPACE_NAME = "Sample Workspace";

// ── Real dev profile theme lookup ─────────────────────────────────────────

/**
 * Reads the currently selected theme from the developer's real (non-isolated)
 * app.json, so e2e/screenshot runs reflect whatever theme is actually set in
 * the app, rather than always resetting to the built-in default.
 */
export function readRealThemeId(): string | null {
    try {
        const localAppData = process.env.LOCALAPPDATA;
        if (!localAppData) return null;
        const realAppJson = path.join(localAppData, "Bifurc", "app.json");
        const raw = fs.readFileSync(realAppJson, "utf-8");
        const parsed = JSON.parse(raw) as { themeId?: string | null };
        return parsed.themeId ?? null;
    } catch {
        return null;
    }
}

// ── Folders ────────────────────────────────────────────────────────────────

const mockFolderApi = {
    id: "folder-mock-api",
    name: "API Mocks",
    parentId: null,
    createdAt: 1704067200000,
    workspaceId: WORKSPACE_ID,
};

const requestFolderTests = {
    id: "folder-req-tests",
    name: "Test Requests",
    parentId: null,
    createdAt: 1704067200000,
    workspaceId: WORKSPACE_ID,
};

// ── Environments ───────────────────────────────────────────────────────────

const envDev = {
    id: "env-dev",
    name: "Development",
    variables: [
        { id: "var-1", key: "baseUrl", value: "http://localhost:3000" },
        { id: "var-2", key: "apiKey", value: "dev-key-12345" },
        { id: "var-3", key: "timeout", value: "5000" },
    ],
    createdAt: 1704067200000,
    workspaceId: WORKSPACE_ID,
};

const envProd = {
    id: "env-prod",
    name: "Production",
    variables: [
        { id: "var-4", key: "baseUrl", value: "https://api.example.com" },
        { id: "var-5", key: "apiKey", value: "prod-key-67890" },
        { id: "var-6", key: "timeout", value: "10000" },
    ],
    createdAt: 1704067260000,
    workspaceId: WORKSPACE_ID,
};

// ── Mappings ───────────────────────────────────────────────────────────────

const mappings = [
    {
        id: "map-001",
        domain: "api.localhost",
        target: "http://127.0.0.1:3000",
        enabled: true,
        label: "Backend API",
        workspaceId: WORKSPACE_ID,
        createdAt: 1704067200000,
    },
    {
        id: "map-002",
        domain: "admin.localhost",
        target: "http://127.0.0.1:4000",
        enabled: false,
        label: "Admin Dashboard",
        workspaceId: WORKSPACE_ID,
        createdAt: 1704067260000,
    },
    {
        id: "map-003",
        domain: "docs.localhost",
        target: "http://127.0.0.1:5000",
        enabled: true,
        label: "",
        workspaceId: WORKSPACE_ID,
        createdAt: 1704067320000,
    },
];

// ── Proxy Rules ────────────────────────────────────────────────────────────

const proxyRules = [
    {
        id: "rule-001",
        name: "Block Analytics",
        pattern: ".*\\/analytics\\/.*",
        useRegex: true,
        targetType: "external",
        targetMappingId: "",
        targetExternal: "",
        requestScript: "// Block request\nreturn { block: true };",
        responseScript: "",
        enabled: true,
        createdAt: 1704067200000,
        folderId: null,
        workspaceId: WORKSPACE_ID,
    },
    {
        id: "rule-002",
        name: "Redirect Legacy API",
        pattern: "/v1/",
        useRegex: false,
        targetType: "external",
        targetMappingId: "",
        targetExternal: "http://127.0.0.1:3000/v2/",
        requestScript: "",
        responseScript: "",
        enabled: false,
        createdAt: 1704067260000,
        folderId: null,
        workspaceId: WORKSPACE_ID,
    },
];

// ── REST Mocks ─────────────────────────────────────────────────────────────

const mocks = [
    {
        id: "mock-001",
        name: "GET Users List",
        method: "GET",
        urlPattern: "/api/users",
        useRegex: false,
        enabled: true,
        capturedHeaders: {},
        capturedBody: "",
        responseStatus: 200,
        responseHeaders: { "content-type": "application/json" },
        responseBody: JSON.stringify([
            { id: 1, name: "Alice Johnson", email: "alice@example.com", role: "admin" },
            { id: 2, name: "Bob Smith", email: "bob@example.com", role: "user" },
            { id: 3, name: "Carol White", email: "carol@example.com", role: "user" },
        ], null, 2),
        responseBodyEncoding: "utf8",
        responseDelay: 0,
        streamingMode: "none",
        createdAt: 1704067200000,
        folderId: mockFolderApi.id,
        workspaceId: WORKSPACE_ID,
    },
    {
        id: "mock-002",
        name: "POST User Error (422)",
        method: "POST",
        urlPattern: "/api/users",
        useRegex: false,
        enabled: false,
        capturedHeaders: {},
        capturedBody: "",
        responseStatus: 422,
        responseHeaders: { "content-type": "application/json" },
        responseBody: JSON.stringify({ error: "Validation failed", fields: { email: "Invalid email format" } }),
        responseBodyEncoding: "utf8",
        responseDelay: 500,
        streamingMode: "none",
        createdAt: 1704067260000,
        folderId: mockFolderApi.id,
        workspaceId: WORKSPACE_ID,
    },
    {
        id: "mock-003",
        name: "GET User by ID",
        method: "GET",
        urlPattern: "/api/users/\\d+",
        useRegex: true,
        enabled: true,
        capturedHeaders: {},
        capturedBody: "",
        responseStatus: 200,
        responseHeaders: { "content-type": "application/json" },
        responseBody: JSON.stringify({ id: 1, name: "Alice Johnson", email: "alice@example.com", role: "admin" }),
        responseBodyEncoding: "utf8",
        responseDelay: 100,
        streamingMode: "none",
        createdAt: 1704067320000,
        folderId: null,
        workspaceId: WORKSPACE_ID,
    },
];

// ── REST Requests ──────────────────────────────────────────────────────────

const requests = [
    {
        id: "req-001",
        name: "Get All Users",
        method: "GET",
        url: "{{baseUrl}}/api/users",
        headers: { "Accept": "application/json", "Authorization": "Bearer {{apiKey}}" },
        body: "",
        preScript: "",
        postScript: "",
        testScript: "",
        createdAt: 1704067200000,
        folderId: requestFolderTests.id,
        workspaceId: WORKSPACE_ID,
    },
    {
        id: "req-002",
        name: "Create New User",
        method: "POST",
        url: "{{baseUrl}}/api/users",
        headers: { "Content-Type": "application/json", "Authorization": "Bearer {{apiKey}}" },
        body: JSON.stringify({ name: "New User", email: "newuser@example.com", role: "user" }, null, 2),
        preScript: "",
        postScript: "",
        testScript: "",
        createdAt: 1704067260000,
        folderId: requestFolderTests.id,
        workspaceId: WORKSPACE_ID,
    },
    {
        id: "req-003",
        name: "Update User",
        method: "PUT",
        url: "{{baseUrl}}/api/users/1",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: "Alice Updated" }),
        preScript: "",
        postScript: "",
        testScript: "",
        createdAt: 1704067320000,
        folderId: null,
        workspaceId: WORKSPACE_ID,
    },
];

// ── WebSockets ─────────────────────────────────────────────────────────────

const wsConnections = [
    {
        id: "ws-001",
        name: "Live Chat Socket",
        url: "ws://localhost:3000/chat",
        headers: { "Authorization": "Bearer {{apiKey}}" },
        createdAt: 1704067200000,
        folderId: null,
        workspaceId: WORKSPACE_ID,
    },
    {
        id: "ws-002",
        name: "Notifications Stream",
        url: "wss://api.example.com/notifications",
        headers: {},
        createdAt: 1704067260000,
        folderId: null,
        workspaceId: WORKSPACE_ID,
    },
];

// ── Webhooks ───────────────────────────────────────────────────────────────

const webhooks = [
    {
        id: "wh-001",
        name: "Order Created",
        urlSuffix: "orders/created",
        createdAt: 1704067200000,
        folderId: null,
        workspaceId: WORKSPACE_ID,
    },
    {
        id: "wh-002",
        name: "Payment Received",
        urlSuffix: "payments",
        createdAt: 1704067260000,
        folderId: null,
        workspaceId: WORKSPACE_ID,
    },
];

// ── Export all data ────────────────────────────────────────────────────────

export const SAMPLE_DATA = {
    workspaceId: WORKSPACE_ID,
    workspaceName: WORKSPACE_NAME,
    folders: {
        mockFolders: [mockFolderApi],
        requestFolders: [requestFolderTests],
        ruleFolders: [],
        wsFolders: [],
        webhookFolders: [],
    },
    environments: [envDev, envProd],
    mappings,
    proxyRules,
    mocks,
    requests,
    wsConnections,
    webhooks,
};

// ── Write sample data to filesystem ────────────────────────────────────────

/**
 * Writes the complete sample workspace to the specified data directory.
 * Creates the folder structure, entity files, and workspace metadata.
 *
 * @param themeId The UI theme id to seed the isolated test profile with. Pass the
 * real dev profile's current theme (see `readRealThemeId`) so e2e runs render
 * with whatever theme is actually selected, instead of silently resetting to
 * the built-in default.
 */
export function writeSampleWorkspace(dataRoot: string, themeId: string | null = null): void {
    const wsDir = path.join(dataRoot, WORKSPACE_ID);

    // Create all entity directories
    const dirs = [
        wsDir,
        path.join(wsDir, "mappings"),
        path.join(wsDir, "rules"),
        path.join(wsDir, "mocks"),
        path.join(wsDir, "mocks", "API Mocks"),  // folder directory
        path.join(wsDir, "requests"),
        path.join(wsDir, "requests", "Test Requests"),  // folder directory
        path.join(wsDir, "environments"),
        path.join(wsDir, "sockets"),
        path.join(wsDir, "webhooks"),
        path.join(wsDir, "capture"),
    ];
    for (const dir of dirs) {
        fs.mkdirSync(dir, { recursive: true });
    }

    // Write app.json (settings) in parent directory
    const appSettings = {
        port: 80,
        webhookPort: 9101,
        companionPort: 9271,
        minimizeToTray: true,
        tlsEnabled: false,
        tlsCaCertPath: null,
        tlsCaKeyPath: null,
        workspaces: [{ id: WORKSPACE_ID, name: WORKSPACE_NAME, activeEnvironmentId: envDev.id }],
        activeWorkspaceId: WORKSPACE_ID,
        hasSeenWelcome: true,
        themeId,
    };
    fs.writeFileSync(
        path.join(path.dirname(dataRoot), "app.json"),
        JSON.stringify(appSettings, null, 2),
        "utf-8"
    );

    // Write workspace.json
    const workspaceFile = {
        id: WORKSPACE_ID,
        name: WORKSPACE_NAME,
        createdAt: 1704067200000,
        activeEnvironmentId: envDev.id,
    };
    fs.writeFileSync(
        path.join(wsDir, "workspace.json"),
        JSON.stringify(workspaceFile, null, 2),
        "utf-8"
    );

    // Write .gitignore
    fs.writeFileSync(
        path.join(wsDir, ".gitignore"),
        "capture/\n*.tmp\n",
        "utf-8"
    );

    // Write mappings
    for (const mapping of mappings) {
        fs.writeFileSync(
            path.join(wsDir, "mappings", `${mapping.id}.json`),
            JSON.stringify(mapping, null, 2),
            "utf-8"
        );
    }

    // Write mappings/enabled.json
    const enabledMappings = mappings.filter(m => m.enabled).map(m => m.id);
    fs.writeFileSync(
        path.join(wsDir, "mappings", "enabled.json"),
        JSON.stringify(enabledMappings, null, 2),
        "utf-8"
    );

    // Write proxy rules
    for (const rule of proxyRules) {
        fs.writeFileSync(
            path.join(wsDir, "rules", `${rule.id}.json`),
            JSON.stringify(rule, null, 2),
            "utf-8"
        );
    }

    // Write rules/enabled.json
    const enabledRules = proxyRules.filter(r => r.enabled).map(r => r.id);
    fs.writeFileSync(
        path.join(wsDir, "rules", "enabled.json"),
        JSON.stringify(enabledRules, null, 2),
        "utf-8"
    );

    // Write rules/names.json
    const ruleNames: Record<string, string> = {};
    for (const rule of proxyRules) {
        ruleNames[rule.id] = rule.name;
    }
    fs.writeFileSync(
        path.join(wsDir, "rules", "names.json"),
        JSON.stringify(ruleNames, null, 2),
        "utf-8"
    );

    // Write rules/index.json
    const ruleIndex = {
        folders: [],
        order: proxyRules.map(r => r.id),
    };
    fs.writeFileSync(
        path.join(wsDir, "rules", "index.json"),
        JSON.stringify(ruleIndex, null, 2),
        "utf-8"
    );

    // Write mocks
    for (const mock of mocks) {
        const mockPath = mock.folderId
            ? path.join(wsDir, "mocks", "API Mocks", `${mock.id}.json`)
            : path.join(wsDir, "mocks", `${mock.id}.json`);
        fs.writeFileSync(mockPath, JSON.stringify(mock, null, 2), "utf-8");
    }

    // Write mocks/enabled.json
    const enabledMocks = mocks.filter(m => m.enabled).map(m => m.id);
    fs.writeFileSync(
        path.join(wsDir, "mocks", "enabled.json"),
        JSON.stringify(enabledMocks, null, 2),
        "utf-8"
    );

    // Write mocks/index.json
    const mockIndex = {
        folders: [mockFolderApi],
        order: mocks.map(m => m.id),
    };
    fs.writeFileSync(
        path.join(wsDir, "mocks", "index.json"),
        JSON.stringify(mockIndex, null, 2),
        "utf-8"
    );

    // Write requests
    for (const request of requests) {
        const reqPath = request.folderId
            ? path.join(wsDir, "requests", "Test Requests", `${request.id}.json`)
            : path.join(wsDir, "requests", `${request.id}.json`);
        fs.writeFileSync(reqPath, JSON.stringify(request, null, 2), "utf-8");
    }

    // Write requests/names.json
    const requestNames: Record<string, string> = {};
    for (const req of requests) {
        requestNames[req.id] = req.name;
    }
    fs.writeFileSync(
        path.join(wsDir, "requests", "names.json"),
        JSON.stringify(requestNames, null, 2),
        "utf-8"
    );

    // Write requests/index.json
    const requestIndex = {
        folders: [requestFolderTests],
        order: requests.map(r => r.id),
    };
    fs.writeFileSync(
        path.join(wsDir, "requests", "index.json"),
        JSON.stringify(requestIndex, null, 2),
        "utf-8"
    );

    // Write environments
    for (const env of SAMPLE_DATA.environments) {
        fs.writeFileSync(
            path.join(wsDir, "environments", `${env.id}.json`),
            JSON.stringify(env, null, 2),
            "utf-8"
        );
    }

    // Write WebSocket connections
    for (const ws of wsConnections) {
        fs.writeFileSync(
            path.join(wsDir, "sockets", `${ws.id}.json`),
            JSON.stringify(ws, null, 2),
            "utf-8"
        );
    }

    // Write sockets/names.json
    const wsNames: Record<string, string> = {};
    for (const ws of wsConnections) {
        wsNames[ws.id] = ws.name;
    }
    fs.writeFileSync(
        path.join(wsDir, "sockets", "names.json"),
        JSON.stringify(wsNames, null, 2),
        "utf-8"
    );

    // Write sockets/index.json
    const wsIndex = {
        folders: [],
        order: wsConnections.map(w => w.id),
    };
    fs.writeFileSync(
        path.join(wsDir, "sockets", "index.json"),
        JSON.stringify(wsIndex, null, 2),
        "utf-8"
    );

    // Write webhooks
    for (const wh of webhooks) {
        fs.writeFileSync(
            path.join(wsDir, "webhooks", `${wh.id}.json`),
            JSON.stringify(wh, null, 2),
            "utf-8"
        );
    }

    // Write webhooks/names.json
    const webhookNames: Record<string, string> = {};
    for (const wh of webhooks) {
        webhookNames[wh.id] = wh.name;
    }
    fs.writeFileSync(
        path.join(wsDir, "webhooks", "names.json"),
        JSON.stringify(webhookNames, null, 2),
        "utf-8"
    );

    // Write webhooks/index.json
    const webhookIndex = {
        folders: [],
        order: webhooks.map(w => w.id),
    };
    fs.writeFileSync(
        path.join(wsDir, "webhooks", "index.json"),
        JSON.stringify(webhookIndex, null, 2),
        "utf-8"
    );
}
