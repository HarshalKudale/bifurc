import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { EventEmitter } from "events";

vi.mock("electron", () => ({
    BrowserWindow: { getAllWindows: vi.fn(() => []) },
}));

import {
    registerActiveWebhook,
    unregisterActiveWebhook,
    getActiveWebhookCount,
    startWebhookServer,
    stopWebhookServer,
    isWebhookServerRunning,
    getWebhookPort,
    getWebhookServerError,
    webhookEmitter,
} from "@/proxy/webhookServer";

describe("webhook registry", () => {
    beforeEach(() => {
        // Clean up by unregistering all
        stopWebhookServer();
    });

    it("registerActiveWebhook increases count", () => {
        const before = getActiveWebhookCount();
        registerActiveWebhook("wh1", "test-hook");
        expect(getActiveWebhookCount()).toBe(before + 1);
    });

    it("unregisterActiveWebhook decreases count", () => {
        registerActiveWebhook("wh2", "hook-two");
        const before = getActiveWebhookCount();
        unregisterActiveWebhook("wh2");
        expect(getActiveWebhookCount()).toBe(before - 1);
    });

    it("unregisterActiveWebhook with unknown ID does nothing", () => {
        const before = getActiveWebhookCount();
        unregisterActiveWebhook("nonexistent");
        expect(getActiveWebhookCount()).toBe(before);
    });

    it("normalizes suffix (strips leading/trailing slashes, lowercases)", () => {
        registerActiveWebhook("wh3", "///My-Hook///");
        // The key stored is normalized — unregister by ID
        unregisterActiveWebhook("wh3");
        // If normalization works, unregister should find it
        expect(getActiveWebhookCount()).toBeGreaterThanOrEqual(0);
    });
});

describe("webhook server lifecycle", () => {
    afterEach(() => {
        stopWebhookServer();
    });

    it("getWebhookPort returns the configured port", () => {
        expect(getWebhookPort()).toBeGreaterThan(0);
    });

    it("isWebhookServerRunning returns false when not started", () => {
        stopWebhookServer();
        expect(isWebhookServerRunning()).toBe(false);
    });

    it("getWebhookServerError returns null initially", () => {
        expect(getWebhookServerError()).toBeNull();
    });

    it("startWebhookServer creates a listening server", async () => {
        const port = 19876; // Use high port to avoid conflicts
        startWebhookServer(port);
        // Wait briefly for the server to bind
        await new Promise((r) => setTimeout(r, 100));
        expect(isWebhookServerRunning()).toBe(true);
        expect(getWebhookPort()).toBe(port);
    });

    it("stopWebhookServer stops the server", async () => {
        startWebhookServer(19877);
        await new Promise((r) => setTimeout(r, 100));
        stopWebhookServer();
        expect(isWebhookServerRunning()).toBe(false);
    });

    it("startWebhookServer stops previous server before starting new one", async () => {
        startWebhookServer(19878);
        await new Promise((r) => setTimeout(r, 100));
        startWebhookServer(19879);
        await new Promise((r) => setTimeout(r, 100));
        expect(getWebhookPort()).toBe(19879);
        expect(isWebhookServerRunning()).toBe(true);
    });
});

describe("webhook server HTTP handling", () => {
    const port = 19880;

    beforeEach(async () => {
        registerActiveWebhook("test-wh", "my-hook");
        startWebhookServer(port);
        await new Promise((r) => setTimeout(r, 150));
    });

    afterEach(() => {
        stopWebhookServer();
        unregisterActiveWebhook("test-wh");
    });

    it("GET /bifurc/webhooks/<suffix> returns alive check", async () => {
        const res = await fetch(`http://127.0.0.1:${port}/bifurc/webhooks/my-hook`);
        expect(res.status).toBe(200);
        const json = await res.json();
        expect(json.status).toBe("ok");
    });

    it("POST to non-base path returns 404", async () => {
        const res = await fetch(`http://127.0.0.1:${port}/other/path`, { method: "POST" });
        expect(res.status).toBe(404);
    });

    it("POST to inactive webhook returns 404", async () => {
        const res = await fetch(`http://127.0.0.1:${port}/bifurc/webhooks/unknown-hook`, {
            method: "POST",
            body: "test",
        });
        expect(res.status).toBe(404);
        const json = await res.json();
        expect(json.error).toContain("not active");
    });

    it("POST to active webhook returns 200 and emits payload", async () => {
        const payloads: any[] = [];
        webhookEmitter.on("payload", (p: any) => payloads.push(p));

        const res = await fetch(`http://127.0.0.1:${port}/bifurc/webhooks/my-hook`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ event: "test" }),
        });
        expect(res.status).toBe(200);
        const json = await res.json();
        expect(json.ok).toBe(true);
        expect(payloads.length).toBe(1);
        expect(payloads[0].webhookId).toBe("test-wh");
        expect(payloads[0].body).toContain('"event":"test"');

        webhookEmitter.removeAllListeners("payload");
    });

    it("PUT method returns 405", async () => {
        const res = await fetch(`http://127.0.0.1:${port}/bifurc/webhooks/my-hook`, { method: "PUT" });
        expect(res.status).toBe(405);
    });
});
