import { describe, it, expect, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import {
  engineSocketPath, findRunningEngine, readRuntimeInfo, removeRuntimeInfo, runtimeFilePath, writeRuntimeInfo,
  type EngineRuntimeInfo,
} from "@bifurc/engine/runtime/discovery";

// A client that knows only the data directory must be able to find the engine serving it.

const dirs: string[] = [];
const tmp = (): string => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-discovery-"));
  dirs.push(d);
  return d;
};
afterEach(() => {
  for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true });
});

const info = (dataDir: string, pid: number): EngineRuntimeInfo => ({
  pid, transport: "socket", address: engineSocketPath(dataDir), dataDir,
  protocolVersion: "1.0.0", engineVersion: "0.0.0", startedAt: 1,
});

describe("engine discovery", () => {
  it("derives a stable socket address from the data dir, not the pid", () => {
    const a = tmp();
    const b = tmp();
    expect(engineSocketPath(a)).toBe(engineSocketPath(a));
    expect(engineSocketPath(a)).not.toBe(engineSocketPath(b));
    expect(engineSocketPath(a)).not.toContain(String(process.pid));
    if (process.platform === "win32") expect(engineSocketPath(a)).toMatch(/^\\\\\.\\pipe\\bifurc-/);
  });

  it("finds a running engine through runtime.json", () => {
    const dir = tmp();
    writeRuntimeInfo(info(dir, process.pid));
    expect(readRuntimeInfo(dir)?.address).toBe(engineSocketPath(dir));
    expect(findRunningEngine(dir)?.pid).toBe(process.pid);
  });

  it("treats a runtime.json left by a dead process as no engine", () => {
    const dir = tmp();
    writeRuntimeInfo(info(dir, 2 ** 30));
    expect(readRuntimeInfo(dir)).not.toBeNull();
    expect(findRunningEngine(dir)).toBeNull();
  });

  it("removes runtime.json only for the pid that wrote it", () => {
    const dir = tmp();
    writeRuntimeInfo(info(dir, process.pid));
    removeRuntimeInfo(dir, process.pid + 1);
    expect(fs.existsSync(runtimeFilePath(dir))).toBe(true);
    removeRuntimeInfo(dir, process.pid);
    expect(fs.existsSync(runtimeFilePath(dir))).toBe(false);
  });
});
