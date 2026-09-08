/**
 * Companion WebSocket Server
 *
 * Provides a localhost-only WebSocket bridge that allows the companion browser
 * extension to execute IPC-equivalent commands (e.g. mock:add, request:add)
 * without going through Electron's IPC (which requires renderer access).
 *
 * Protocol:
 *   Client → Server: { id: string, action: string, payload: any }
 *   Server → Client: { id: string, ok: boolean, data?: any, error?: string }
 */

import * as fs from "fs";
import * as path from "path";
import { WebSocketServer, WebSocket } from "ws";
import { BrowserWindow } from "electron";
import { ALLOWED_ACTIONS } from "@/companion/allowedActions";
import {
    loadConfig, saveConfig, generateId, AppConfig,
    MockRule, SavedRequest, Folder,
} from "@/store/config";
import {
    writeEntity, upsertNameEntry, findEntityRelPath,
    readEnabledSet, writeEnabledSet, bootstrapEnabledSet,
    readIndex, writeIndex, sanitizeDirName, wsDir,
} from "@/store/workspaceFs";
import { getWorkspaceSyncStatus, invalidateCache } from "@/sync/statusTracker";
import { reloadConfig } from "@/proxy/server";

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

function broadcastEntityStatus(wsId: string): void {
    const windows = BrowserWindow.getAllWindows();
    const status = getWorkspaceSyncStatus(wsId);
    // Serialize the status to plain object to avoid "Failed to serialize arguments" error
    const serializedStatus = JSON.parse(JSON.stringify(status));
    for (const w of windows) {
        if (!w.isDestroyed()) {
            try {
                w.webContents.send("sync:entityStatus", { wsId, status: serializedStatus });
            } catch (err) {
                console.error('[companion] Failed to broadcast entity status:', err);
            }
        }
    }
}

function syncEnabledSet(wsId: string, kind: string, id: string, enabled: boolean): void {
    const current = readEnabledSet(wsId, kind) ?? bootstrapEnabledSet(wsId, kind);
    if (enabled) {
        current.add(id);
    } else {
        current.delete(id);
    }
    writeEnabledSet(wsId, kind, current);
}

function notifyRendererRefresh(): void {
    const windows = BrowserWindow.getAllWindows();
    for (const w of windows) {
        if (!w.isDestroyed()) w.webContents.send("companion:refresh");
    }
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
    broadcastEntityStatus(wsId);
    notifyRendererRefresh();
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
    broadcastEntityStatus(wsId);
    notifyRendererRefresh();
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
    notifyRendererRefresh();
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

        if (!ALLOWED_ACTIONS.has(action)) {
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
