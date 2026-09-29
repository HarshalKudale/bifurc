/**
 * The guard on removing a stale `.git/index.lock`.
 *
 * `withIndexLockRetry` removes a lock after five failed attempts, because a live git process does not
 * hold the index for two seconds on a repo of this size and the alternative is a Publish button that
 * can never work again. That makes the **path check** the part that matters: the path comes from git's
 * own stderr, and a message we did not anticipate must not be able to name a file we then delete.
 *
 * Two properties, both asserted here:
 *
 * 1. a path that is not a `index.lock` is never touched, however git's message is worded;
 * 2. a path outside this workspace's `.git/` is never touched.
 *
 * No repository is needed — `removeStaleIndexLock` only reads the path out of a string and compares it
 * against `wsDir(wsId)`, which the data-root override supplies.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

const WS_ID = "ws-lock-guard";
let tmpDir: string;

beforeEach(async () => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "lp-lock-guard-"));
  const { setDataDirOverride } = await import("@bifurc/engine/gitops/gitStore");
  setDataDirOverride(tmpDir);
  const { initWorkspaceDir } = await import("@bifurc/engine/workspace/workspaceFs");
  initWorkspaceDir(WS_ID, "Lock Guard");
  fs.mkdirSync(path.join(tmpDir, WS_ID, ".git"), { recursive: true });
});

afterEach(async () => {
  const { setDataDirOverride } = await import("@bifurc/engine/gitops/gitStore");
  setDataDirOverride("");
  try { fs.rmSync(tmpDir, { recursive: true, force: true }); } catch { /* safe-delete shim */ }
});

const gitMessage = (p: string) => `fatal: Unable to create '${p}': File exists.`;

describe("removeStaleIndexLock — what it will and will not delete", () => {
  it("removes the workspace's own index lock", async () => {
    const { removeStaleIndexLock } = await import("@bifurc/engine/gitops/gitLock");
    const lock = path.join(tmpDir, WS_ID, ".git", "index.lock");
    fs.writeFileSync(lock, "");

    expect(removeStaleIndexLock(WS_ID, gitMessage(lock))).toBe(true);
    expect(fs.existsSync(lock)).toBe(false);
  });

  it("leaves a file that is not called index.lock, however git's message reads", async () => {
    const { removeStaleIndexLock } = await import("@bifurc/engine/gitops/gitLock");
    // A plausible-looking message naming something else entirely. `git add` can report other locks
    // (a ref lock, a packed-refs lock) with the same "Unable to create" wording.
    const victim = path.join(tmpDir, WS_ID, ".git", "HEAD.lock");
    fs.writeFileSync(victim, "keep me");

    expect(removeStaleIndexLock(WS_ID, gitMessage(victim))).toBe(false);
    expect(fs.existsSync(victim)).toBe(true);
  });

  it("leaves an index.lock that belongs to another repository", async () => {
    const { removeStaleIndexLock } = await import("@bifurc/engine/gitops/gitLock");
    // The guard is the prefix check, not the basename: `index.lock` is a common name and the message
    // could come from a repo we do not own.
    const other = path.join(tmpDir, "another-workspace", ".git");
    fs.mkdirSync(other, { recursive: true });
    const victim = path.join(other, "index.lock");
    fs.writeFileSync(victim, "");

    expect(removeStaleIndexLock(WS_ID, gitMessage(victim))).toBe(false);
    expect(fs.existsSync(victim)).toBe(true);
  });

  it("does nothing when the message names no path at all", async () => {
    const { removeStaleIndexLock } = await import("@bifurc/engine/gitops/gitLock");
    expect(removeStaleIndexLock(WS_ID, "fatal: Unable to create '': File exists.")).toBe(false);
    expect(removeStaleIndexLock(WS_ID, "something else went wrong")).toBe(false);
  });
});
