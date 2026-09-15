import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

vi.mock("@bifurc/engine/store/gitStore", () => ({
  checkGitInstalled: vi.fn(),
}));
vi.mock("@/applications/portUtils", () => ({
  checkPortInUse: vi.fn(),
}));

import { checkGitInstalled } from "@bifurc/engine/store/gitStore";
import { checkPortInUse } from "@/applications/portUtils";
import { preflight } from "@/startup";

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
});
