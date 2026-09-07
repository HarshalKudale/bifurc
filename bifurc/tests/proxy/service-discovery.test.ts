import { describe, it, expect, vi, beforeEach } from "vitest";
import { execSync } from "child_process";

vi.mock("child_process", () => ({
  execSync: vi.fn(),
}));

import { discoverServices } from "@/proxy/service-discovery";

// ── Helpers ──────────────────────────────────────────────────────────────────

function psConnectionsJson(
  connections: Array<{ LocalAddress: string; LocalPort: number; OwningProcess: number }>,
): string {
  return JSON.stringify(connections);
}

function psProcessesJson(procs: Array<{ Id: number; Name: string }>): string {
  return JSON.stringify(procs);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("src/proxy/service-discovery.ts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("discoverServices()", () => {
    it("returns an empty array when Get-NetTCPConnection throws", () => {
      vi.mocked(execSync).mockImplementation(() => {
        throw new Error("PS failed");
      });

      const result = discoverServices();

      expect(result).toEqual([]);
    });

    it("returns an empty array when no localhost/all-interfaces listeners exist", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          // First call: Get-NetTCPConnection — only remote address listeners
          psConnectionsJson([
            { LocalAddress: "192.168.1.1", LocalPort: 8080, OwningProcess: 100 },
          ]) as any,
        );

      const result = discoverServices();

      expect(result).toEqual([]);
    });

    it("returns services for 0.0.0.0 listeners", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([{ LocalAddress: "0.0.0.0", LocalPort: 3000, OwningProcess: 200 }]) as any,
        )
        .mockReturnValueOnce(
          psProcessesJson([{ Id: 200, Name: "node" }]) as any,
        );

      const result = discoverServices();

      expect(result).toHaveLength(1);
      expect(result[0].port).toBe(3000);
      expect(result[0].processName).toBe("node");
    });

    it("returns services for 127.0.0.1 listeners", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([{ LocalAddress: "127.0.0.1", LocalPort: 4000, OwningProcess: 300 }]) as any,
        )
        .mockReturnValueOnce(
          psProcessesJson([{ Id: 300, Name: "python" }]) as any,
        );

      const result = discoverServices();

      expect(result).toHaveLength(1);
      expect(result[0].address).toBe("127.0.0.1");
      expect(result[0].pid).toBe(300);
      expect(result[0].processName).toBe("python");
    });

    it("returns services for :: (all IPv6 interfaces) listeners", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([{ LocalAddress: "::", LocalPort: 5000, OwningProcess: 400 }]) as any,
        )
        .mockReturnValueOnce(
          psProcessesJson([{ Id: 400, Name: "java" }]) as any,
        );

      const result = discoverServices();

      expect(result).toHaveLength(1);
      expect(result[0].port).toBe(5000);
    });

    it("returns services for ::1 (localhost IPv6) listeners", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([{ LocalAddress: "::1", LocalPort: 6000, OwningProcess: 500 }]) as any,
        )
        .mockReturnValueOnce(
          psProcessesJson([{ Id: 500, Name: "ruby" }]) as any,
        );

      const result = discoverServices();

      expect(result).toHaveLength(1);
      expect(result[0].port).toBe(6000);
    });

    it("deduplicates listeners on the same port", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([
            { LocalAddress: "0.0.0.0", LocalPort: 8080, OwningProcess: 10 },
            { LocalAddress: "127.0.0.1", LocalPort: 8080, OwningProcess: 10 },
          ]) as any,
        )
        .mockReturnValueOnce(psProcessesJson([{ Id: 10, Name: "nginx" }]) as any);

      const result = discoverServices();

      expect(result).toHaveLength(1);
      expect(result[0].port).toBe(8080);
    });

    it("sorts results by port number ascending", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([
            { LocalAddress: "0.0.0.0", LocalPort: 9000, OwningProcess: 10 },
            { LocalAddress: "0.0.0.0", LocalPort: 3000, OwningProcess: 20 },
            { LocalAddress: "0.0.0.0", LocalPort: 6000, OwningProcess: 30 },
          ]) as any,
        )
        .mockReturnValueOnce(
          psProcessesJson([
            { Id: 10, Name: "svc-a" },
            { Id: 20, Name: "svc-b" },
            { Id: 30, Name: "svc-c" },
          ]) as any,
        );

      const result = discoverServices();

      expect(result.map((s) => s.port)).toEqual([3000, 6000, 9000]);
    });

    it("handles a single connection result (PS returns object, not array)", () => {
      // PowerShell returns a plain object (not array) when there is only one result
      const single = { LocalAddress: "0.0.0.0", LocalPort: 7777, OwningProcess: 50 };
      vi.mocked(execSync)
        .mockReturnValueOnce(JSON.stringify(single) as any)
        .mockReturnValueOnce(psProcessesJson([{ Id: 50, Name: "nginx" }]) as any);

      const result = discoverServices();

      expect(result).toHaveLength(1);
      expect(result[0].port).toBe(7777);
    });

    it("falls back to 'PID <pid>' when process name resolution fails", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([{ LocalAddress: "0.0.0.0", LocalPort: 8888, OwningProcess: 999 }]) as any,
        )
        .mockImplementationOnce(() => {
          throw new Error("Get-Process failed");
        });

      const result = discoverServices();

      expect(result).toHaveLength(1);
      expect(result[0].processName).toBe("PID 999");
    });

    it("handles multiple services and resolves all their process names", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([
            { LocalAddress: "0.0.0.0", LocalPort: 1234, OwningProcess: 11 },
            { LocalAddress: "127.0.0.1", LocalPort: 5678, OwningProcess: 22 },
          ]) as any,
        )
        .mockReturnValueOnce(
          psProcessesJson([
            { Id: 11, Name: "proc-a" },
            { Id: 22, Name: "proc-b" },
          ]) as any,
        );

      const result = discoverServices();

      expect(result).toHaveLength(2);
      const portMap = new Map(result.map((s) => [s.port, s]));
      expect(portMap.get(1234)?.processName).toBe("proc-a");
      expect(portMap.get(5678)?.processName).toBe("proc-b");
    });

    it("handles a single process result (PS returns object, not array) for process names", () => {
      vi.mocked(execSync)
        .mockReturnValueOnce(
          psConnectionsJson([{ LocalAddress: "0.0.0.0", LocalPort: 2222, OwningProcess: 77 }]) as any,
        )
        .mockReturnValueOnce(JSON.stringify({ Id: 77, Name: "single-proc" }) as any);

      const result = discoverServices();

      expect(result).toHaveLength(1);
      expect(result[0].processName).toBe("single-proc");
    });
  });
});
