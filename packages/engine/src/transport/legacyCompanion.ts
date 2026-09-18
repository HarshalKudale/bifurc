/**
 * Companion WebSocket server — **v1, deprecated, frozen.** The browser extension's bridge into
 * the app, kept alive for a deprecation window so the released extension keeps working.
 *
 * Provides a localhost-only WebSocket bridge that allows the companion browser extension to
 * execute IPC-equivalent commands (e.g. mock:add, request:add) without going through Electron's
 * IPC (which requires renderer access).
 *
 * Protocol:
 *   Client → Server: { id: string, action: string, payload: any }
 *   Server → Client: { id: string, ok: boolean, data?: any, error?: string }
 *
 * ## Why this file moved, and why it is still here
 *
 * P4 work item 1 asks for it to leave `src/companion/` ("the directory name `companion` will make
 * no sense once the browser extension is just another client") — done, it now sits beside the
 * transports it is being superseded by. It has **not** been rewritten to delegate to the
 * `CommandRegistry`, and that is a decision with evidence, not an omission: see below.
 *
 * It is scheduled for deletion once the extension speaks v2. Until then it is the *only* thing the
 * released extension can talk to, so it stays byte-compatible and keeps serving port 9271.
 *
 * ## The v1 envelope is frozen, and `error` must stay a **string**
 *
 * `../bifurc-extension` interpolates the error directly — `devtools-panel.js` does
 * ``showToast(`✗ Error: ${resp.error || "Unknown error"}`)``. An `{code, message, details}` object
 * would render as `[object Object]`. So this endpoint can never adopt the enriched `RpcError` that
 * `transport/ws.ts` serves; that is a v2 change on the extension's side.
 *
 * The extension also sends **no `hello`** and is a bare `{id, action, payload}` client, and for
 * `folder:add` it reads `resp.data.id` (`devtools-panel.js`) to pass as the `folderId` of the bulk
 * create that follows. All three are load-bearing.
 *
 * ## These four actions are NOT aliases of registry commands — do not "simplify" them
 *
 * The obvious migration is to map `mock:add` → `entity.create`, `folder:add` → `folder.add`,
 * `config:get` → `config.get` and delete the hand-written bodies. **That would break the frozen
 * contract in four ways**, and it would break the additive-only property this allowlist exists to
 * guarantee. Verified against `src/ipc/handlers/crudHandlers.ts`, `entityCrudFactory.ts` and
 * `folderHandlers.ts` on 2026-09-18:
 *
 * | | v1 (here) | registry equivalent |
 * |---|---|---|
 * | mock validation | `"urlPattern is required"` | `"urlPattern is required **for mocks**"` |
 * | request validation | `"url is required"` | `"url is required **for requests**"` |
 * | duplicate mock | adds verbatim | `onAddConflict` **disables an existing enabled mock** |
 * | blank folder name | refused, `"name is required"` | **no check** — creates a folder named `"   "` |
 * | unknown folder kind | `"Unknown folder kind: bogus"` | `TypeError` from `FOLDER_CONFIG[undefined]` |
 *
 * The first four are asserted verbatim by `tests/integration/companionServer.integration.test.ts`
 * (exact `error` strings), so aliasing fails those tests outright. The third is the serious one:
 * `onAddConflict` mutates *existing* state, which contradicts `V1_COMPANION_ACTIONS`' own
 * documented rule that the companion is a lower-trust caller and may only **add**. A browser page
 * capturing a URL the user already has mocked would silently switch that mock off.
 *
 * So the v2 mapping is a **decision to be taken, not a refactor to be performed** — either the
 * extension inherits the registry's de-duplication (and the additive-only guarantee is formally
 * relaxed), or the v2 commands keep explicit additive semantics. `plan/05`'s item 1 records the
 * same table. Do not resolve it by editing this file.
 */

import * as fs from "fs";
import * as path from "path";
import { WebSocketServer, WebSocket } from "ws";
import { V1_COMPANION_ACTIONS } from "./legacyCompanionActions";
import {
    loadConfig, saveConfig, generateId, AppConfig,
    MockRule, SavedRequest, Folder,
} from "../store/config";
import {
    writeEntity, upsertNameEntry, findEntityRelPath,
    readEnabledSet, writeEnabledSet, bootstrapEnabledSet,
    readIndex, writeIndex, sanitizeDirName, wsDir,
} from "../store/workspaceFs";
import { invalidateCache } from "../sync/statusTracker";
import { reloadConfig } from "../proxy/server";
import { bus, emitEntityStatus } from "../eventBus";

// ── Types ─────────────────────────────────────────────────────────────────────

interface IncomingMessage {
    id: string;
    action: string;
    payload: any;
}

interface OutgoingMessage {
    id: string;
    ok: boolean;
    data?: any;
    error?: string;
}

// ── Server state ──────────────────────────────────────────────────────────────

let wss: WebSocketServer | null = null;
let currentPort: number = 9271;

// ── Helpers ───────────────────────────────────────────────────────────────────

function syncEnabledSet(wsId: string, kind: string, id: string, enabled: boolean): void {
    const current = readEnabledSet(wsId, kind) ?? bootstrapEnabledSet(wsId, kind);
    if (enabled) {
        current.add(id);
    } else {
        current.delete(id);
    }
    writeEnabledSet(wsId, kind, current);
}

// ── Action handlers ───────────────────────────────────────────────────────────

async function handleAction(action: string, payload: any): Promise<any> {
    switch (action) {
        case "config:get":
            return loadConfig();

        case "mock:add":
            return handleMockAdd(payload);

        case "request:add":
            return handleRequestAdd(payload);

        case "folder:add":
            return handleFolderAdd(payload);

        default:
            throw new Error(`Unknown action: ${action}`);
    }
}

function handleMockAdd(mock: Omit<MockRule, "id" | "createdAt">): MockRule {
    // Validate required fields
    if (!mock.urlPattern || !mock.urlPattern.trim()) {
        throw new Error("urlPattern is required");
    }
    if (!mock.method || !mock.method.trim()) {
        throw new Error("method is required");
    }

    const cfg = loadConfig();
    const wsId = mock.workspaceId ?? cfg.activeWorkspaceId;
    const newMock: MockRule = {
        ...mock,
        id: generateId(),
        createdAt: Date.now(),
        workspaceId: wsId,
        enabled: mock.enabled ?? true,
    };

    // Persist
    const folderName = newMock.folderId
        ? (cfg.mockFolders ?? []).find((f) => f.id === newMock.folderId)?.name
        : null;
    writeEntity(wsId, "mocks", newMock.id, newMock, folderName ?? null);
    syncEnabledSet(wsId, "mocks", newMock.id, newMock.enabled);
    upsertNameEntry(wsId, "mocks", newMock.id, {
        name: newMock.name,
        method: newMock.method,
        url: newMock.urlPattern,
    });
    reloadConfig();
    emitEntityStatus(wsId);
    bus.emitTyped("entity.changed", { wsId, kind: "mocks", id: newMock.id, action: "created" });
    return newMock;
}

function handleRequestAdd(req: Omit<SavedRequest, "id" | "createdAt">): SavedRequest {
    // Validate required fields
    if (!req.url || !req.url.trim()) {
        throw new Error("url is required");
    }
    if (!req.method || !req.method.trim()) {
        throw new Error("method is required");
    }

    const cfg = loadConfig();
    const wsId = req.workspaceId ?? cfg.activeWorkspaceId;
    const newReq: SavedRequest = {
        ...req,
        id: generateId(),
        createdAt: Date.now(),
        workspaceId: wsId,
    };

    const folderName = newReq.folderId
        ? (cfg.requestFolders ?? []).find((f) => f.id === newReq.folderId)?.name
        : null;
    writeEntity(wsId, "requests", newReq.id, newReq, folderName ?? null);
    upsertNameEntry(wsId, "requests", newReq.id, {
        name: newReq.name,
        method: newReq.method,
        url: newReq.url,
    });
    reloadConfig();
    emitEntityStatus(wsId);
    bus.emitTyped("entity.changed", { wsId, kind: "requests", id: newReq.id, action: "created" });
    return newReq;
}

function handleFolderAdd(payload: { kind: string; name: string; parentId?: string | null }): Folder {
    const { kind, name, parentId = null } = payload;
    if (!name || !name.trim()) throw new Error("name is required");
    const validKinds = ["mock", "request", "ws", "webhook", "rule",
        "graphqlRequest", "graphqlMock", "grpcRequest", "grpcMock", "soapRequest", "soapMock"];
    if (!validKinds.includes(kind)) throw new Error(`Unknown folder kind: ${kind}`);

    const cfg = loadConfig();
    const wsId = cfg.activeWorkspaceId;
    const folder: Folder = { id: generateId(), name: name.trim(), parentId, workspaceId: wsId, createdAt: Date.now() };
    const kindMap: Record<string, string> = {
        mock: "mockFolders", request: "requestFolders", ws: "wsFolders",
        webhook: "webhookFolders", rule: "ruleFolders",
        graphqlRequest: "graphqlRequestFolders", graphqlMock: "graphqlMockFolders",
        grpcRequest: "grpcRequestFolders", grpcMock: "grpcMockFolders",
        soapRequest: "soapRequestFolders", soapMock: "soapMockFolders",
    };
    const fsKindMap: Record<string, string> = {
        mock: "mocks", request: "requests", ws: "sockets",
        webhook: "webhooks", rule: "rules",
        graphqlRequest: "graphqlRequests", graphqlMock: "graphqlMocks",
        grpcRequest: "grpcRequests", grpcMock: "grpcMocks",
        soapRequest: "soapRequests", soapMock: "soapMocks",
    };
    const key = kindMap[kind];
    const fsKind = fsKindMap[kind];
    (cfg as any)[key] = [...((cfg as any)[key] ?? []), folder];
    saveConfig(cfg);
    // Create the physical directory on disk
    const folderDir = path.join(wsDir(wsId), fsKind, sanitizeDirName(folder.name));
    fs.mkdirSync(folderDir, { recursive: true });
    // Persist folder to index.json so subsequent entity writes can resolve folderId
    const idx = readIndex(wsId, fsKind);
    idx.folders.push(folder);
    writeIndex(wsId, fsKind, idx);
    bus.emitTyped("entity.changed", { wsId, kind: fsKind, id: folder.id, action: "created" });
    return folder;
}

// ── WebSocket message handler ─────────────────────────────────────────────────

function handleConnection(ws: WebSocket): void {
    ws.on("message", async (raw) => {
        let msg: IncomingMessage;
        try {
            msg = JSON.parse(raw.toString());
        } catch {
            ws.send(JSON.stringify({ id: "unknown", ok: false, error: "Invalid JSON" }));
            return;
        }

        const { id, action, payload } = msg;

        if (!id || !action) {
            ws.send(JSON.stringify({ id: id ?? "unknown", ok: false, error: "Missing id or action" }));
            return;
        }

        if (!V1_COMPANION_ACTIONS.has(action)) {
            ws.send(JSON.stringify({ id, ok: false, error: `Action "${action}" is not allowed` }));
            return;
        }

        try {
            const data = await handleAction(action, payload);
            console.log(`[companion] Successfully handled action: ${action}`);
            ws.send(JSON.stringify({ id, ok: true, data } as OutgoingMessage));
        } catch (err: unknown) {
            const error = err instanceof Error ? err.message : "Unknown error";
            console.error(`[companion] Error handling action "${action}":`, error);
            ws.send(JSON.stringify({ id, ok: false, error } as OutgoingMessage));
        }
    });

    ws.on("error", () => {
        // Silently handle client errors — connection will close
    });
}

// ── Public API ────────────────────────────────────────────────────────────────

export function startCompanionServer(port: number): void {
    if (wss) stopCompanionServer();
    currentPort = port;

    wss = new WebSocketServer({
        host: "127.0.0.1", // Bind to localhost only for security
        port,
    });

    wss.on("connection", handleConnection);

    wss.on("error", (err) => {
        console.error(`[companion] WebSocket server error on port ${port}:`, err.message);
    });

    wss.on("listening", () => {
        console.log(`[companion] WebSocket server listening on ws://127.0.0.1:${port}`);
    });
}

export function stopCompanionServer(): void {
    if (wss) {
        // Close all active connections
        for (const client of wss.clients) {
            client.close(1001, "Server shutting down");
        }
        wss.close();
        wss = null;
        console.log("[companion] WebSocket server stopped");
    }
}

export function restartCompanionServer(port: number): void {
    stopCompanionServer();
    startCompanionServer(port);
}

export function getCompanionPort(): number {
    return currentPort;
}

export function isCompanionRunning(): boolean {
    return wss !== null;
}
