/**
 * Port utilities — check if a TCP port is occupied and kill the owning process.
 */

import { exec } from "child_process";
import * as net from "net";

export interface PortInfo {
    inUse: boolean;
    pid?: number;
}

/**
 * Check if a local TCP port is in use.
 * If it is, attempt to resolve the PID that owns it.
 */
export function checkPortInUse(port: number): Promise<PortInfo> {
    return new Promise((resolve) => {
        const server = net.createServer();

        server.on("error", () => {
            server.close();
            findPidOnPort(port).then((pid) =>
                resolve({ inUse: true, pid }),
            );
        });

        server.listen(port, "127.0.0.1", () => {
            server.close(() => resolve({ inUse: false }));
        });
    });
}

/**
 * Kill whatever process is listening on the given port.
 */
export function killProcessOnPort(port: number): Promise<{ ok: boolean }> {
    return findPidOnPort(port).then((pid) => {
        if (!pid) return { ok: false };
        return new Promise<{ ok: boolean }>((resolve) => {
            const cmd =
                process.platform === "win32"
                    ? `taskkill /F /PID ${pid}`
                    : `kill -9 ${pid}`;
            exec(cmd, (err) => resolve({ ok: !err }));
        });
    });
}

// ── Internal helpers ──────────────────────────────────────────────────────────

function findPidOnPort(port: number): Promise<number | undefined> {
    return new Promise((resolve) => {
        if (process.platform === "win32") {
            // netstat -ano gives lines like:
            //   TCP    0.0.0.0:3000    0.0.0.0:0    LISTENING    1234
            exec("netstat -ano -p TCP", (err, stdout) => {
                if (err) return resolve(undefined);
                const re = new RegExp(
                    `(?:0\\.0\\.0\\.0|127\\.0\\.0\\.1|\\[::\\]):${port}\\s+\\S+\\s+LISTENING\\s+(\\d+)`,
                    "i",
                );
                const m = stdout.match(re);
                resolve(m ? parseInt(m[1], 10) : undefined);
            });
        } else {
            // lsof -t returns just the PID(s), one per line
            exec(`lsof -t -i :${port}`, (_, stdout) => {
                const line = stdout.trim().split("\n")[0];
                const pid = line ? parseInt(line, 10) : undefined;
                resolve(pid && !isNaN(pid) ? pid : undefined);
            });
        }
    });
}
