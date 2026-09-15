import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import {
  setDataRoot,
  resetDataRootForTests,
  dataDir,
  isDataRootSet,
  DataRootNotInitialisedError,
  platformDefaultDataDir,
  resolveDataDir,
} from "@bifurc/engine/store/paths";

describe("store/paths", () => {
  afterEach(() => {
    resetDataRootForTests();
    vi.unstubAllEnvs();
  });

  describe("setDataRoot / dataDir", () => {
    it("throws DataRootNotInitialisedError before setDataRoot is called", () => {
      expect(isDataRootSet()).toBe(false);
      expect(() => dataDir()).toThrow(DataRootNotInitialisedError);
    });

    it("returns the resolved absolute path after setDataRoot", () => {
      const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-paths-test-"));
      setDataRoot(dir);
      expect(isDataRootSet()).toBe(true);
      expect(dataDir()).toBe(path.resolve(dir));
      fs.rmSync(dir, { recursive: true, force: true });
    });

    it("creates the directory (and subdirectories) if missing", () => {
      const base = fs.mkdtempSync(path.join(os.tmpdir(), "bifurc-paths-test-"));
      const nested = path.join(base, "a", "b", "c");
      expect(fs.existsSync(nested)).toBe(false);
      setDataRoot(nested);
      expect(fs.existsSync(nested)).toBe(true);
      fs.rmSync(base, { recursive: true, force: true });
    });
  });

  describe("platformDefaultDataDir", () => {
    it("returns a path ending in Bifurc/bifurc for the current platform", () => {
      const dir = platformDefaultDataDir();
      expect(dir.toLowerCase()).toContain("bifurc");
    });
  });

  describe("resolveDataDir", () => {
    it("prefers --data-dir <path> over everything else", () => {
      vi.stubEnv("BIFURC_DATA_DIR", "/from/env");
      const resolved = resolveDataDir(["node", "script.js", "--data-dir", "/from/flag"]);
      expect(resolved).toBe("/from/flag");
    });

    it("accepts --data-dir=<path> form", () => {
      const resolved = resolveDataDir(["node", "script.js", "--data-dir=/from/flag-eq"]);
      expect(resolved).toBe("/from/flag-eq");
    });

    it("falls back to BIFURC_DATA_DIR when no flag is present", () => {
      vi.stubEnv("BIFURC_DATA_DIR", "/from/env");
      const resolved = resolveDataDir(["node", "script.js"]);
      expect(resolved).toBe("/from/env");
    });

    it("falls back to the platform default when neither flag nor env var is present", () => {
      vi.stubEnv("BIFURC_DATA_DIR", "");
      delete process.env.BIFURC_DATA_DIR;
      const resolved = resolveDataDir(["node", "script.js"]);
      expect(resolved).toBe(platformDefaultDataDir());
    });
  });
});
