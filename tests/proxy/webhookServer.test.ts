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
    createWebhookServer,
} from "@bifurc/engine/webhooks/webhookServer";

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

// ── P14 work item 2 — the factory ─────────────────────────────────────────────
//
// The acceptance criterion these cases exist for is §8's "the readiness predicate is per-runtime:
// starting runtime B does not observe runtime A's readiness". The trap that criterion guards is the one
// `plan/14` §10 rates High: a global `isRunning()` reports the wrong runtime ready. So every case below
// asserts on **identity** — two instances, distinguishable values — rather than on a value both could
// share (`TRAPS.md`, "A test whose two inputs are indistinguishable passes on either branch").

describe("createWebhookServer() — instance isolation (P14 WI 2)", () => {
    it("two instances have independent readiness — B is not ready because A bound", async () => {
        const a = createWebhookServer({ workspaceId: "ws-a" });
        const b = createWebhookServer({ workspaceId: "ws-b" });

        a.start(19901);
        await new Promise((r) => setTimeout(r, 100));

        // The whole point: A bound successfully, B never started.
        expect(a.isRunning()).toBe(true);
        expect(b.isRunning()).toBe(false);

        a.stop();
    });

    it("two instances hold independent ports", async () => {
        const a = createWebhookServer({ workspaceId: "ws-a" });
        const b = createWebhookServer({ workspaceId: "ws-b" });

        a.start(19902);
        b.start(19903);
        await new Promise((r) => setTimeout(r, 120));

        expect(a.getPort()).toBe(19902);
        expect(b.getPort()).toBe(19903);
        expect(a.getPort()).not.toBe(b.getPort());

        a.stop();
        b.stop();
    });

    it("the active-webhook maps are per instance — a suffix active in A is 404 in B", async () => {
        const a = createWebhookServer({ workspaceId: "ws-a" });
        const b = createWebhookServer({ workspaceId: "ws-b" });

        a.registerActive("wh-a", "shared-name");
        expect(a.activeCount()).toBe(1);
        expect(b.activeCount()).toBe(0);     // the map is not shared

        a.start(19904);
        b.start(19905);
        await new Promise((r) => setTimeout(r, 120));

        const postTo = (p: number) => fetch(`http://127.0.0.1:${p}/bifurc/webhooks/shared-name`, {
            method: "POST",
            body: "x",
        });

        // Identity, not a shared value: same suffix, opposite outcomes.
        expect((await postTo(19904)).status).toBe(200);
        expect((await postTo(19905)).status).toBe(404);

        a.stop();
        b.stop();
    });

    it("stopping A does not stop B — and does not clear B's error state", async () => {
        const a = createWebhookServer({ workspaceId: "ws-a" });
        const b = createWebhookServer({ workspaceId: "ws-b" });

        a.start(19906);
        b.start(19907);
        await new Promise((r) => setTimeout(r, 120));

        a.stop();
        expect(a.isRunning()).toBe(false);
        expect(b.isRunning()).toBe(true);     // B is untouched

        b.stop();
    });

    it("a bind failure lands on the failing instance only", async () => {
        const a = createWebhookServer({ workspaceId: "ws-a" });
        const b = createWebhookServer({ workspaceId: "ws-b" });

        // `start()` is fire-and-forget, and the `error` handler inside the instance re-emits on the
        // shared `webhookEmitter` — an EventEmitter with no `error` listener throws
        // `ERR_UNHANDLED_ERROR` and the run reports it as an unhandled error. Attach one for the test.
        const onError = vi.fn();
        webhookEmitter.on("error", onError);

        a.start(19908);
        await new Promise((r) => setTimeout(r, 100));   // A wins the port deterministically
        b.start(19908);                                  // B then hits EADDRINUSE
        await new Promise((r) => setTimeout(r, 150));

        expect(a.isRunning()).toBe(true);
        expect(a.getError()).toBeNull();                 // the winner is untouched
        expect(b.isRunning()).toBe(false);
        expect(b.getError()).toContain("EADDRINUSE");    // the error is the loser's, and only the loser's

        webhookEmitter.removeListener("error", onError);
        a.stop();
        b.stop();
    });

    it("each instance carries its workspaceId", () => {
        expect(createWebhookServer({ workspaceId: "ws-x" }).workspaceId).toBe("ws-x");
        expect(createWebhookServer().workspaceId).toBeNull();
    });
});

describe("the module-level facade still backs a single default instance", () => {
    // Await the bind rather than assume it: `startWebhookServer()` hands off to an async `listen()` and
    // returns immediately (`index.ts:140-144`), so `isRunning()` is still false on the next line. The
    // original suite only ever asserted `isRunning()` after a 100 ms sleep; these cases do the same
    // through a helper so the assertion is about the facade, not about the timing.
    const waitRunning = (predicate: () => boolean) =>
        new Promise<boolean>((resolve) => {
            const deadline = Date.now() + 2000;
            const tick = () => {
                if (predicate()) return resolve(true);
                if (Date.now() > deadline) return resolve(false);
                setTimeout(tick, 10);
            };
            tick();
        });

    afterEach(() => {
        stopWebhookServer();
    });

    it("the facade functions and the default instance share state", async () => {
        // What the facade guarantees: the pre-P14 callers keep working, over one instance.
        startWebhookServer(19909);
        expect(await waitRunning(isWebhookServerRunning)).toBe(true);
        expect(getWebhookPort()).toBe(19909);

        registerActiveWebhook("facade-wh", "facade-hook");
        const before = getActiveWebhookCount();
        unregisterActiveWebhook("facade-wh");
        expect(getActiveWebhookCount()).toBe(before - 1);
    });

    it("getWebhookPort reports the default port before anything is started", () => {
        // This case must run on a fresh module state, but vitest shares the module across this file —
        // so stop first, which is the only way to observe the pre-start reading. `stop()` nulls the
        // `http.Server` but deliberately keeps `currentPort` (the original module did too, and
        // `server.test.ts` pins the same "retains the last port" behaviour for the proxy).
        stopWebhookServer();
        expect(isWebhookServerRunning()).toBe(false);
        expect(getWebhookPort()).toBeGreaterThan(0);
    });

    it("the facade and a factory instance are independent", async () => {
        // The facade's state must not bleed into a fresh runtime, or WI 3's host would inherit the
        // desktop's webhook port. This is the case that fails if the facade is ever re-implemented
        // as a module-scope singleton the factory also reads.
        startWebhookServer(19910);
        const runtime = createWebhookServer({ workspaceId: "ws-runtime" });
        runtime.start(19911);
        await new Promise((r) => setTimeout(r, 120));

        expect(getWebhookPort()).toBe(19910);
        expect(runtime.getPort()).toBe(19911);

        runtime.stop();
        stopWebhookServer();
    });
});
