import { describe, it, expect, afterEach } from "vitest";
import * as net from "net";
import { checkPortInUse, killProcessOnPort } from "@/applications/portUtils";

/**
 * Port checking backs the "this port is already in use" guard shown before
 * starting an application or the proxy. It is entirely I/O — it binds a real
 * socket and, when that fails, shells out to `netstat`/`lsof` — so it is verified
 * against real ports here rather than mocked.
 */

const open: net.Server[] = [];

/** Bind a real port on 127.0.0.1 and return it. */
function occupyPort(): Promise<number> {
    return new Promise((resolve, reject) => {
        const srv = net.createServer();
        srv.on("error", reject);
        srv.listen(0, "127.0.0.1", () => {
            open.push(srv);
            const addr = srv.address();
            resolve(typeof addr === "object" && addr ? addr.port : 0);
        });
    });
}

/** Ask the OS for a port and immediately release it, so it is free. */
function freePort(): Promise<number> {
    return new Promise((resolve, reject) => {
        const srv = net.createServer();
        srv.on("error", reject);
        srv.listen(0, "127.0.0.1", () => {
            const addr = srv.address();
            const port = typeof addr === "object" && addr ? addr.port : 0;
            srv.close(() => resolve(port));
        });
    });
}

afterEach(() => {
    while (open.length) open.pop()!.close();
});

describe("checkPortInUse", () => {
    it("reports a free port as not in use", async () => {
        const port = await freePort();
        const info = await checkPortInUse(port);
        expect(info.inUse).toBe(false);
    });

    it("reports an occupied port as in use", async () => {
        const port = await occupyPort();
        const info = await checkPortInUse(port);
        expect(info.inUse).toBe(true);
    });

    it("resolves rather than rejecting when the port is occupied", async () => {
        // The whole point of the helper is that it never throws at the call site.
        const port = await occupyPort();
        await expect(checkPortInUse(port)).resolves.toBeDefined();
    });

    it("always resolves to a boolean inUse flag", async () => {
        const port = await freePort();
        const info = await checkPortInUse(port);
        expect(typeof info.inUse).toBe("boolean");
    });

    it("can be called repeatedly for the same port", async () => {
        const port = await occupyPort();
        const a = await checkPortInUse(port);
        const b = await checkPortInUse(port);
        expect(a.inUse).toBe(true);
        expect(b.inUse).toBe(true);
    });
});

describe("killProcessOnPort", () => {
    it("reports failure when nothing is listening on the port", async () => {
        // Deliberately only exercised against a FREE port: killing the process that
        // owns an in-use port is destructive and must never run in a test suite.
        const port = await freePort();
        const res = await killProcessOnPort(port);
        expect(res.ok).toBe(false);
    });

    it("always resolves to an ok flag rather than throwing", async () => {
        const port = await freePort();
        await expect(killProcessOnPort(port)).resolves.toHaveProperty("ok");
    });
});
