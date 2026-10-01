import { describe, it, expect, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { HELLO_ACTION, PROTOCOL_VERSION, type Transport } from "@bifurc/protocol";
import { CommandRegistry } from "@bifurc/engine/kernel/registry";
import { EngineEventBus } from "@bifurc/engine/kernel/eventBus";
import { attachEngineSocket } from "@bifurc/engine/runtime/attachSocket";
import { findRunningEngine, readRuntimeInfo, writeRuntimeInfo } from "@bifurc/engine/runtime/discovery";
import { createSocketTransport } from "@bifurc/engine/transport/socket";

// An engine already running in this process (the desktop's) is reachable by the CLI through its socket, and a
// second engine is never started alongside it.

const dirs: string[] = [];
const tmp = (): string => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-attach-"));
  dirs.push(d);
  return d;
};
afterEach(() => {
  for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true });
});

describe("attachEngineSocket", () => {
  it("serves the running registry over the data dir's socket and publishes it in runtime.json", async () => {
    const dataDir = tmp();
    const registry = new CommandRegistry();
    registry.register("config.get", () => ({ from: "the engine in this process" }));
    const attached = await attachEngineSocket({
      dataDir, registry, bus: new EngineEventBus(), onNotice: () => undefined,
    });
    expect(attached).not.toBeNull();
    expect(findRunningEngine(dataDir)).toMatchObject({ pid: process.pid, transport: "socket", address: attached!.address });

    const client: Transport = createSocketTransport(attached!.address);
    try {
      await client.request(HELLO_ACTION, { protocolVersion: PROTOCOL_VERSION, clientName: "cli", clientVersion: "0" });
      expect(await client.request("config.get", {})).toEqual({ from: "the engine in this process" });
    } finally {
      await client.close();
    }

    await attached!.close();
    expect(readRuntimeInfo(dataDir)).toBeNull();
  });

  it("does not open a second endpoint when another live engine already serves the data dir", async () => {
    const dataDir = tmp();
    // The parent process is certainly alive and is not us.
    const otherPid = process.ppid;
    writeRuntimeInfo({
      pid: otherPid, transport: "socket", address: "x", dataDir,
      protocolVersion: PROTOCOL_VERSION, engineVersion: "0", startedAt: 1,
    });
    const attached = await attachEngineSocket({
      dataDir, registry: new CommandRegistry(), bus: new EngineEventBus(), onNotice: () => undefined,
    });
    expect(attached).toBeNull();
    expect(readRuntimeInfo(dataDir)?.pid).toBe(otherPid);
  });
});
