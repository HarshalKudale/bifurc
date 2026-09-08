import simpleGit, { SimpleGitProgressEvent } from "simple-git";
import * as fs from "fs";
import * as path from "path";
import { getGit } from "@/store/gitStore";

export async function isRemoteEmpty(remote: string, branch: string): Promise<boolean> {
  try {
    const result = await simpleGit().raw(["ls-remote", "--refs", remote]);
    return result.trim() === "";
  } catch (e) {
    throw new Error(`Cannot access remote: ${e instanceof Error ? e.message : String(e)}`);
  }
}

export async function fetchRemoteHead(remote: string, branch: string): Promise<string | null> {
  try {
    const result = await simpleGit().raw(["ls-remote", remote, `refs/heads/${branch}`]);
    const sha = result.trim().split(/\s+/)[0];
    return sha || null;
  } catch {
    return null;
  }
}

export async function performGitClone(
  remote: string,
  branch: string,
  targetDir: string,
  onProgress: (stage: string, percent: number) => void,
): Promise<void> {
  const tempDir = targetDir + "_clone_tmp";
  if (fs.existsSync(tempDir)) fs.rmSync(tempDir, { recursive: true, force: true });

  await simpleGit({
    progress({ stage, progress }: SimpleGitProgressEvent) { onProgress(stage, progress); },
  }).clone(remote, tempDir, ["--branch", branch, "--single-branch", "--progress"]);

  const entries = fs.readdirSync(targetDir);
  for (const e of entries) fs.rmSync(path.join(targetDir, e), { recursive: true, force: true });
  for (const e of fs.readdirSync(tempDir)) fs.renameSync(path.join(tempDir, e), path.join(targetDir, e));
  fs.rmSync(tempDir, { recursive: true, force: true });

  const g = simpleGit(targetDir);
  await g.addConfig("user.email", "bifurc@local", false, "local");
  await g.addConfig("user.name", "Bifurc", false, "local");
}

export async function performGitPull(
  wsId: string,
  branch: string,
): Promise<{ updated: boolean; changedPaths?: string[] }> {
  const g = getGit(wsId);
  await g.fetch("origin", branch);

  let localHead: string;
  let remoteHead: string;
  try {
    localHead = (await g.revparse(["HEAD"])).trim();
    remoteHead = (await g.revparse([`origin/${branch}`])).trim();
  } catch {
    return { updated: false };
  }

  if (localHead === remoteHead) {
    return { updated: false };
  }

  let changedPaths: string[] = [];
  try {
    const raw = await g.raw(["diff", "--name-only", localHead, `origin/${branch}`]);
    changedPaths = raw.split("\n").map((l) => l.trim()).filter(Boolean);
  } catch { /* non-fatal */ }

  let stashed = false;
  try {
    const statusCheck = await g.status();
    const hasLocalChanges =
      statusCheck.modified.length > 0 ||
      statusCheck.not_added.length > 0 ||
      statusCheck.deleted.length > 0 ||
      statusCheck.created.length > 0;
    if (hasLocalChanges) {
      await g.raw(["stash", "push", "--include-untracked", "-m", "auto-stash before pull"]);
      stashed = true;
    }
  } catch { /* non-fatal */ }

  try {
    await g.merge([`origin/${branch}`, "--no-edit"]);
  } catch {
    try { await g.raw(["merge", "--abort"]); } catch {}
    if (stashed) {
      try { await g.stash(["pop"]); } catch {}
    }
    throw new Error("Pull merge failed");
  }

  if (stashed) {
    try {
      await g.stash(["pop"]);
    } catch {
      try {
        await g.raw(["checkout", "--ours", "."]);
        await g.raw(["stash", "drop"]);
      } catch {}
    }
  }

  return { updated: true, changedPaths };
}
