import { SyncConfig, SyncMeta, SyncState } from "@/sync/types";
import { loadSettings, saveSettings } from "@/store/appSettings";

const _syncStates = new Map<string, SyncState>();

export function getState(wsId: string): SyncState {
  if (!_syncStates.has(wsId)) {
    _syncStates.set(wsId, { status: "idle", error: null, lastPushedAt: null, lastPulledAt: null, progressMessage: null });
  }
  return _syncStates.get(wsId)!;
}

export function setState(wsId: string, patch: Partial<SyncState>): SyncState {
  const state = { ...getState(wsId), ...patch };
  _syncStates.set(wsId, state);
  return state;
}

export function clearState(wsId: string): void {
  _syncStates.delete(wsId);
}

let _statusListener: ((wsId: string, state: SyncState) => void) | null = null;

export function onSyncStatusChange(cb: (wsId: string, state: SyncState) => void): void {
  _statusListener = cb;
}

export function emit(wsId: string): void {
  if (_statusListener) _statusListener(wsId, getState(wsId));
}

export function getSyncState(wsId: string): SyncState {
  const meta = getSyncMeta(wsId);
  const state = getState(wsId);
  return { ...state, lastPushedAt: meta?.lastPushedAt ?? null, lastPulledAt: meta?.lastPulledAt ?? null };
}

export function getSyncConfig(wsId: string): SyncConfig | null {
  const settings = loadSettings();
  const ws = settings.workspaces.find((w) => w.id === wsId);
  return (ws as any)?.syncConfig ?? null;
}

export function getSyncMeta(wsId: string): SyncMeta | null {
  const settings = loadSettings();
  const ws = settings.workspaces.find((w) => w.id === wsId);
  return (ws as any)?.syncMeta ?? null;
}

export function saveSyncConfig(wsId: string, config: SyncConfig | null): void {
  const settings = loadSettings();
  const ws = settings.workspaces.find((w) => w.id === wsId);
  if (ws) (ws as any).syncConfig = config;
  saveSettings(settings);
}

export function saveSyncMeta(wsId: string, meta: SyncMeta | null): void {
  const settings = loadSettings();
  const ws = settings.workspaces.find((w) => w.id === wsId);
  if (ws) (ws as any).syncMeta = meta;
  saveSettings(settings);
}
