import { execSync } from "child_process";

export interface ServiceInfo {
  port: number;
  address: string;
  pid: number;
  processName: string;
}

// ── Windows ────────────────────────────────────────────────────────────────

function discoverWindows(): ServiceInfo[] {
  const PS = "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";

  function ps(command: string): string {
    return execSync(`"${PS}" -NoProfile -NonInteractive -Command "${command}"`, {
      encoding: "utf-8",
      timeout: 10000,
    });
  }

  let connections: { LocalAddress: string; LocalPort: number; OwningProcess: number }[];
  try {
    const raw = ps(
      "Get-NetTCPConnection -State Listen | Select-Object LocalAddress,LocalPort,OwningProcess | ConvertTo-Json -Compress"
    );
    const parsed = JSON.parse(raw.trim());
    connections = Array.isArray(parsed) ? parsed : [parsed];
  } catch (e) {
    console.error("[discovery] Get-NetTCPConnection failed:", e);
    return [];
  }

  const portMap = new Map<number, ServiceInfo>();
  for (const c of connections) {
    const addr = c.LocalAddress;
    if (addr !== "0.0.0.0" && addr !== "127.0.0.1" && addr !== "::" && addr !== "::1") continue;
    if (!portMap.has(c.LocalPort)) {
      portMap.set(c.LocalPort, { port: c.LocalPort, address: addr, pid: c.OwningProcess, processName: "" });
    }
  }

  if (portMap.size === 0) return [];

  const pids = [...new Set([...portMap.values()].map((s) => s.pid))];
  try {
    const pidList = pids.join(",");
    const raw = ps(
      `Get-Process -Id @(${pidList}) -ErrorAction SilentlyContinue | Select-Object Id,Name | ConvertTo-Json -Compress`
    );
    const parsed = JSON.parse(raw.trim());
    const procs: { Id: number; Name: string }[] = Array.isArray(parsed) ? parsed : [parsed];
    const pidToName = new Map(procs.map((p) => [p.Id, p.Name]));
    for (const svc of portMap.values()) {
      svc.processName = pidToName.get(svc.pid) ?? `PID ${svc.pid}`;
    }
  } catch {
    for (const svc of portMap.values()) {
      svc.processName = `PID ${svc.pid}`;
    }
  }

  return Array.from(portMap.values()).sort((a, b) => a.port - b.port);
}

// ── macOS / Linux ──────────────────────────────────────────────────────────

function discoverUnix(): ServiceInfo[] {
  // lsof is available on both macOS and most Linux distros
  let raw: string;
  try {
    raw = execSync("lsof -iTCP -sTCP:LISTEN -n -P 2>/dev/null", {
      encoding: "utf-8",
      timeout: 10000,
    });
  } catch (e) {
    // Fall back to ss (Linux) if lsof isn't available
    try {
      raw = execSync("ss -tlnp 2>/dev/null", { encoding: "utf-8", timeout: 10000 });
      return parseSsOutput(raw);
    } catch {
      console.error("[discovery] Both lsof and ss failed:", e);
      return [];
    }
  }
  return parseLsofOutput(raw);
}

function parseLsofOutput(raw: string): ServiceInfo[] {
  const portMap = new Map<number, ServiceInfo>();
  const lines = raw.trim().split("\n").slice(1); // skip header

  for (const line of lines) {
    const parts = line.trim().split(/\s+/);
    if (parts.length < 9) continue;
    const processName = parts[0];
    const pid = parseInt(parts[1], 10);
    const nameField = parts[8]; // e.g. "*:8080" or "127.0.0.1:8080 (LISTEN)"
    const match = nameField.match(/:(\d+)(?:\s+\(LISTEN\))?$/);
    if (!match) continue;
    const port = parseInt(match[1], 10);
    if (!portMap.has(port)) {
      portMap.set(port, { port, address: "127.0.0.1", pid, processName });
    }
  }
  return Array.from(portMap.values()).sort((a, b) => a.port - b.port);
}

function parseSsOutput(raw: string): ServiceInfo[] {
  const portMap = new Map<number, ServiceInfo>();
  const lines = raw.trim().split("\n").slice(1);

  for (const line of lines) {
    const parts = line.trim().split(/\s+/);
    if (parts.length < 5) continue;
    // ss format: State Recv-Q Send-Q Local:Port Peer:Port Process
    const localField = parts[3];
    const portMatch = localField.match(/:(\d+)$/);
    if (!portMatch) continue;
    const port = parseInt(portMatch[1], 10);
    const processField = parts[5] ?? "";
    const pidMatch = processField.match(/pid=(\d+)/);
    const nameMatch = processField.match(/"([^"]+)"/);
    const pid = pidMatch ? parseInt(pidMatch[1], 10) : 0;
    const processName = nameMatch ? nameMatch[1] : `PID ${pid}`;
    if (!portMap.has(port)) {
      portMap.set(port, { port, address: "127.0.0.1", pid, processName });
    }
  }
  return Array.from(portMap.values()).sort((a, b) => a.port - b.port);
}

// ── Entry point ────────────────────────────────────────────────────────────

export function discoverServices(): ServiceInfo[] {
  try {
    if (process.platform === "win32") return discoverWindows();
    return discoverUnix();
  } catch (e) {
    console.error("[discovery] Unexpected error:", e);
    return [];
  }
}
