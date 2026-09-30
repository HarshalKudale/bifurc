import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { HELLO_ACTION, PROTOCOL_VERSION, type Transport } from "@bifurc/protocol";
import { createSocketTransport } from "@bifurc/engine/transport/socket";
import { createWsTransport } from "@bifurc/engine/transport/ws";
import { findRunningEngine, type EngineRuntimeInfo } from "@bifurc/engine/runtime/discovery";

/** An error whose message is the whole story for the user — printed without a stack. */
export class CliError extends Error {}

export interface EngineConnection {
  info: EngineRuntimeInfo;
  request<T = unknown>(command: string, payload?: unknown): Promise<T>;
  close(): Promise<void>;
}

/** Connect to the engine serving `dataDir`, found through its runtime.json, and open a session. */
export async function connect(dataDir: string, clientVersion: string): Promise<EngineConnection> {
  const info = findRunningEngine(dataDir);
  if (info === null) {
    throw new CliError(`No engine is running for ${dataDir}.\nStart one with: bifurc engine start`);
  }
  if (info.transport === "stdio") {
    throw new CliError(
      `The engine for ${dataDir} (pid ${info.pid}) serves stdio only — it belongs to the process that ` +
        `spawned it and cannot be reached from here.`,
    );
  }
  const transport: Transport =
    info.transport === "ws" ? createWsTransport(info.address) : createSocketTransport(info.address);
  try {
    await transport.request(HELLO_ACTION, { protocolVersion: PROTOCOL_VERSION, clientName: "cli", clientVersion });
  } catch (err) {
    await transport.close().catch(() => undefined);
    throw new CliError(`Could not open a session with the engine at ${info.address}: ${(err as Error).message}`);
  }
  return {
    info,
    request: <T>(command: string, payload: unknown = {}) => transport.request(command, payload) as Promise<T>,
    close: () => transport.close(),
  };
}

/**
 * Start a standalone engine for `dataDir` in the background and wait until it has published its address.
 * Its output goes to `<dataDir>/engine.log`.
 */
export async function startEngine(
  dataDir: string,
  transport: "socket" | "ws",
  timeoutMs = 20_000,
): Promise<EngineRuntimeInfo> {
  const hostCli = require.resolve("@bifurc/engine/runtime/hostCli");
  fs.mkdirSync(dataDir, { recursive: true });
  const logPath = path.join(dataDir, "engine.log");
  const log = fs.openSync(logPath, "a");
  const child = spawn(process.execPath, [hostCli, "--data-dir", dataDir, "--transport", transport], {
    detached: true,
    stdio: ["ignore", log, log],
    windowsHide: true,
  });
  let exitCode: number | null = null;
  child.on("exit", (code) => {
    exitCode = code ?? 1;
  });
  child.unref();
  fs.closeSync(log);

  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const info = findRunningEngine(dataDir);
    if (info !== null && info.pid === child.pid) return info;
    if (exitCode !== null) break;
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  const tail = tailOf(logPath);
  throw new CliError(
    exitCode !== null
      ? `The engine exited (code ${exitCode}) before it was ready.\n${tail}`
      : `The engine did not become ready within ${timeoutMs / 1000}s. See ${logPath}.\n${tail}`,
  );
}

/** Ask the engine to shut down and wait until it is gone. */
export async function stopEngine(conn: EngineConnection, dataDir: string, timeoutMs = 15_000): Promise<void> {
  await conn.request("engine.shutdown", {});
  await conn.close().catch(() => undefined);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (findRunningEngine(dataDir) === null) return;
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  throw new CliError(`The engine (pid ${conn.info.pid}) acknowledged the shutdown but is still running.`);
}

function tailOf(file: string, lines = 12): string {
  try {
    return fs.readFileSync(file, "utf8").trimEnd().split(/\r?\n/).slice(-lines).join("\n");
  } catch {
    return "";
  }
}
