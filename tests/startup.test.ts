import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

vi.mock("@bifurc/engine/gitops/gitStore", () => ({
  checkGitInstalled: vi.fn(),
}));
vi.mock("@bifurc/engine/applications/portUtils", () => ({
  checkPortInUse: vi.fn(),
}));

import { checkGitInstalled } from "@bifurc/engine/gitops/gitStore";
import { checkPortInUse } from "@bifurc/engine/applications/portUtils";
import { preflight } from "@bifurc/engine/startup";

describe("startup/preflight", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-startup-test-"));
    vi.mocked(checkGitInstalled).mockResolvedValue(true);
    vi.mocked(checkPortInUse).mockResolvedValue({ inUse: false });
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    vi.clearAllMocks();
  });

  it("reports all-ok when git present, dir writable, ports free, mkcert loaded", async () => {
    const checks = await preflight({ dataDir: tmpDir });
    expect(checks.length).toBeGreaterThan(0);
    expect(checks.every((c) => c.ok)).toBe(true);
  });

  it("reports git-missing when git is not installed", async () => {
    vi.mocked(checkGitInstalled).mockResolvedValue(false);
    const checks = await preflight({ dataDir: tmpDir });
    const c = checks.find((c) => c.code === "git-missing");
    expect(c).toBeDefined();
    expect(c!.ok).toBe(false);
    expect(c!.hint).toContain("git-scm.com");
  });

  it("reports data-dir-unwritable when the path cannot be created", async () => {
    // Parent is a plain file, so mkdirSync(..., {recursive:true}) for a child path must fail.
    const blockerFile = path.join(tmpDir, "blocker");
    fs.writeFileSync(blockerFile, "x");
    const badDir = path.join(blockerFile, "nested");

    const checks = await preflight({ dataDir: badDir });
    const c = checks.find((cc) => cc.code === "data-dir-unwritable");
    expect(c).toBeDefined();
    expect(c!.ok).toBe(false);
  });

  it("reports port-in-use only for the occupied port, leaving others ok", async () => {
    vi.mocked(checkPortInUse).mockImplementation(async (port: number) =>
      port === 8080 ? { inUse: true, pid: 1234 } : { inUse: false },
    );
    const checks = await preflight({
      dataDir: tmpDir,
      ports: [
        { name: "proxy", port: 8080 },
        { name: "webhook", port: 9101 },
      ],
    });
    const bad = checks.find((c) => c.code === "port-in-use");
    expect(bad).toBeDefined();
    expect(bad!.message).toContain("8080");
    expect(bad!.message).toContain("1234");
    expect(checks.filter((c) => c.code === "port-in-use")).toHaveLength(1);
  });

  it("never throws even when every check fails", async () => {
    vi.mocked(checkGitInstalled).mockResolvedValue(false);
    vi.mocked(checkPortInUse).mockResolvedValue({ inUse: true, pid: 1 });
    const badDir = path.join(tmpDir, "blocker2");
    fs.writeFileSync(badDir, "x"); // a file, not a dir — nested writes under it must fail
    await expect(
      preflight({ dataDir: path.join(badDir, "nested"), ports: [{ name: "x", port: 1 }] }),
    ).resolves.toBeDefined();
  });

  // ── P14 WI 5 — a busy BASE port is advisory, a busy OVERRIDE is fatal ────────────────────────
  describe("port advisories (P14 WI 5 / D16)", () => {
    it("marks a busy base port as advisory and names the increment in the hint", async () => {
      vi.mocked(checkPortInUse).mockResolvedValue({ inUse: true, pid: 4321 });
      const checks = await preflight({
        dataDir: tmpDir,
        ports: [{ name: "proxy", port: 8080, advisory: true, nextFree: 8081 }],
      });
      const c = checks.find((cc) => cc.code === "port-in-use");
      expect(c).toBeDefined();
      expect(c!.ok).toBe(false);
      expect(c!.advisory).toBe(true);
      expect(c!.message).toContain("8080");
      expect(c!.message).toContain("4321");
      expect(c!.hint).toContain("8081");
      expect(c!.hint).toMatch(/count upward|and up/);
    });

    it("still advises when the caller does not know nextFree, without inventing a number", async () => {
      vi.mocked(checkPortInUse).mockResolvedValue({ inUse: true });
      const checks = await preflight({
        dataDir: tmpDir,
        ports: [{ name: "proxy", port: 8080, advisory: true }],
      });
      const c = checks.find((cc) => cc.code === "port-in-use");
      expect(c!.advisory).toBe(true);
      expect(c!.hint).toContain("count upward");
      expect(c!.hint).not.toMatch(/\b808\d\b/);
    });

    it("keeps a busy per-workspace override fatal — no advisory flag, and a fix-this hint", async () => {
      vi.mocked(checkPortInUse).mockResolvedValue({ inUse: true, pid: 99 });
      const checks = await preflight({
        dataDir: tmpDir,
        ports: [{ name: "workspace override", port: 9443 }],
      });
      const c = checks.find((cc) => cc.code === "port-in-use");
      expect(c).toBeDefined();
      expect(c!.ok).toBe(false);
      // Absent, never `false` — `!c.ok` alone must stay the correct test for every pre-P14 caller.
      expect(c!.advisory).toBeUndefined();
      expect(c!.hint).toContain("change this workspace's configured port");
      expect(c!.hint).not.toContain("count upward");
    });

    it("flags only the busy port, and only that one is advisory", async () => {
      vi.mocked(checkPortInUse).mockImplementation(async (port: number) =>
        port === 8080 || port === 9443 ? { inUse: true, pid: 7 } : { inUse: false },
      );
      const checks = await preflight({
        dataDir: tmpDir,
        ports: [
          { name: "proxy-base", port: 8080, advisory: true, nextFree: 8081 },
          { name: "webhook", port: 9101 },
          { name: "workspace override", port: 9443 },
        ],
      });
      const busy = checks.filter((c) => c.code === "port-in-use");
      expect(busy).toHaveLength(2);
      expect(busy.filter((c) => c.advisory === true)).toHaveLength(1);
      expect(busy.find((c) => c.message!.includes("8080"))!.advisory).toBe(true);
      expect(busy.find((c) => c.message!.includes("9443"))!.advisory).toBeUndefined();
    });

    it("leaves a free advisory port ok, with no advisory flag set", async () => {
      const checks = await preflight({
        dataDir: tmpDir,
        ports: [{ name: "proxy", port: 8080, advisory: true, nextFree: 8080 }],
      });
      expect(checks.every((c) => c.ok)).toBe(true);
      expect(checks.some((c) => c.advisory === true)).toBe(false);
    });
  });
});
