import { getGit, initWorkspaceRepo } from "@/store/gitStore";
import { wsDir } from "@/store/workspaceFs";
import { loadSettings, saveSettings } from "@/store/appSettings";
import { SyncConfig, SyncMeta, SyncState, SyncStatus } from "@/sync/types";
import simpleGit, { SimpleGitProgressEvent } from "simple-git";
import * as fs from "fs";
import * as path from "path";

/** Extract entity ID from a workspace-relative path like "requests/folder/abc123.json" */
function entityIdFromPath(p: string): string | null {
  const base = p.split("/").pop();
  if (!base || !base.endsWith(".json")) return null;
  const id = base.slice(0, -5);
  // Skip index/enabled/names files
  if (id === "index" || id === "enabled" || id === "names") return null;
  return id;
}

import {
  getState,
  setState,
  emit,
  getSyncConfig,
  getSyncMeta,
  saveSyncConfig,
  saveSyncMeta,
  onSyncStatusChange,
  getSyncState,
  clearState
} from "./syncState";

export { onSyncStatusChange, getSyncState, getSyncConfig };

function isWorkspaceEmpty(wsId: string): boolean {
  const dir = wsDir(wsId);
  const kinds = ["mappings", "rules", "environments", "mocks", "requests", "sockets"];
  for (const kind of kinds) {
    const kindDir = path.join(dir, kind);
    if (!fs.existsSync(kindDir)) continue;
    const files = fs.readdirSync(kindDir, { withFileTypes: true });
    for (const f of files) {
      if (f.isFile() && f.name.endsWith(".json") && f.name !== "index.json" && f.name !== "enabled.json") return false;
      if (f.isDirectory() && f.name !== "drafts" && f.name !== "capture") {
        const subFiles = fs.readdirSync(path.join(kindDir, f.name));
        if (subFiles.some((sf) => sf.endsWith(".json") && sf !== "index.json" && sf !== "enabled.json")) return false;
      }
    }
  }
  return true;
}

import { isRemoteEmpty, fetchRemoteHead, performGitClone, performGitPull } from "./gitSyncOps";

export async function setRemote(
  wsId: string,
  remote: string,
  branch: string,
): Promise<{ ok: boolean; cloned?: boolean; adoptedId?: string; error?: string }> {
  const dir = wsDir(wsId);
  setState(wsId, { status: "cloning", error: null, progressMessage: "Connecting…" });
  emit(wsId);

  function emitProgress(stage: string, percent: number) {
    const label = stage === "receiving" ? "Receiving" : stage === "resolving" ? "Resolving" : stage === "writing" ? "Writing" : stage === "counting" ? "Counting" : "Working";
    setState(wsId, { progressMessage: `${label}… ${percent}%` });
    emit(wsId);
  }

  try {
    const empty = isWorkspaceEmpty(wsId);

    if (empty) {
      const remoteEmpty = await isRemoteEmpty(remote, branch);

      if (remoteEmpty) {
        // Both sides empty — just wire up and push the initial commit
        setState(wsId, { progressMessage: "Pushing initial commit…" });
        emit(wsId);
        await initWorkspaceRepo(wsId);
        const g = getGit(wsId);
        try { await g.removeRemote("origin"); } catch {}
        await g.addRemote("origin", remote);
        await g.push("origin", `HEAD:${branch}`, ["--set-upstream"]);
        saveSyncConfig(wsId, { remote, branch, autoSync: false });
        saveSyncMeta(wsId, { lastPushedAt: Date.now(), lastPulledAt: null, lastSyncedCommit: null });
        setState(wsId, { status: "idle", error: null, progressMessage: null });
        emit(wsId);
        return { ok: true, cloned: false };
      }

      // Remote has content — clone it into the workspace dir
      setState(wsId, { progressMessage: "Cloning remote…" });
      emit(wsId);
      const tempDir = dir + "_clone_tmp";
      if (fs.existsSync(tempDir)) fs.rmSync(tempDir, { recursive: true, force: true });

      await simpleGit({
        progress({ stage, progress }: SimpleGitProgressEvent) { emitProgress(stage, progress); },
      }).clone(remote, tempDir, ["--branch", branch, "--single-branch", "--progress"]);

      setState(wsId, { progressMessage: "Unpacking files…" });
      emit(wsId);

      // Swap cloned content into workspace dir
      const entries = fs.readdirSync(dir);
      for (const e of entries) fs.rmSync(path.join(dir, e), { recursive: true, force: true });
      for (const e of fs.readdirSync(tempDir)) fs.renameSync(path.join(tempDir, e), path.join(dir, e));
      fs.rmSync(tempDir, { recursive: true, force: true });

      const g = simpleGit(dir);
      await g.addConfig("user.email", "bifurc@local", false, "local");
      await g.addConfig("user.name", "Bifurc", false, "local");

      // Read the remote workspace's identity from workspace.json
      let adoptedId = wsId;
      let adoptedName: string | null = null;
      try {
        const wsJson = JSON.parse(fs.readFileSync(path.join(dir, "workspace.json"), "utf-8"));
        if (wsJson.id && wsJson.id !== wsId) {
          // Remote workspace has a different ID — rename the dir and update app.json
          const newDir = wsDir(wsJson.id);
          if (!fs.existsSync(newDir)) fs.renameSync(dir, newDir);
          adoptedId = wsJson.id;
          adoptedName = wsJson.name ?? null;
        }
      } catch { /* workspace.json missing or malformed — keep local id */ }

      // Update app.json: replace local workspace entry with remote identity
      const settings = loadSettings();
      const ws = settings.workspaces.find((w) => w.id === wsId);
      if (ws && adoptedId !== wsId) {
        ws.id = adoptedId;
        if (adoptedName) ws.name = adoptedName;
        if (settings.activeWorkspaceId === wsId) settings.activeWorkspaceId = adoptedId;
      }
      saveSettings(settings);

      // Persist sync config + meta under the adopted ID
      saveSyncConfig(adoptedId, { remote, branch, autoSync: false });
      saveSyncMeta(adoptedId, { lastPushedAt: null, lastPulledAt: Date.now(), lastSyncedCommit: null });
      // Clean up stale state entry for old id if it changed
      if (adoptedId !== wsId) clearState(wsId);
      setState(adoptedId, { status: "idle", error: null, progressMessage: null });
      emit(adoptedId);
      return { ok: true, cloned: true, adoptedId };

    } else {
      // Non-empty workspace — remote must be empty to push
      const remoteEmpty = await isRemoteEmpty(remote, branch);
      if (!remoteEmpty) {
        setState(wsId, { status: "idle", error: "Remote is not empty", progressMessage: null });
        emit(wsId);
        return { ok: false, error: "Remote is not empty. Connect an empty workspace to clone from a non-empty remote, or use an empty remote for pushing local data." };
      }

      setState(wsId, { progressMessage: "Pushing to remote…" });
      emit(wsId);
      await initWorkspaceRepo(wsId);

      const gWithProgress = simpleGit(dir, {
        progress({ stage, progress }: SimpleGitProgressEvent) { emitProgress(stage, progress); },
      });
      try { await gWithProgress.removeRemote("origin"); } catch {}
      await gWithProgress.addRemote("origin", remote);
      await gWithProgress.push("origin", `HEAD:${branch}`, ["--set-upstream", "--progress"]);

      saveSyncConfig(wsId, { remote, branch, autoSync: false });
      saveSyncMeta(wsId, { lastPushedAt: Date.now(), lastPulledAt: null, lastSyncedCommit: null });
      setState(wsId, { status: "idle", error: null, progressMessage: null });
      emit(wsId);
      return { ok: true, cloned: false };
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setState(wsId, { status: "error", error: msg, progressMessage: null });
    emit(wsId);
    return { ok: false, error: msg };
  }
}

export async function disconnect(wsId: string): Promise<{ ok: boolean }> {
  try {
    const g = getGit(wsId);
    try { await g.removeRemote("origin"); } catch {}
    saveSyncConfig(wsId, null);
    saveSyncMeta(wsId, null);
    setState(wsId, { status: "idle", error: null, lastPushedAt: null, lastPulledAt: null });
    emit(wsId);
    return { ok: true };
  } catch {
    return { ok: true };
  }
}

export async function syncPush(wsId: string): Promise<{ ok: boolean; error?: string }> {
  const config = getSyncConfig(wsId);
  if (!config) return { ok: false, error: "No remote configured" };

  setState(wsId, { status: "pushing", error: null });
  emit(wsId);

  try {
    const g = getGit(wsId);
    await g.push("origin", `HEAD:${config.branch}`);
    const now = Date.now();
    const meta = getSyncMeta(wsId);
    saveSyncMeta(wsId, { lastPushedAt: now, lastPulledAt: meta?.lastPulledAt ?? null, lastSyncedCommit: null });
    setState(wsId, { status: "idle", error: null, lastPushedAt: now });
    emit(wsId);
    return { ok: true };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setState(wsId, { status: "error", error: msg });
    emit(wsId);
    return { ok: false, error: msg };
  }
}

export async function syncPull(wsId: string): Promise<{ ok: boolean; updated?: boolean; updatedIds?: string[]; error?: string }> {
  const config = getSyncConfig(wsId);
  if (!config) return { ok: false, error: "No remote configured" };

  setState(wsId, { status: "pulling", error: null, updatedIds: undefined });
  emit(wsId);

  try {
    const { updated, changedPaths } = await performGitPull(wsId, config.branch);

    const now = Date.now();
    const meta = getSyncMeta(wsId);
    saveSyncMeta(wsId, { lastPushedAt: meta?.lastPushedAt ?? null, lastPulledAt: now, lastSyncedCommit: meta?.lastSyncedCommit ?? null });

    if (!updated) {
      setState(wsId, { status: "idle", error: null, lastPulledAt: now });
      emit(wsId);
      return { ok: true, updated: false };
    }

    const updatedIds = (changedPaths || [])
      .map(entityIdFromPath)
      .filter((id): id is string => id !== null);

    setState(wsId, { status: "idle", error: null, lastPulledAt: now, updatedIds });
    emit(wsId);
    return { ok: true, updated: true, updatedIds };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    setState(wsId, { status: "error", error: msg });
    emit(wsId);
    return { ok: false, error: msg };
  }
}

export async function setAutoSync(wsId: string, enabled: boolean): Promise<{ ok: boolean }> {
  const config = getSyncConfig(wsId);
  if (!config) return { ok: false };
  saveSyncConfig(wsId, { ...config, autoSync: enabled });
  return { ok: true };
}

/**
 * Fetch only the remote branch tip SHA — lightweight (~200 bytes over HTTP).
 * Used by the polling loop to detect new remote commits without a full fetch.
 */
export async function getRemoteHead(wsId: string): Promise<string | null> {
  const config = getSyncConfig(wsId);
  if (!config) return null;
  try {
    const result = await simpleGit().raw(["ls-remote", config.remote, `refs/heads/${config.branch}`]);
    const sha = result.trim().split(/\s+/)[0];
    return sha || null;
  } catch {
    return null;
  }
}
