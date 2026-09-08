import { getSyncConfig, syncPull, getRemoteHead } from "@/sync/syncManager";

// Lazy import to avoid circular dependency — server.ts imports syncManager indirectly
let _reloadConfig: (() => void) | null = null;
export function setAutoSyncReloadFn(fn: () => void): void { _reloadConfig = fn; }

interface PollerState {
  timer: ReturnType<typeof setInterval>;
  lastKnownRemoteHead: string | null;
}

const _pollers = new Map<string, PollerState>();

// Poll interval: lightweight ls-remote check every 30 seconds
const POLL_INTERVAL_MS = 30_000;

export function startAutoSync(wsId: string): void {
  stopAutoSync(wsId);
  const config = getSyncConfig(wsId);
  if (!config?.autoSync) return;

  const state: PollerState = { timer: null as any, lastKnownRemoteHead: null };

  // Seed the last-known head so the first tick doesn't immediately pull
  getRemoteHead(wsId).then((sha) => { state.lastKnownRemoteHead = sha; }).catch(() => {});

  state.timer = setInterval(async () => {
    const cfg = getSyncConfig(wsId);
    if (!cfg?.autoSync) { stopAutoSync(wsId); return; }

    try {
      const remoteHead = await getRemoteHead(wsId);
      if (!remoteHead) return;

      // Only pull when the remote tip has advanced
      if (remoteHead === state.lastKnownRemoteHead) return;

      state.lastKnownRemoteHead = remoteHead;
      const result = await syncPull(wsId);
      if (result.ok && result.updated && _reloadConfig) _reloadConfig();
    } catch {}
  }, POLL_INTERVAL_MS);

  _pollers.set(wsId, state);
}

export function stopAutoSync(wsId: string): void {
  const poller = _pollers.get(wsId);
  if (poller) {
    clearInterval(poller.timer);
    _pollers.delete(wsId);
  }
}

export function stopAllAutoSync(): void {
  for (const [wsId] of _pollers) stopAutoSync(wsId);
}

export function updateLastKnownHead(wsId: string, sha: string): void {
  const poller = _pollers.get(wsId);
  if (poller) poller.lastKnownRemoteHead = sha;
}
